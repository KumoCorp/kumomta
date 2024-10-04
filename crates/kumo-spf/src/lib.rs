pub mod dns;
pub mod eval;
pub use eval::EvalContext;
pub mod record;
use record::Qualifier;
#[cfg(test)]
mod tests;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpfDisposition {
    /// A result of "none" means either (a) no syntactically valid DNS domain
    /// name was extracted from the SMTP session that could be used as the
    /// one to be authorized, or (b) no SPF records were retrieved from
    /// the DNS.
    None,

    /// A "neutral" result means the ADMD has explicitly stated that it is
    /// not asserting whether the IP address is authorized.
    Neutral,

    /// A "pass" result is an explicit statement that the client is
    /// authorized to inject mail with the given identity.
    Pass,

    /// A "fail" result is an explicit statement that the client is not
    /// authorized to use the domain in the given identity.
    Fail,

    /// A "softfail" result is a weak statement by the publishing ADMD that
    /// the host is probably not authorized.  It has not published a
    /// stronger, more definitive policy that results in a "fail".
    SoftFail,

    /// A "temperror" result means the SPF verifier encountered a transient
    /// (generally DNS) error while performing the check.  A later retry may
    /// succeed without further DNS operator action.
    TempError,

    /// A "permerror" result means the domain's published records could not
    /// be correctly interpreted.  This signals an error condition that
    /// definitely requires DNS operator intervention to be resolved.
    PermError,
}

impl From<Qualifier> for SpfDisposition {
    fn from(qualifier: Qualifier) -> Self {
        match qualifier {
            Qualifier::Pass => Self::Pass,
            Qualifier::Fail => Self::Fail,
            Qualifier::SoftFail => Self::SoftFail,
            Qualifier::Neutral => Self::Neutral,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpfResult {
    pub disposition: SpfDisposition,
    pub context: String,
}
