use serde::Deserialize;

#[derive(Deserialize, Clone, Copy, Debug, Default)]
pub enum ConformanceDisposition {
    #[default]
    Deny,
    Allow,
    Fix,
}
