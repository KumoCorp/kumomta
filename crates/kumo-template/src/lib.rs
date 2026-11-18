use handlebars::{Handlebars, Renderable, Template as HandlebarsTemplate};
use lruttl::declare_cache;
use minijinja::value::{Enumerator, Object, ValueKind};
use minijinja::{Environment, Template as JinjaTemplate, Value as JinjaValue};
use minijinja_contrib::add_to_environment;
use self_cell::self_cell;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt::Write;
use std::sync::{Arc, LazyLock, OnceLock};
use std::time::Duration;
use thiserror::Error;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TemplateDialect {
    #[default]
    Jinja,
    Static,
    Handlebars,
}

/// Builds the minijinja environment that every Jinja template renders with.
fn new_jinja_environment() -> Environment<'static> {
    let mut env = Environment::new();
    env.set_unknown_method_callback(minijinja_contrib::pycompat::unknown_method_callback);
    add_to_environment(&mut env);

    env.add_filter(
        "normalize_smtp_response",
        mod_smtp_response_normalize::normalize,
    );

    env
}

/// The registry every Handlebars [`Template`] renders against. A handlebars
/// template compiles independently of any registry and consults a registry at
/// render time only to look up helpers. One shared instance serves every
/// template.
static HANDLEBARS_REGISTRY: LazyLock<Handlebars<'static>> = LazyLock::new(|| {
    let mut registry = Handlebars::new();
    registry.set_recursive_lookup(true);
    registry
});

/// Converts a minijinja compile error into an error that names the template
/// and quotes the offending line of `source`.
fn describe_jinja_error(err: minijinja::Error, source: &str) -> anyhow::Error {
    let mut reason = String::new();

    if let Some(detail) = err.detail() {
        write!(&mut reason, "{}: {}", err.kind(), detail).ok();
    } else {
        write!(&mut reason, "{}", err.kind()).ok();
    }

    if let Some((line_no, source_line)) = err.line().and_then(|line| {
        source
            .lines()
            .nth(line - 1) // err.line() is 1-based
            .map(|source_line| (line, source_line))
    }) {
        let truncated_line = &source_line[..source_line.ceil_char_boundary(1024)];

        if let Some(name) = err.name() {
            write!(
                &mut reason,
                " (in template '{name}' line {line_no}: '{truncated_line}')"
            )
            .ok();
        } else {
            write!(
                &mut reason,
                " (in template line {line_no}: '{truncated_line}')"
            )
            .ok();
        }
    }

    anyhow::anyhow!("{reason}")
}

/// Returns true when `source` contains none of the expansion markers of
/// `dialect`, meaning the engine would render it with no substitution.
fn source_is_static(dialect: TemplateDialect, source: &str) -> bool {
    match dialect {
        TemplateDialect::Static => true,
        TemplateDialect::Handlebars => !source.contains("{{"),
        TemplateDialect::Jinja => {
            !(source.contains("{{") || source.contains("{%") || source.contains("{#"))
        }
    }
}

/// Returns the prefix of `source` that `dialect` renders for a static value:
/// for Jinja, `source` with one trailing `\n` and then one trailing `\r`
/// removed, matching the trim the minijinja lexer applies to every template it
/// compiles. For the other dialects, `source` unchanged.
fn trim_static_source(dialect: TemplateDialect, source: &str) -> &str {
    if dialect == TemplateDialect::Jinja {
        let s = source.strip_suffix('\n').unwrap_or(source);
        s.strip_suffix('\r').unwrap_or(s)
    } else {
        source
    }
}

/// Renders `template` against `context` with the escaping rule the registry
/// expects: substitutions are html-escaped only for a `.html` template name.
fn render_handlebars(
    registry: &Handlebars<'static>,
    template: &HandlebarsTemplate,
    context: &handlebars::Context,
) -> anyhow::Result<String> {
    let mut render_context = handlebars::RenderContext::new(None);
    render_context.set_recursive_lookup(true);
    let is_html = template
        .name
        .as_deref()
        .map(|name| name.ends_with(".html"))
        .unwrap_or(false);
    render_context.set_disable_escape(!is_html);

    Ok(template.renders(registry, context, &mut render_context)?)
}

