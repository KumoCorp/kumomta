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

                let mut render_context = handlebars::RenderContext::new(None);
                render_context.set_recursive_lookup(true);
                let is_html = template
                    .name
                    .as_deref()
                    .map(|name| name.ends_with(".html"))
                    .unwrap_or(false);
                render_context.set_disable_escape(!is_html);

                let output = template.renders(registry, &context, &mut render_context)?;
                w.write_all(output.as_bytes())?;
                Ok(())
            }
        }
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
}
