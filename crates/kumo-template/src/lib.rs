use minijinja::{Environment, Template as JinjaTemplate, Value as JinjaValue};
use minijinja_contrib::add_to_environment;
use self_cell::self_cell;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TemplateDialect {
    #[default]
    Jinja,
    Static,
}

enum Engine {
    Jinja { env: Environment<'static> },
    Static { env: HashMap<String, String> },
}

pub enum Template<'env, 'source> {
    Jinja(JinjaTemplate<'env, 'source>),
    Static(&'env str),
}

impl<'env, 'source> Template<'env, 'source> {
    pub fn render<S: Serialize>(&self, ctx: S) -> anyhow::Result<String> {
        match &self {
            Self::Jinja(t) => Ok(t.render(ctx)?),
            Self::Static(s) => Ok(s.to_string()),
        }
    }

    pub fn render_to_write<S: Serialize, W: std::io::Write>(
        &self,
        ctx: S,
        mut w: W,
    ) -> anyhow::Result<()> {
        match &self {
            Self::Jinja(t) => {
                t.render_to_write(ctx, w)?;
                Ok(())
            }
            Self::Static(s) => {
                w.write(s.as_bytes())?;
                Ok(())
            }
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
                Self {
                    engine: Engine::Jinja { env },
                }
            }
            TemplateDialect::Static => Self {
                engine: Engine::Static {
                    env: HashMap::new(),
                },
            },
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
            Engine::Jinja { env } => Ok(env.add_template_owned(name.into(), source.into())?),
            Engine::Static { env } => {
                env.insert(name.into(), source.into());
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
        }
    }

    /// Define a global value that can be reference by all templates
    pub fn add_global<N, V>(&mut self, name: N, value: V)
    where
        N: Into<String>,
        V: Serialize,
    {
        match &mut self.engine {
            Engine::Jinja { env } => env.add_global(name.into(), JinjaValue::from_serialize(value)),
            Engine::Static { .. } => { /* NOP */ }
        }
    }

    pub fn render<CTX>(&self, name: &str, source: &str, context: CTX) -> anyhow::Result<String>
    where
        CTX: serde::Serialize,
    {
        match &self.engine {
            Engine::Jinja { env } => Ok(env.render_named_str(name, source, context)?),
            Engine::Static { .. } => Ok(source.to_string()),
        }
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