/// A rendering context converted once into the native form of a dialect.
pub enum PreparedContext {
    Jinja(JinjaValue),
    Static,
    Handlebars(handlebars::Context),
}

impl PreparedContext {
    fn new<CTX>(dialect: TemplateDialect, context: CTX) -> anyhow::Result<Self>
    where
        CTX: Serialize,
    {
        Ok(match dialect {
            TemplateDialect::Jinja => Self::Jinja(JinjaValue::from_serialize(context)),
            TemplateDialect::Static => Self::Static,
            TemplateDialect::Handlebars => {
                Self::Handlebars(handlebars::Context::from(serde_json::to_value(context)?))
            }
        })
    }
}

/// A Jinja render context that converts its value on the first lookup, at
/// most once, and also defines a global value named `alias` that returns
/// that value.
struct LazyContext<CTX> {
    inner: Arc<CTX>,
    alias: String,
    converted: OnceLock<JinjaValue>,
}

impl<CTX: Serialize> LazyContext<CTX> {
    fn value(&self) -> &JinjaValue {
        self.converted
            .get_or_init(|| JinjaValue::from_serialize(&*self.inner))
    }
}

impl<CTX> std::fmt::Debug for LazyContext<CTX> {
    fn fmt(&self, fmt: &mut std::fmt::Formatter) -> std::fmt::Result {
        fmt.debug_struct("LazyContext")
            .field("alias", &self.alias)
            .field("converted", &self.converted.get().is_some())
            .finish()
    }
}

impl<CTX: Serialize + Send + Sync + 'static> Object for LazyContext<CTX> {
    fn get_value(self: &Arc<Self>, key: &JinjaValue) -> Option<JinjaValue> {
        let key = key.as_str()?;
        let value = self.value();
        if value.kind() == ValueKind::Invalid {
            // Return the `Invalid` value to ensure that the error is
            // not rendered as an empty value.
            return Some(value.clone());
        }
        if key == self.alias {
            // The alias is explicitly defined as returning the entire value,
            // taking precedence over a hypothetical conflicting field with
            // the same name.
            return Some(value.clone());
        }
        // A missing name must yield None rather than undefined, otherwise the
        // engine would stop short of its own globals such as `range`.
        value
            .get_attr(key)
            .ok()
            .filter(|value| !value.is_undefined())
    }

    fn enumerate(self: &Arc<Self>) -> Enumerator {
        match self.value().try_iter() {
            Ok(keys) => Enumerator::Values(
                keys.filter(|key| key.as_str() != Some(self.alias.as_str()))
                    .chain(std::iter::once(JinjaValue::from(self.alias.as_str())))
                    .collect(),
            ),
            Err(_) => Enumerator::NonEnumerable,
        }
    }
}

type JinjaTemplateRef<'a> = JinjaTemplate<'a, 'a>;

self_cell!(
    /// A compiled Jinja template kept together with the environment it
    /// borrows from.
    struct OwnedJinja {
        owner: Environment<'static>,
        #[covariant]
        dependent: JinjaTemplateRef,
    }
);

enum Compiled {
    Static(String),
    Jinja(OwnedJinja),
    Handlebars(HandlebarsTemplate),
}

struct TemplateInner {
    dialect: TemplateDialect,
    name: String,
    compiled: Compiled,
}

/// A compiled template. It owns everything it needs to render, is `Send` and
/// `Sync`, and is cheap to clone: every clone shares the same compiled state
/// and can be cached or rendered concurrently from any task.
#[derive(Clone)]
pub struct Template(Arc<TemplateInner>);

impl std::fmt::Debug for Template {
    fn fmt(&self, fmt: &mut std::fmt::Formatter) -> std::fmt::Result {
        fmt.debug_struct("Template")
            .field("dialect", &self.0.dialect)
            .field("name", &self.0.name)
            .field("is_static", &self.is_static())
            .finish()
    }
}

