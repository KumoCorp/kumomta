use crate::DKIMError;

#[derive(Clone)]
/// Result of the DKIM verification
pub struct DKIMResult {
    value: &'static str,
    error: Option<DKIMError>,
    domain_used: String,
}
impl DKIMResult {
    /// Constructs a `pass` result
    pub fn pass(domain_used: String) -> Self {
        DKIMResult {
            value: "pass",
            error: None,
            domain_used,
        }
    }
    /// Constructs a `neutral` result
    pub fn neutral(domain_used: String) -> Self {
        DKIMResult {
            value: "neutral",
            error: None,
            domain_used,
        }
    }
    /// Constructs a `fail` result with a reason
    pub fn fail(reason: DKIMError, domain_used: String) -> Self {
        DKIMResult {
            value: "fail",
            error: Some(reason),
            domain_used,
        }
    }

    pub fn error(&self) -> Option<DKIMError> {
        self.error.clone()
    }

    /// Returns the domain used to pass the DKIM verification
    pub fn domain_used(&self) -> String {
        self.domain_used.to_lowercase()
    }

    /// Returns the verification result as a summary: fail, neutral or pass.
    pub fn summary(&self) -> &'static str {
        self.value
    }

    /// Similar to `summary` but with detail on fail. Typically used for the
    /// `Authentication-Results` header.
    pub fn with_detail(&self) -> String {
        if let Some(err) = self.error() {
            format!("{} ({})", self.value, err)
        } else {
            self.value.to_owned()
        }
    }
}
