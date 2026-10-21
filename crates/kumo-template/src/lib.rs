use handlebars::{Handlebars, Renderable, Template as HandlebarsTemplate};
use minijinja::{Environment, Template as JinjaTemplate, Value as JinjaValue};
use minijinja_contrib::add_to_environment;
use self_cell::self_cell;
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::collections::HashMap;
use std::fmt::Write;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TemplateDialect {
    #[default]
    Jinja,
    Static,
    Handlebars,
}

enum Engine {
    Jinja {
        env: Environment<'static>,
    },
    Static,
    Handlebars {
        registry: Handlebars<'static>,
        globals: HashMap<String, serde_json::Value>,
    },
}

/// Returns true when `source` contains none of the expansion markers of
/// `dialect`, meaning the engine would render it with no substitution.
pub fn source_is_static(dialect: TemplateDialect, source: &str) -> bool {
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

pub enum Template<'env, 'source> {
    Jinja(JinjaTemplate<'env, 'source>),
    Static(&'env str),
    Handlebars {
        engine: &'env TemplateEngine,
        template: &'env HandlebarsTemplate,
    },
}

impl<'env, 'source> Template<'env, 'source> {
    /// Returns true when this template renders its source verbatim, ignoring
    /// the context.
    pub fn is_static(&self) -> bool {
        matches!(self, Self::Static(_))
    }

    pub fn render<S: Serialize>(&self, ctx: S) -> anyhow::Result<String> {
        match &self {
            Self::Jinja(t) => Ok(t.render(ctx)?),
            Self::Static(s) => Ok(s.to_string()),
            Self::Handlebars { .. } => {
                let mut output: Vec<u8> = vec![];
                self.render_to_write(&ctx, &mut output)?;
                Ok(String::from_utf8(output)?)
            }
        }
    }

    pub fn render_to_write<S: Serialize, W: std::io::Write>(
        &self,
        ctx: S,
        mut w: W,
    ) -> anyhow::Result<()> {
        match &self {
            Self::Jinja(t) => {
                t.render_captured_to(ctx, w)?;
                Ok(())
            }
            Self::Static(s) => {
                w.write_all(s.as_bytes())?;
                Ok(())
            }
            Self::Handlebars { engine, template } => {
                let Engine::Handlebars { registry, globals } = &engine.engine else {
                    anyhow::bail!("impossible Handlebars Template vs. TemplateEngine state")
                };

                let context = serde_json::to_value(ctx)?;
                let context = merge_contexts(globals, context);
                let context = handlebars::Context::wraps(context)?;

                let output = render_handlebars(registry, template, &context)?;
                w.write_all(output.as_bytes())?;
                Ok(())
            }
        }
    }

    /// Renders this template against a context already converted to the native
    /// form of the engine, returning the rendered output.
    pub fn render_prepared(&self, ctx: &PreparedContext) -> anyhow::Result<String> {
        match (self, ctx) {
            (Self::Jinja(t), PreparedContext::Jinja(value)) => Ok(t.render(value)?),
            (Self::Static(s), _) => Ok(s.to_string()),
            (Self::Handlebars { engine, template }, PreparedContext::Handlebars(context)) => {
                let Engine::Handlebars { registry, .. } = &engine.engine else {
                    anyhow::bail!("impossible Handlebars Template vs. TemplateEngine state")
                };
                render_handlebars(registry, template, context)
            }
            _ => anyhow::bail!("prepared context dialect does not match template dialect"),
        }
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

/// A rendering context converted once into the native form of the active
/// dialect. Obtained from [`TemplateEngine::prepare_context`] and rendered
/// against with [`Template::render_prepared`].
pub enum PreparedContext {
    Jinja(JinjaValue),
    Static,
    Handlebars(handlebars::Context),
}

/// A single compiled template that owns its engine.
///
/// Unlike [`Template`], which borrows from a [`TemplateEngine`], this has no
/// lifetime parameters and is `Send + Sync`, so it can be cached and shared
/// across requests.
pub struct StandaloneTemplate {
    name: String,
    inner: StandaloneInner,
}

enum StandaloneInner {
    /// A marker-free source, held as the text it renders to, with no engine
    Literal(String),
    Compiled(EngineAndTemplate),
}

type SingleTemplate<'a> = Template<'a, 'a>;

self_cell!(
    /// The engine together with its template, resolved once at compile time
    /// so that rendering does not look the template up again.
    struct EngineAndTemplate {
        owner: TemplateEngine,
        #[covariant]
        dependent: SingleTemplate,
    }
);

impl StandaloneTemplate {
    /// Compile `source` as a template named `name`. If name ends with `.html`
    /// then automatic escaping of html entities will be performed on
    /// substitutions.
    ///
    /// A source that is free of expansion markers for `dialect` is held as a
    /// literal, without creating an engine.
    pub fn compile<N, S>(dialect: TemplateDialect, name: N, source: S) -> anyhow::Result<Self>
    where
        N: Into<String>,
        S: Into<String>,
    {
        let name = name.into();
        let mut source = source.into();

        if source_is_static(dialect, &source) {
            source.truncate(trim_static_source(dialect, &source).len());
            return Ok(Self {
                name,
                inner: StandaloneInner::Literal(source),
            });
        }

        let mut engine = TemplateEngine::with_dialect(dialect);
        engine.add_template(name.clone(), source)?;
        let compiled = EngineAndTemplate::try_new(engine, |engine| engine.get_template(&name))?;
        Ok(Self {
            name,
            inner: StandaloneInner::Compiled(compiled),
        })
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns true when the source had no expansion markers, so rendering
    /// returns it verbatim.
    pub fn is_static(&self) -> bool {
        matches!(self.inner, StandaloneInner::Literal(_))
    }

    pub fn render<S: Serialize>(&self, ctx: S) -> anyhow::Result<String> {
        match &self.inner {
            StandaloneInner::Literal(s) => Ok(s.clone()),
            StandaloneInner::Compiled(compiled) => compiled.borrow_dependent().render(ctx),
        }
    }

    /// Renders against a context prepared by a [`TemplateEngine`] of the same
    /// dialect. For Handlebars, the globals of the engine that prepared the
    /// context are the ones that apply.
    pub fn render_prepared(&self, ctx: &PreparedContext) -> anyhow::Result<String> {
        match &self.inner {
            StandaloneInner::Literal(s) => Ok(s.clone()),
            StandaloneInner::Compiled(compiled) => compiled.borrow_dependent().render_prepared(ctx),
        }
    }
}

impl std::fmt::Debug for StandaloneTemplate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StandaloneTemplate")
            .field("name", &self.name)
            .field("is_static", &self.is_static())
            .finish_non_exhaustive()
    }
}

/// Holds a set of templates
pub struct TemplateEngine {
    dialect: TemplateDialect,
    engine: Engine,
    literals: HashMap<String, String>,
}

impl TemplateEngine {
    pub fn new() -> Self {
        Self::with_dialect(TemplateDialect::default())
    }

    pub fn with_dialect(dialect: TemplateDialect) -> Self {
        let engine = match dialect {
            TemplateDialect::Jinja => {
                let mut env = Environment::new();
                env.set_unknown_method_callback(
                    minijinja_contrib::pycompat::unknown_method_callback,
                );
                add_to_environment(&mut env);

                env.add_filter(
                    "normalize_smtp_response",
                    mod_smtp_response_normalize::normalize,
                );

                Engine::Jinja { env }
            }
            TemplateDialect::Static => Engine::Static,
            TemplateDialect::Handlebars => {
                let mut registry = Handlebars::new();
                registry.set_recursive_lookup(true);
                Engine::Handlebars {
                    registry,
                    globals: HashMap::new(),
                }
            }
        };
        Self {
            dialect,
            engine,
            literals: HashMap::new(),
        }
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
        S: Into<String>,
    {
        let name = name.into();
        let mut source = source.into();

        if source_is_static(self.dialect, &source) {
            source.truncate(trim_static_source(self.dialect, &source).len());
            self.literals.insert(name, source);
            return Ok(());
        }

        match &mut self.engine {
            Engine::Jinja { env } => {
                let source: Cow<'_, str> = source.into();

                Ok(env
                    .add_template_owned(name, source.clone())
                    .map_err(|err| {
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
                            let truncated_line =
                                &source_line[..source_line.ceil_char_boundary(1024)];

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
                    })?)
            }
            Engine::Static => anyhow::bail!("Static dialect cannot hold a dynamic template"),
            Engine::Handlebars { registry, .. } => {
                registry.register_template_string(&name, source)?;
                Ok(())
            }
        }
    }

    /// Get a reference to a named template
    pub fn get_template(&self, name: &str) -> anyhow::Result<Template<'_, '_>> {
        if let Some(source) = self.literals.get(name) {
            return Ok(Template::Static(source));
        }
        match &self.engine {
            Engine::Jinja { env } => Ok(Template::Jinja(env.get_template(name)?)),
            Engine::Static => anyhow::bail!("template {name} is not defined"),
            Engine::Handlebars { registry, .. } => {
                let template = registry
                    .get_template(name)
                    .ok_or_else(|| anyhow::anyhow!("template {name} is not defined"))?;

                Ok(Template::Handlebars {
                    engine: self,
                    template,
                })
            }
        }
    }

    /// Converts a context into the native form of the active dialect, merging in
    /// the globals previously added with [`TemplateEngine::add_global`].
    pub fn prepare_context<CTX>(&self, context: CTX) -> anyhow::Result<PreparedContext>
    where
        CTX: Serialize,
    {
        match &self.engine {
            Engine::Jinja { .. } => Ok(PreparedContext::Jinja(JinjaValue::from_serialize(context))),
            Engine::Static => Ok(PreparedContext::Static),
            Engine::Handlebars { globals, .. } => {
                let context = serde_json::to_value(context)?;
                let context = merge_contexts(globals, context);
                Ok(PreparedContext::Handlebars(handlebars::Context::wraps(
                    context,
                )?))
            }
        }
    }

    /// Define a global value that can be reference by all templates
    pub fn add_global<N, V>(&mut self, name: N, value: V) -> anyhow::Result<()>
    where
        N: Into<String>,
        V: Serialize,
    {
        match &mut self.engine {
            Engine::Jinja { env } => env.add_global(name.into(), JinjaValue::from_serialize(value)),
            Engine::Static => { /* NOP */ }
            Engine::Handlebars { globals, .. } => {
                globals.insert(name.into(), serde_json::to_value(value)?);
            }
        }

        Ok(())
    }

    pub fn render<CTX>(&self, name: &str, source: &str, context: CTX) -> anyhow::Result<String>
    where
        CTX: serde::Serialize,
    {
        if source_is_static(self.dialect, source) {
            return Ok(trim_static_source(self.dialect, source).to_string());
        }
        match &self.engine {
            Engine::Jinja { env } => Ok(env.render_named_str(name, source, context)?),
            Engine::Static => Ok(source.to_string()),
            Engine::Handlebars { .. } => {
                let template = HandlebarsTemplate::compile_with_name(source, name.to_string())?;
                let template = Template::Handlebars {
                    engine: self,
                    template: &template,
                };

                template.render(context)
            }
        }
    }
}

fn merge_contexts(
    globals: &HashMap<String, serde_json::Value>,
    over: serde_json::Value,
) -> serde_json::Value {
    match over {
        serde_json::Value::Object(mut obj) => {
            for (k, v) in globals {
                if !obj.contains_key(k) {
                    obj.insert(k.into(), v.clone());
                }
            }
            serde_json::Value::Object(obj)
        }
        other => other,
    }
}

pub type TemplateList<'a> = Vec<Template<'a, 'a>>;

self_cell!(
    /// CompiledTemplates is useful when you have a set of templates
    /// that you will expand frequently in a tight loop.
    /// Because the underlying crate returns only references to `Template`s,
    /// it is a covariant, self-referential structure that needs to be
    /// constructed like this:
    ///
    /// ```rust
    /// fn get_templates<'b>(
    ///   engine: &'b TemplateEngine
    /// ) -> anyhow::Result<TemplateList<'b>> {
    ///   let mut templates = vec![];
    ///   templates.push(engine.get_template("something")?);
    ///   Ok(templates)
    /// }
    ///
    /// let engine = TemplateEngine::new();
    /// engine.add_template("something", "some text")?;
    /// let compiled = CompiledTemplates::try_new(engine, |engine| {
    ///   get_templates(engine)
    /// });
    /// ```
    pub struct CompiledTemplates {
        owner: TemplateEngine,
        #[covariant]
        dependent: TemplateList,
    }
);

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// A marker-free source is a static template and renders to itself across
    /// every dialect, ignoring the supplied context.
    #[test]
    fn marker_free_source_is_static() {
        for dialect in [
            TemplateDialect::Jinja,
            TemplateDialect::Handlebars,
            TemplateDialect::Static,
        ] {
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

    /// Confirms `Template::Static` renders a marker-free source to the same
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
        for dialect in [
            TemplateDialect::Jinja,
            TemplateDialect::Handlebars,
            TemplateDialect::Static,
        ] {
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
    /// markup. A marker-free source is all literal and renders unescaped.
    #[test]
    fn html_escaping_applies_only_to_substitutions() {
        let literal = "<b>bold &amp; <i>italic</i></b>";

        let mut engine = TemplateEngine::with_dialect(TemplateDialect::Handlebars);
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
    }

    fn assert_send_sync<T: Send + Sync>() {}

    #[test]
    fn standalone_template_is_send_sync() {
        assert_send_sync::<StandaloneTemplate>();
    }

    /// A standalone template renders identically to the same source added to
    /// a TemplateEngine, for dynamic and static sources in every dialect,
    /// whether rendered with a serde context or a prepared context.
    #[test]
    fn standalone_renders_like_engine() {
        let ctx = json!({"name": "Tom & <Jerry>", "items": [1, 2]});
        for dialect in [
            TemplateDialect::Jinja,
            TemplateDialect::Handlebars,
            TemplateDialect::Static,
        ] {
            for (name, source) in [
                ("text_body.txt", "Hi {{ name }}"),
                ("html_body.html", "<p>Hi {{ name }}</p>"),
                ("text_body.txt", "no markers here\n"),
            ] {
                let mut engine = TemplateEngine::with_dialect(dialect);
                engine.add_template(name, source).unwrap();
                let expected = engine.get_template(name).unwrap().render(&ctx).unwrap();
                let prepared = engine.prepare_context(&ctx).unwrap();

                let standalone = StandaloneTemplate::compile(dialect, name, source).unwrap();
                k9::assert_equal!(standalone.name(), name);
                k9::assert_equal!(standalone.render(&ctx).unwrap(), expected);
                k9::assert_equal!(standalone.render_prepared(&prepared).unwrap(), expected);
            }
        }
    }

    /// A marker-free source becomes a literal that needs no engine; a source
    /// with markers is compiled.
    #[test]
    fn standalone_static_source_is_literal() {
        for dialect in [
            TemplateDialect::Jinja,
            TemplateDialect::Handlebars,
            TemplateDialect::Static,
        ] {
            let literal =
                StandaloneTemplate::compile(dialect, "text_body.txt", "no markers").unwrap();
            assert!(literal.is_static(), "{dialect:?}");
        }
        for dialect in [TemplateDialect::Jinja, TemplateDialect::Handlebars] {
            let compiled =
                StandaloneTemplate::compile(dialect, "text_body.txt", "hi {{ name }}").unwrap();
            assert!(!compiled.is_static(), "{dialect:?}");
        }
    }

    #[test]
    fn standalone_reports_syntax_errors() {
        let err =
            StandaloneTemplate::compile(TemplateDialect::Jinja, "text_body.txt", "Hi {{ name")
                .unwrap_err();
        assert!(
            format!("{err:#}").contains("(in template 'text_body.txt' line 1: 'Hi {{ name')"),
            "{err:#}"
        );
    }
}