impl Template {
    /// Compiles `source` as a template of `dialect` named `name`. A name
    /// ending in `.html` enables html entity escaping of substitutions.
    /// If `source` is free of expansion markers, a Static Template is
    /// returned.
    fn compile(dialect: TemplateDialect, name: &str, source: &str) -> anyhow::Result<Self> {
        let compiled = if source_is_static(dialect, source) {
            Compiled::Static(trim_static_source(dialect, source).to_string())
        } else {
            match dialect {
                TemplateDialect::Static => Compiled::Static(source.to_string()),
                TemplateDialect::Jinja => {
                    let mut env = new_jinja_environment();
                    env.add_template_owned(name.to_string(), source.to_string())
                        .map_err(|err| describe_jinja_error(err, source))?;
                    Compiled::Jinja(OwnedJinja::try_new(env, |env| env.get_template(name))?)
                }
                TemplateDialect::Handlebars => Compiled::Handlebars(
                    HandlebarsTemplate::compile_with_name(source, name.to_string())?,
                ),
            }
        };

        Ok(Self(Arc::new(TemplateInner {
            dialect,
            name: name.to_string(),
            compiled,
        })))
    }

    pub fn dialect(&self) -> TemplateDialect {
        self.0.dialect
    }

    pub fn name(&self) -> &str {
        &self.0.name
    }

    /// Returns true when this template renders its source verbatim
    pub fn is_static(&self) -> bool {
        matches!(self.0.compiled, Compiled::Static(_))
    }

    /// Renders this template against `ctx`, returning the rendered output.
    pub fn render<S: Serialize>(&self, ctx: S) -> anyhow::Result<String> {
        if let Compiled::Static(s) = &self.0.compiled {
            return Ok(s.clone());
        }
        self.render_prepared(&PreparedContext::new(self.0.dialect, ctx)?)
    }

    /// Renders this template against a prepared context,
    /// returning the rendered output.
    pub fn render_prepared(&self, ctx: &PreparedContext) -> anyhow::Result<String> {
        match (&self.0.compiled, ctx) {
            (Compiled::Static(s), _) => Ok(s.clone()),
            (Compiled::Jinja(cell), PreparedContext::Jinja(value)) => {
                Ok(cell.borrow_dependent().render(value)?)
            }
            (Compiled::Handlebars(template), PreparedContext::Handlebars(context)) => {
                render_handlebars(&HANDLEBARS_REGISTRY, template, context)
            }
            _ => anyhow::bail!("prepared context dialect does not match template dialect"),
        }
    }
}

/// Marks a compile error as distinct from a failure of the cache lookup machinery.
#[derive(Debug, Error)]
#[error("{0:#}")]
struct CompileFailure(anyhow::Error);

/// The reason [`TemplateEngine::compile`] failed.
#[derive(Debug, Clone, Error)]
pub enum CompileError {
    /// The source does not and will never compile.
    #[error("{0:#}")]
    Compile(Arc<anyhow::Error>),
    /// The cache could not answer, perhaps due to a timeout.
    /// Potentially retryable.
    #[error("{0:#}")]
    Cache(Arc<anyhow::Error>),
}

/// Identifies a compiled template in [`COMPILED_TEMPLATES`] by a digest of
/// its dialect, name and source.
#[derive(Clone, Debug, Hash, PartialEq, Eq)]
struct TemplateKey([u8; 32]);

impl TemplateKey {
    fn new(dialect: TemplateDialect, name: &str, source: &str) -> Self {
        let mut hasher = blake3::Hasher::new();
        hasher.update(&[dialect as u8]);
        hasher.update(&(name.len() as u64).to_le_bytes());
        hasher.update(name.as_bytes());
        hasher.update(source.as_bytes());
        Self(hasher.finalize().into())
    }
}

declare_cache! {
/// Compiled templates from the kumo-template module.
static COMPILED_TEMPLATES: LruCacheWithTtl<TemplateKey, Template>::new("compiled_templates", 1024);
}

