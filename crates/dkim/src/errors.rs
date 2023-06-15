/// DKIM error status
#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum Status {
    Permfail,
    Tempfail,
}

quick_error! {
    #[derive(Debug, PartialEq, Clone)]
    /// DKIM errors
    pub enum DKIMError {
        UnsupportedHashAlgorithm(value: String) {
            display("unsupported hash algorithm: {}", value)
        }
        UnsupportedCanonicalizationType(value: String) {
            display("unsupported canonicalization: {}", value)
        }
        SignatureSyntaxError(err: String) {
            display("signature syntax error: {}", err)
        }
        SignatureMissingRequiredTag(name: &'static str) {
            display("signature missing required tag ({})", name)
        }
        IncompatibleVersion {
            display("incompatible version")
        }
        DomainMismatch {
            display("domain mismatch")
        }
        FromFieldNotSigned {
            display("From field not signed")
        }
        SignatureExpired {
            display("signature expired")
        }
        UnacceptableSignatureHeader {
            display("unacceptable signature header")
        }
        UnsupportedQueryMethod {
            display("unsupported query method")
        }
        KeyUnavailable(err: String) {
            display("key unavailable: {}", err)
        }
        UnknownInternalError(err: String) {
            display("internal error: {}", err)
        }
        NoKeyForSignature {
            display("no key for signature")
        }
        KeySyntaxError {
            display("key syntax error")
        }
        KeyIncompatibleVersion {
            display("key incompatible version")
        }
        InappropriateKeyAlgorithm {
            display("inappropriate key algorithm")
        }
        SignatureDidNotVerify {
            display("signature did not verify")
        }
        BodyHashDidNotVerify {
            display("body hash did not verify")
        }
        MalformedBody {
            display("malformed email body")
        }
        FailedToSign(err: String) {
            display("failed sign: {}", err)
        }
        BuilderError(err: &'static str) {
            display("failed to build object: {}", err)
        }
        HeaderSerializeError(err: String) {
            display("failed to serialize DKIM header: {err}")
        }
    }
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
            | UnsupportedCanonicalizationType(_)
            | UnsupportedHashAlgorithm(_) => Status::Permfail,
            KeyUnavailable(_)
            | UnknownInternalError(_)
            | BuilderError(_)
            | FailedToSign(_)
            | HeaderSerializeError(_) => Status::Tempfail,
        }
    }
}
