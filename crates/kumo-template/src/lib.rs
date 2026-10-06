use handlebars::{Handlebars, Renderable, Template as HandlebarsTemplate};
use minijinja::{Environment, Template as JinjaTemplate, Value as JinjaValue};
use minijinja_contrib::add_to_environment;
use self_cell::self_cell;
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::collections::HashMap;
use std::fmt::Write;

#[derive(Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TemplateDialect {
    #[default]
    Jinja,
    Static,
    Handlebars,
}

#[derive(Debug, Clone)]
pub enum TemplateContext {
    Jinja(minijinja::Value),
    Static,
    Handlebars(handlebars::Context),
}

enum Engine {
    Jinja {
        env: Environment<'static>,
    },
    Static {
        env: HashMap<String, String>,
    },
    Handlebars {
        registry: Handlebars<'static>,
        globals: HashMap<String, serde_json::Value>,
    },
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
    pub fn render<S: Serialize>(&self, ctx: S) -> anyhow::Result<String> {
        match self {
            Self::Jinja(t) => Ok(t.render(ctx)?),
            Self::Static(s) => Ok(s.to_string()),
            Self::Handlebars { engine, .. } => {
                let context = engine.create_context(ctx)?;
                self.render_with_context(&context)
            }
        }
    }

    pub fn render_to_write<S: Serialize, W: std::io::Write>(
        &self,
        ctx: S,
        w: W,
    ) -> anyhow::Result<()> {
        match self {
            Self::Jinja(t) => {
                t.render_captured_to(ctx, w)?;
                Ok(())
            }
            Self::Static(s) => {
                let mut w = w;
                w.write_all(s.as_bytes())?;
                Ok(())
            }
            Self::Handlebars { engine, .. } => {
                let context = engine.create_context(ctx)?;
                self.render_with_context_to_write(&context, w)
            }
        }
    }

    pub fn render_with_context(&self, ctx: &TemplateContext) -> anyhow::Result<String> {
        match (self, ctx) {
            (Self::Jinja(t), TemplateContext::Jinja(v)) => Ok(t.render(v)?),
            (Self::Static(s), TemplateContext::Static) => Ok(s.to_string()),
            (Self::Handlebars { .. }, TemplateContext::Handlebars(_)) => {
                let mut output: Vec<u8> = vec![];
                self.render_with_context_to_write(ctx, &mut output)?;
                Ok(String::from_utf8(output)?)
            }
            _ => anyhow::bail!("mismatched template dialect and TemplateContext"),
        }
    }

    pub fn render_with_context_to_write<W: std::io::Write>(
        &self,
        ctx: &TemplateContext,
        mut w: W,
    ) -> anyhow::Result<()> {
        match (self, ctx) {
            (Self::Jinja(t), TemplateContext::Jinja(v)) => {
                t.render_captured_to(v, w)?;
                Ok(())
            }
            (Self::Static(s), TemplateContext::Static) => {
                w.write_all(s.as_bytes())?;
                Ok(())
            }
            (Self::Handlebars { engine, template }, TemplateContext::Handlebars(context)) => {
                let Engine::Handlebars { registry, .. } = &engine.engine else {
                    anyhow::bail!("impossible Handlebars Template vs. TemplateEngine state")
                };

                let mut render_context = handlebars::RenderContext::new(None);
                render_context.set_recursive_lookup(true);
                let is_html = template
                    .name
                    .as_deref()
                    .map(|name| name.ends_with(".html"))
                    .unwrap_or(false);
                render_context.set_disable_escape(!is_html);

                let output = template.renders(registry, context, &mut render_context)?;
                w.write_all(output.as_bytes())?;
                Ok(())
            }
            _ => anyhow::bail!("mismatched template dialect and TemplateContext"),
        }
    }
}

/// Holds a set of templates
pub struct TemplateEngine {
    engine: Engine,
}

impl TemplateEngine {
    pub fn new() -> Self {
        Self::with_dialect(TemplateDialect::default())
    }

