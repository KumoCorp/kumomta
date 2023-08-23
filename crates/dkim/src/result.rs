use crate::canonicalization::Type;
use crate::DKIMError;

#[derive(Clone, Debug, PartialEq)]
pub enum DKIMVerificationStatus {
    Pass {
        header_canon: Type,
        body_canon: Type,
    },
    Neutral,
    Fail(DKIMError),
}

impl DKIMVerificationStatus {
    /// Returns the verification result as a summary: fail, neutral or pass.
    pub fn summary(&self) -> &'static str {
        match self {
            Self::Pass { .. } => "pass",
            Self::Neutral => "neutral",
            Self::Fail(_) => "fail",
        }
    }

    pub fn is_pass(&self) -> bool {
        matches!(self, Self::Pass { .. })
    }

    pub fn is_neutral(&self) -> bool {
        matches!(self, Self::Neutral)
    }

    pub fn is_fail(&self) -> bool {
        matches!(self, Self::Fail(_))
    }

    /// Returns the header canocalization type for passing results
    pub fn header_canonicalization_type(&self) -> Option<Type> {
        match self {
            Self::Pass { header_canon, .. } => Some(*header_canon),
            _ => None,
        }
    }

    /// Returns the body canocalization type
    pub fn body_canonicalization_type(&self) -> Option<Type> {
        match self {
            Self::Pass { body_canon, .. } => Some(*body_canon),
            _ => None,
        }
    }

    /// Similar to `summary` but with detail on fail. Typically used for the
    /// `Authentication-Results` header.
    pub fn detail(&self) -> String {
        match self {
            Self::Pass { .. } | Self::Neutral => self.summary().to_string(),
            Self::Fail(err) => format!("fail ({err:#})"),
        }
    }

    pub fn error(&self) -> Option<DKIMError> {
        match self {
            DKIMVerificationStatus::Fail(err) => Some(err.clone()),
            _ => None,
        }
    }
}

#[derive(Clone, Debug)]
/// Result of the DKIM verification
pub struct DKIMResult {
    status: DKIMVerificationStatus,
    domain_used: String,
}
impl DKIMResult {
    /// Constructs a `pass` result
    pub fn pass(domain_used: &str, header_canon: Type, body_canon: Type) -> Self {
        DKIMResult {
            status: DKIMVerificationStatus::Pass {
                header_canon,
                body_canon,
            },
            domain_used: domain_used.to_lowercase(),
        }
    }
    /// Constructs a `neutral` result
    pub fn neutral(domain_used: &str) -> Self {
        DKIMResult {
            status: DKIMVerificationStatus::Neutral,
            domain_used: domain_used.to_lowercase(),
        }
    }
    /// Constructs a `fail` result with a reason
    pub fn fail(reason: DKIMError, domain_used: &str) -> Self {
        DKIMResult {
            status: DKIMVerificationStatus::Fail(reason),
            domain_used: domain_used.to_lowercase(),
        }
    }

    pub fn status(&self) -> &DKIMVerificationStatus {
        &self.status
    }

    /// Returns the domain used to pass the DKIM verification
    pub fn domain_used(&self) -> &str {
        &self.domain_used
    }
}
