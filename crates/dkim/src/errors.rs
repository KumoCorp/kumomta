use dns_resolver::DnsError;
use thiserror::Error;

/// DKIM error status
#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum Status {
    Permfail,
    Tempfail,
}

#[derive(Debug, PartialEq, Clone, Error)]
/// DKIM errors
pub enum DKIMError {
    #[error("unsupported hash algorithm: {0}")]
    UnsupportedHashAlgorithm(String),
    #[error("unsupported canonicalization: {0}")]
    UnsupportedCanonicalizationType(String),
    #[error("signature syntax error: {0}")]
    SignatureSyntaxError(String),
    #[error("signature missing required tag ({0})")]
    SignatureMissingRequiredTag(&'static str),
    #[error("incompatible version")]
    IncompatibleVersion,
    #[error("domain mismatch")]
    DomainMismatch,
    #[error("From field not signed")]
    FromFieldNotSigned,
    #[error("signature expired")]
    SignatureExpired,
    #[error("unacceptable signature header")]
    UnacceptableSignatureHeader,
    #[error("unsupported query method")]
    UnsupportedQueryMethod,
    #[error("key unavailable: {0}")]
    KeyUnavailable(String),
    #[error("internal error: {0}")]
    UnknownInternalError(String),
    #[error("no key for signature")]
    NoKeyForSignature,
    #[error("key syntax error")]
    KeySyntaxError,
    #[error("key incompatible version")]
    KeyIncompatibleVersion,
    #[error("inappropriate key algorithm")]
    InappropriateKeyAlgorithm,
    #[error("signature did not verify")]
    SignatureDidNotVerify,
    #[error("body hash did not verify")]
    BodyHashDidNotVerify,
    #[error("malformed email body")]
    MalformedBody,
    #[error("failed sign: {0}")]
    FailedToSign(String),
    #[error("failed to build object: {0}")]
    BuilderError(&'static str),
    #[error("failed to serialize DKIM header: {0}")]
    HeaderSerializeError(String),
    #[error("failed to load private key: {0}")]
    PrivateKeyLoadError(String),
    #[error("failed to parse message: {0:#}")]
    MailParsingError(#[from] mailparsing::MailParsingError),
    #[error("Canonical CRLF line endings are required for correct signing and verification")]
    CanonicalLineEndingsRequired,
    #[error(transparent)]
    Dns(#[from] DnsError),
}

impl DKIMError {
    pub fn status(self) -> Status {
        use DKIMError::*;
        match self {
            SignatureSyntaxError(_)
            | SignatureMissingRequiredTag(_)
            | IncompatibleVersion
            | DomainMismatch
            | FromFieldNotSigned
            | SignatureExpired
            | UnacceptableSignatureHeader
            | UnsupportedQueryMethod
            | NoKeyForSignature
            | KeySyntaxError
            | KeyIncompatibleVersion
            | InappropriateKeyAlgorithm
            | SignatureDidNotVerify
            | BodyHashDidNotVerify
            | MalformedBody
            | CanonicalLineEndingsRequired
            | MailParsingError(_)
            | UnsupportedCanonicalizationType(_)
            | UnsupportedHashAlgorithm(_) => Status::Permfail,
            KeyUnavailable(_)
            | UnknownInternalError(_)
            | BuilderError(_)
            | FailedToSign(_)
            | PrivateKeyLoadError(_)
            | HeaderSerializeError(_) => Status::Tempfail,
            Dns(dns) => match dns {
                DnsError::InvalidName(_) => Status::Permfail,
                DnsError::ResolveFailed(_) => Status::Tempfail,
            },
        }
    }
}
