use minijinja::Environment;
pub use minijinja::{context, Error, Template, Value};
use minijinja_contrib::add_to_environment;
use self_cell::self_cell;

/// Holds a set of templates
pub struct TemplateEngine {
    env: Environment<'static>,
}

impl TemplateEngine {
    pub fn new() -> Self {
        let mut env = Environment::new();
        add_to_environment(&mut env);
        Self { env }
    }

    /// Add a named template with the specified source.
    /// If name ends with `.html` then automatical escaping of html entities
    /// will be performed on substitutions.
    pub fn add_template<N, S>(&mut self, name: N, source: S) -> Result<(), Error>
    where
        N: Into<String>,
        S: Into<String>,
    {
        self.env.add_template_owned(name.into(), source.into())
    }

    /// Get a reference to a named template
    pub fn get_template(&self, name: &str) -> Result<Template<'_, '_>, Error> {
        self.env.get_template(name)
    }

    /// Define a global value that can be reference by all templates
    pub fn add_global<N, V>(&mut self, name: N, value: V)
    where
        N: Into<String>,
        V: Into<Value>,
    {
        self.env.add_global(name.into(), value)
    }

    pub fn render<CTX>(&self, name: &str, source: &str, context: CTX) -> Result<String, Error>
    where
        CTX: serde::Serialize,
    {
        self.env.render_named_str(name, source, context)
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
