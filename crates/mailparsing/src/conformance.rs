use serde::Deserialize;

#[derive(Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ConformanceDisposition {
    #[default]
    Deny,
    Allow,
    Fix,
}
