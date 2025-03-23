use serde::{Deserialize, Serialize};

#[derive(Deserialize, Serialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ConformanceDisposition {
    #[default]
    Deny,
    Allow,
    Fix,
}