    pub fn with_dialect(dialect: TemplateDialect) -> Self {
        match dialect {
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

                Self {
                    engine: Engine::Jinja { env },
                }
            }
            TemplateDialect::Static => Self {
                engine: Engine::Static {
                    env: HashMap::new(),
                },
            },
            TemplateDialect::Handlebars => {
                let mut registry = Handlebars::new();
                registry.set_recursive_lookup(true);
                Self {
                    engine: Engine::Handlebars {
                        registry,
                        globals: HashMap::new(),
                    },
                }
            }
        }
    }

    /// Add a named template with the specified source.
    /// If name ends with `.html` then automatical escaping of html entities
    /// will be performed on substitutions.
    pub fn add_template<N, S>(&mut self, name: N, source: S) -> anyhow::Result<()>
    where
        N: Into<String>,
        S: Into<String>,
    {
        match &mut self.engine {
            Engine::Jinja { env } => {
                let source: Cow<'_, str> = source.into().into();

                Ok(env
                    .add_template_owned(name.into(), source.clone())
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
            Engine::Static { env } => {
                env.insert(name.into(), source.into());
                Ok(())
            }
            Engine::Handlebars { registry, .. } => {
                registry.register_template_string(&name.into(), source.into())?;
                Ok(())
            }
        }
    }

    /// Get a reference to a named template
    pub fn get_template(&self, name: &str) -> anyhow::Result<Template<'_, '_>> {
        match &self.engine {
            Engine::Jinja { env } => Ok(Template::Jinja(env.get_template(name)?)),
            Engine::Static { env } => {
                Ok(Template::Static(env.get(name).ok_or_else(|| {
                    anyhow::anyhow!("template {name} is not defined")
                })?))
            }
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
            Engine::Static { .. } => { /* NOP */ }
            Engine::Handlebars { globals, .. } => {
                globals.insert(name.into(), serde_json::to_value(value)?);
            }
        }

        Ok(())
    }

    pub fn create_context<S: Serialize>(&self, ctx: S) -> anyhow::Result<TemplateContext> {
        match &self.engine {
            Engine::Jinja { .. } => Ok(TemplateContext::Jinja(minijinja::Value::from_serialize(
                ctx,
            ))),
            Engine::Static { .. } => Ok(TemplateContext::Static),
            Engine::Handlebars { globals, .. } => {
                let context = serde_json::to_value(ctx)?;
                let context = merge_contexts(globals, context);
                let context = handlebars::Context::wraps(context)?;
                Ok(TemplateContext::Handlebars(context))
            }
        }
    }

    pub fn render<CTX>(&self, name: &str, source: &str, context: CTX) -> anyhow::Result<String>
    where
        CTX: serde::Serialize,
    {
        match &self.engine {
            Engine::Jinja { env } => Ok(env.render_named_str(name, source, context)?),
            Engine::Static { .. } => Ok(source.to_string()),
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
    /// use kumo_template::{CompiledTemplates, TemplateEngine, TemplateList};
    ///
    /// fn get_templates<'b>(
    ///   engine: &'b TemplateEngine
    /// ) -> anyhow::Result<TemplateList<'b>> {
    ///   let mut templates = vec![];
    ///   templates.push(engine.get_template("something")?);
    ///   Ok(templates)
    /// }
    ///
    /// let mut engine = TemplateEngine::new();
    /// engine.add_template("something", "some text")?;
    /// let compiled = CompiledTemplates::try_new(engine, |engine| {
    ///   get_templates(engine)
    /// })?;
    /// # Ok::<(), anyhow::Error>(())
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

    #[test]
    fn test_template_context_jinja() {
        let mut engine = TemplateEngine::with_dialect(TemplateDialect::Jinja);
        engine.add_template("t1", "Hello, {{ name }}!").unwrap();
        engine.add_template("t2", "Goodbye, {{ name }}!").unwrap();

        let ctx = engine.create_context(json!({"name": "World"})).unwrap();
        let t1 = engine.get_template("t1").unwrap();
        let t2 = engine.get_template("t2").unwrap();

        assert_eq!(t1.render_with_context(&ctx).unwrap(), "Hello, World!");
        assert_eq!(t2.render_with_context(&ctx).unwrap(), "Goodbye, World!");
    }

    #[test]
    fn test_template_context_handlebars() {
        let mut engine = TemplateEngine::with_dialect(TemplateDialect::Handlebars);
        engine
            .add_template("html_part.html", "<b>Hello, {{ name }}</b>")
            .unwrap();
        engine
            .add_template("text_part.txt", "Hello, {{ name }}")
            .unwrap();

        let ctx = engine
            .create_context(json!({"name": "Alice & Bob"}))
            .unwrap();
        let html_tmpl = engine.get_template("html_part.html").unwrap();
        let text_tmpl = engine.get_template("text_part.txt").unwrap();

        // html_part.html auto-escapes HTML entities
        assert_eq!(
            html_tmpl.render_with_context(&ctx).unwrap(),
            "<b>Hello, Alice &amp; Bob</b>"
        );
        // text_part.txt does not escape
        assert_eq!(
            text_tmpl.render_with_context(&ctx).unwrap(),
            "Hello, Alice & Bob"
        );
    }

    #[test]
    fn test_template_context_static() {
        let mut engine = TemplateEngine::with_dialect(TemplateDialect::Static);
        engine.add_template("s1", "Static text").unwrap();

        let ctx = engine.create_context(json!({})).unwrap();
        let tmpl = engine.get_template("s1").unwrap();
        assert_eq!(tmpl.render_with_context(&ctx).unwrap(), "Static text");
    }
}