const COMPILED_TEMPLATE_TTL: Duration = Duration::from_secs(600);

/// Compiles templates in a specific dialect and optionally
/// provides a registry for named fragments.
pub struct TemplateEngine {
    dialect: TemplateDialect,
    templates: HashMap<String, Template>,
}

impl TemplateEngine {
    pub fn new() -> Self {
        Self::with_dialect(TemplateDialect::default())
    }

    pub fn with_dialect(dialect: TemplateDialect) -> Self {
        Self {
            dialect,
            templates: HashMap::new(),
        }
    }

    pub fn dialect(&self) -> TemplateDialect {
        self.dialect
    }

    /// Add a named template with the specified source. If name ends with
    /// `.html` then automatic escaping of html entities will be performed on
    /// substitutions.
    ///
    /// A source that is free of expansion markers for the active dialect is
    /// stored as a literal static template, bypassing the compiler and
    /// evaluator of the engine entirely.
    pub fn add_template<N, S>(&mut self, name: N, source: S) -> anyhow::Result<()>
    where
        N: Into<String>,
        S: AsRef<str>,
    {
        let name = name.into();
        let template = Template::compile(self.dialect, &name, source.as_ref())?;
        self.templates.insert(name, template);
        Ok(())
    }

    /// Returns the template registered under `name` with
    /// [`TemplateEngine::add_template`].
    pub fn get_template(&self, name: &str) -> anyhow::Result<Template> {
        self.templates
            .get(name)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("template {name} is not defined"))
    }

    /// Compiles `source` as a template named `name`, without registering it.
    /// The compilation is cached.
    pub async fn compile(&self, name: &str, source: &str) -> Result<Template, CompileError> {
        let dialect = self.dialect;

        // Static fragments are returned directly, as the copy is much cheaper
        // than maintaining a cached entry.
        if source_is_static(dialect, source) {
            return Template::compile(dialect, name, source)
                .map_err(|err| CompileError::Compile(Arc::new(err)));
        }

        let key = TemplateKey::new(dialect, name, source);
        let lookup = COMPILED_TEMPLATES
            .get_or_try_insert(&key, |_| COMPILED_TEMPLATE_TTL, async {
                Template::compile(dialect, name, source).map_err(CompileFailure)
            })
            .await
            .map_err(|err| {
                if err.downcast_ref::<CompileFailure>().is_some() {
                    CompileError::Compile(err)
                } else {
                    CompileError::Cache(err)
                }
            })?;
        Ok(lookup.item)
    }

    /// Converts `context` into the native form for this engine's dialect,
    /// producing a [`PreparedContext`] that can render more than one template
    /// without being converted again.
    pub fn prepare_context<CTX>(&self, context: CTX) -> anyhow::Result<PreparedContext>
    where
        CTX: Serialize,
    {
        PreparedContext::new(self.dialect, context)
    }

    /// Converts `context` as [`TemplateEngine::prepare_context`] does, and in
    /// addition binds the whole converted value to a global variable
    /// named `alias`.
    pub fn prepare_context_with_alias<CTX>(
        &self,
        context: Arc<CTX>,
        alias: &str,
    ) -> anyhow::Result<PreparedContext>
    where
        CTX: Serialize + Send + Sync + 'static,
    {
        Ok(match self.dialect {
            TemplateDialect::Jinja => {
                PreparedContext::Jinja(JinjaValue::from_object(LazyContext {
                    inner: context,
                    alias: alias.to_string(),
                    converted: OnceLock::new(),
                }))
            }
            TemplateDialect::Static => PreparedContext::Static,
            TemplateDialect::Handlebars => {
                let mut root = serde_json::to_value(&*context)?;
                let whole = root.clone();
                if let Some(fields) = root.as_object_mut() {
                    fields.insert(alias.to_string(), whole);
                }
                PreparedContext::Handlebars(handlebars::Context::from(root))
            }
        })
    }

    /// Compiles `source` and renders it against `context` in one step.
    pub fn render<CTX>(&self, name: &str, source: &str, context: CTX) -> anyhow::Result<String>
    where
        CTX: Serialize,
    {
        Template::compile(self.dialect, name, source)?.render(context)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const DIALECTS: [TemplateDialect; 3] = [
        TemplateDialect::Jinja,
        TemplateDialect::Handlebars,
        TemplateDialect::Static,
    ];

    /// A marker-free source is a static template and renders to itself across
    /// every dialect, ignoring the supplied context.
    #[test]
    fn marker_free_source_is_static() {
        for dialect in DIALECTS {
            let mut engine = TemplateEngine::with_dialect(dialect);
            engine
                .add_template("h", "In-Reply-To: <abc@example.com>")
                .unwrap();
            let t = engine.get_template("h").unwrap();
            assert!(
                t.is_static(),
                "{dialect:?} should treat marker-free source as static"
            );
            k9::assert_equal!(
                t.render(json!({"name": "ignored"})).unwrap(),
                "In-Reply-To: <abc@example.com>".to_string()
            );
        }
    }

    /// Confirms a static template renders a marker-free source to the same
    /// bytes the real engine would have produced for it, for Jinja (whose
    /// lexer strips trailing bytes) and Handlebars (whose lexer does not).
    #[test]
    fn static_render_matches_engine_on_trailing_newline() {
        // (source, expected output) pairs, expected being what the Jinja or
        // Handlebars engine renders for that source.
        let jinja_cases = [
            ("text\n", "text"),
            ("text\r\n", "text"),
            ("text\n\n", "text\n"),
            ("text", "text"),
        ];
        for (src, expected) in jinja_cases {
            let mut engine = TemplateEngine::with_dialect(TemplateDialect::Jinja);
            engine.add_template("t", src).unwrap();
            let out = engine.get_template("t").unwrap().render(json!({})).unwrap();
            k9::assert_equal!(out, expected.to_string());
        }

        let hbs_cases = [
            ("text\n", "text\n"),
            ("text\r\n", "text\r\n"),
            ("text\n\n", "text\n\n"),
            ("text", "text"),
        ];
        for (src, expected) in hbs_cases {
            let mut engine = TemplateEngine::with_dialect(TemplateDialect::Handlebars);
            engine.add_template("t", src).unwrap();
            let out = engine.get_template("t").unwrap().render(json!({})).unwrap();
            k9::assert_equal!(out, expected.to_string());
        }

        // TemplateEngine::render, the one-shot path used when a named template
        // is not pre-added, strips the same trailing newline.
        let jinja2 = TemplateEngine::with_dialect(TemplateDialect::Jinja);
        k9::assert_equal!(
            jinja2.render("t", "text\n", json!({})).unwrap(),
            "text".to_string()
        );
    }

    /// A prepared context renders identically to passing the serde context
    /// directly, for dynamic and static templates in every dialect.
    #[test]
    fn render_prepared_matches_render() {
        for dialect in DIALECTS {
            let mut engine = TemplateEngine::with_dialect(dialect);
            engine.add_template("dyn", "hello {{ name }}").unwrap();
            engine.add_template("lit", "no markers here").unwrap();

            let ctx = json!({"name": "Alice"});
            let prepared = engine.prepare_context(&ctx).unwrap();
            for key in ["dyn", "lit"] {
                let template = engine.get_template(key).unwrap();
                k9::assert_equal!(
                    template.render_prepared(&prepared).unwrap(),
                    template.render(&ctx).unwrap()
                );
            }
        }
    }

    /// Confirms that a source containing a marker is rendered by the engine
    /// rather than treated as static, even when the marker is a comment that
    /// substitutes no value.
    #[test]
    fn marked_source_is_dynamic() {
        let mut jinja = TemplateEngine::with_dialect(TemplateDialect::Jinja);
        jinja.add_template("h", "hello {{ name }}").unwrap();
        let t = jinja.get_template("h").unwrap();
        assert!(!t.is_static());
        k9::assert_equal!(
            t.render(json!({"name": "Alice"})).unwrap(),
            "hello Alice".to_string()
        );

        let mut hbs = TemplateEngine::with_dialect(TemplateDialect::Handlebars);
        hbs.add_template("h", "hello {{ name }}").unwrap();
        assert!(!hbs.get_template("h").unwrap().is_static());

        // Jinja statement and comment markers also keep a source dynamic.
        let mut j2 = TemplateEngine::with_dialect(TemplateDialect::Jinja);
        j2.add_template("c", "{# comment #}kept").unwrap();
        assert!(!j2.get_template("c").unwrap().is_static());
    }

    /// HTML auto-escaping applies to substituted values, never to literal
    /// markup, and is selected by the `.html` name suffix in both engine
    /// dialects. A marker-free source is all literal and renders unescaped.
    #[test]
    fn html_escaping_applies_only_to_substitutions() {
        let literal = "<b>bold &amp; <i>italic</i></b>";

        for dialect in [TemplateDialect::Jinja, TemplateDialect::Handlebars] {
            let mut engine = TemplateEngine::with_dialect(dialect);
            engine.add_template("static.html", literal).unwrap();
            let static_template = engine.get_template("static.html").unwrap();
            assert!(static_template.is_static());
            k9::assert_equal!(
                static_template.render(json!({})).unwrap(),
                literal.to_string()
            );

            engine
                .add_template("dynamic.html", "<b>{{ v }}</b>")
                .unwrap();
            let dynamic_template = engine.get_template("dynamic.html").unwrap();
            assert!(!dynamic_template.is_static());
            k9::assert_equal!(
                dynamic_template.render(json!({"v": "a & b"})).unwrap(),
                "<b>a &amp; b</b>".to_string()
            );

            engine
                .add_template("dynamic.txt", "<b>{{ v }}</b>")
                .unwrap();
            k9::assert_equal!(
                engine
                    .get_template("dynamic.txt")
                    .unwrap()
                    .render(json!({"v": "a & b"}))
                    .unwrap(),
                "<b>a & b</b>".to_string()
            );
        }
    }

    /// A compile error reports the template name and the offending line.
    #[test]
    fn compile_error_names_template() {
        let mut engine = TemplateEngine::with_dialect(TemplateDialect::Jinja);
        let err = engine.add_template("t.txt", "ok\n{{ name").unwrap_err();
        k9::snapshot!(
            err.to_string(),
            "syntax error: unexpected end of input, expected end of variable block (in template 't.txt' line 2: '{{ name')"
        );
    }

    /// A template from the cached compile path renders the same output as the
    /// same source registered with add_template, in every dialect.
    #[tokio::test]
    async fn compile_matches_add_template() {
        for dialect in DIALECTS {
            let ctx = json!({"name": "Alice"});
            for source in ["hello {{ name }}", "no markers here", "trailing\n"] {
                let mut engine = TemplateEngine::with_dialect(dialect);
                engine.add_template("t.txt", source).unwrap();
                let registered = engine.get_template("t.txt").unwrap();

                let compiled = engine.compile("t.txt", source).await.unwrap();
                k9::assert_equal!(compiled.is_static(), registered.is_static());
                k9::assert_equal!(
                    compiled.render(&ctx).unwrap(),
                    registered.render(&ctx).unwrap()
                );
            }
        }
    }

    /// Repeated compiles of the same dialect, name and source share one
    /// compilation. A change to any of the three compiles afresh.
    #[tokio::test]
    async fn compile_shares_one_compilation() {
        let source = "cached {{ name }} compile_shares_one_compilation";
        let jinja = TemplateEngine::with_dialect(TemplateDialect::Jinja);
        let first = jinja.compile("t.txt", source).await.unwrap();
        let again = jinja.compile("t.txt", source).await.unwrap();
        assert!(Arc::ptr_eq(&first.0, &again.0));

        let other_name = jinja.compile("u.txt", source).await.unwrap();
        assert!(!Arc::ptr_eq(&first.0, &other_name.0));

        let hbs = TemplateEngine::with_dialect(TemplateDialect::Handlebars);
        let other_dialect = hbs.compile("t.txt", source).await.unwrap();
        assert!(!Arc::ptr_eq(&first.0, &other_dialect.0));
        k9::assert_equal!(other_dialect.dialect(), TemplateDialect::Handlebars);

        k9::assert_equal!(
            first.render(json!({"name": "Alice"})).unwrap(),
            "cached Alice compile_shares_one_compilation".to_string()
        );
    }

    // Rendering a static template skips the conversion step where
    // serialization would fail, unlike a dynamic template of the same dialect.
    #[test]
    fn static_render_does_not_serialize_context() {
        struct Unserializable;

        impl Serialize for Unserializable {
            fn serialize<S>(&self, _ser: S) -> Result<S::Ok, S::Error>
            where
                S: serde::Serializer,
            {
                Err(serde::ser::Error::custom("cannot serialize"))
            }
        }

        for dialect in [TemplateDialect::Jinja, TemplateDialect::Handlebars] {
            let mut engine = TemplateEngine::with_dialect(dialect);
            engine.add_template("lit", "no markers here").unwrap();
            k9::assert_equal!(
                engine
                    .get_template("lit")
                    .unwrap()
                    .render(Unserializable)
                    .unwrap(),
                "no markers here".to_string()
            );

            engine.add_template("dyn", "hello {{ name }}").unwrap();
            let err = engine
                .get_template("dyn")
                .unwrap()
                .render(Unserializable)
                .unwrap_err();
            let expected = match dialect {
                TemplateDialect::Jinja => {
                    "could not serialize to value: cannot serialize".to_string()
                }
                _ => "cannot serialize".to_string(),
            };
            k9::assert_equal!(err.to_string(), expected);
        }
    }

    /// A compile failure comes back through the cache as the `Compile`
    /// variant with its message intact.
    #[tokio::test]
    async fn compile_reports_errors() {
        let engine = TemplateEngine::with_dialect(TemplateDialect::Jinja);
        let err = engine.compile("bad.txt", "{{ nope").await.unwrap_err();
        assert!(
            matches!(err, CompileError::Compile(_)),
            "expected Compile, got {err:?}"
        );
        k9::snapshot!(
            err.to_string(),
            "syntax error: unexpected end of input, expected end of variable block (in template 'bad.txt' line 1: '{{ nope')"
        );
    }

    /// A log record that counts how many times it is serialized.
    struct CountingRecord {
        serializations: std::sync::atomic::AtomicUsize,
    }

    impl Serialize for CountingRecord {
        fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
            self.serializations
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            json!({"type": "Delivery", "size": 1047}).serialize(serializer)
        }
    }

    fn counting_record() -> Arc<CountingRecord> {
        Arc::new(CountingRecord {
            serializations: std::sync::atomic::AtomicUsize::new(0),
        })
    }

    /// A record that serializes to JSON with a `log_record` key that
    /// (potentially) shadows the global alias name.
    struct ShadowingRecord;

    impl Serialize for ShadowingRecord {
        fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
            json!({"type": "Delivery", "log_record": "shadowed"}).serialize(serializer)
        }
    }

    /// Checks that an aliased context exposes the value's fields at the top
    /// level and the whole value under the alias, in both engine dialects.
    #[test]
    fn aliased_context_exposes_fields_and_whole_value() {
        for dialect in [TemplateDialect::Jinja, TemplateDialect::Handlebars] {
            let mut engine = TemplateEngine::with_dialect(dialect);
            engine
                .add_template("t", "{{ type }}/{{ size }} {{ log_record.type }}")
                .unwrap();
            let prepared = engine
                .prepare_context_with_alias(counting_record(), "log_record")
                .unwrap();
            k9::assert_equal!(
                engine
                    .get_template("t")
                    .unwrap()
                    .render_prepared(&prepared)
                    .unwrap(),
                "Delivery/1047 Delivery".to_string()
            );
        }

        let mut jinja = TemplateEngine::with_dialect(TemplateDialect::Jinja);
        jinja
            .add_template("json", "{{ log_record | tojson }}")
            .unwrap();
        let prepared = jinja
            .prepare_context_with_alias(counting_record(), "log_record")
            .unwrap();
        k9::assert_equal!(
            jinja
                .get_template("json")
                .unwrap()
                .render_prepared(&prepared)
                .unwrap(),
            r#"{"size":1047,"type":"Delivery"}"#.to_string()
        );
    }

    /// Checks that the alias wins over a field of the same name in both
    /// engine dialects, with that field still reachable through the alias.
    #[test]
    fn aliased_context_alias_wins_over_field() {
        for dialect in [TemplateDialect::Jinja, TemplateDialect::Handlebars] {
            let mut engine = TemplateEngine::with_dialect(dialect);
            engine
                .add_template("t", "{{ log_record.type }}/{{ log_record.log_record }}")
                .unwrap();
            let prepared = engine
                .prepare_context_with_alias(Arc::new(ShadowingRecord), "log_record")
                .unwrap();
            k9::assert_equal!(
                engine
                    .get_template("t")
                    .unwrap()
                    .render_prepared(&prepared)
                    .unwrap(),
                "Delivery/shadowed".to_string()
            );
        }
    }

    /// Reports the serialization failure for both a top-level field and the
    /// alias itself when an aliased Jinja context value fails to serialize,
    /// rather than rendering an empty substitution.
    #[test]
    fn aliased_context_reports_serialization_failure() {
        struct Unserializable;

        impl Serialize for Unserializable {
            fn serialize<S>(&self, _ser: S) -> Result<S::Ok, S::Error>
            where
                S: serde::Serializer,
            {
                Err(serde::ser::Error::custom("cannot serialize"))
            }
        }

        let mut engine = TemplateEngine::with_dialect(TemplateDialect::Jinja);
        engine.add_template("field", "{{ name }}").unwrap();
        engine.add_template("alias", "{{ log_record }}").unwrap();
        let prepared = engine
            .prepare_context_with_alias(Arc::new(Unserializable), "log_record")
            .unwrap();
        for (name, expected) in [
            (
                "field",
                "could not serialize to value: cannot serialize (in field:1)",
            ),
            (
                "alias",
                "could not serialize to value: cannot serialize (in alias:1)",
            ),
        ] {
            let err = engine
                .get_template(name)
                .unwrap()
                .render_prepared(&prepared)
                .unwrap_err();
            k9::assert_equal!(err.to_string(), expected.to_string());
        }
    }

    /// Converts a Jinja aliased context value at most once, on the first name
    /// lookup of any kind, however many fields and renders read it.
    /// Names not defined in the value must still be resolved against
    /// engine-provided globals.
    #[test]
    fn aliased_context_serializes_at_most_once() {
        let mut engine = TemplateEngine::with_dialect(TemplateDialect::Jinja);
        engine
            .add_template("fields", "{{ type }} {{ size }} {{ log_record.size }}")
            .unwrap();
        engine
            .add_template("globals", "{{ range(3) | length }}")
            .unwrap();

        let record = counting_record();
        let prepared = engine
            .prepare_context_with_alias(record.clone(), "log_record")
            .unwrap();
        let reads = || {
            record
                .serializations
                .load(std::sync::atomic::Ordering::Relaxed)
        };

        k9::assert_equal!(reads(), 0);
        let globals = engine.get_template("globals").unwrap();
        k9::assert_equal!(globals.render_prepared(&prepared).unwrap(), "3".to_string());
        k9::assert_equal!(reads(), 1);

        let fields = engine.get_template("fields").unwrap();
        for _ in 0..3 {
            k9::assert_equal!(
                fields.render_prepared(&prepared).unwrap(),
                "Delivery 1047 1047".to_string()
            );
        }
        k9::assert_equal!(reads(), 1);
    }
}
