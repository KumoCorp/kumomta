use thiserror::Error;

#[derive(Error, Debug, PartialEq)]
pub enum MailParsingError {
    #[error("invalid header: {0}")]
    HeaderParse(String),
    #[error("Header {0} not found in mime part")]
    MissingHeader(String),
    #[error("Unknown Mime-Version: {0}")]
    UnknownMimeVersion(String),
    #[error("Invalid Content-Transfer-Encoding: {0}")]
    InvalidContentTransferEncoding(String),
    #[error("parsing body: {0}")]
    BodyParse(String),
}
