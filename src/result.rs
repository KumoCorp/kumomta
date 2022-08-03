use crate::{canonicalization, DKIMError};

#[derive(Clone)]
/// Result of the DKIM verification
pub struct DKIMResult {
    value: &'static str,
    error: Option<DKIMError>,
    domain_used: String,
    header_canonicalization_type: Option<canonicalization::Type>,
    body_canonicalization_type: Option<canonicalization::Type>,
}
impl DKIMResult {
    /// Constructs a `pass` result
    pub fn pass(
        domain_used: String,
        header_canonicalization_type: canonicalization::Type,
        body_canonicalization_type: canonicalization::Type,
    ) -> Self {
        DKIMResult {
            value: "pass",
            error: None,
            domain_used,
            header_canonicalization_type: Some(header_canonicalization_type),
            body_canonicalization_type: Some(body_canonicalization_type),
        }
    }
    /// Constructs a `neutral` result
    pub fn neutral(domain_used: String) -> Self {
        DKIMResult {
            value: "neutral",
            error: None,
            domain_used,
            header_canonicalization_type: None,
            body_canonicalization_type: None,
        }
    }
    /// Constructs a `fail` result with a reason
    pub fn fail(reason: DKIMError, domain_used: String) -> Self {
        DKIMResult {
            value: "fail",
            error: Some(reason),
            domain_used,
            header_canonicalization_type: None,
            body_canonicalization_type: None,
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

    /// Returns the header canocalization type
    pub fn header_canonicalization_type(&self) -> Option<canonicalization::Type> {
        self.header_canonicalization_type.clone()
    }

    /// Returns the body canocalization type
    pub fn body_canonicalization_type(&self) -> Option<canonicalization::Type> {
        self.body_canonicalization_type.clone()
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
