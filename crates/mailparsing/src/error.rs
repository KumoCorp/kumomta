use thiserror::Error;

#[derive(Error, Debug, Clone, PartialEq)]
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
    #[error("Unexpected MimePart structure during write_message: {0}")]
    WriteMessageWtf(&'static str),
    #[error("IO error during write_message")]
    WriteMessageIOError,
    #[error("Error building message: {0}")]
    BuildError(&'static str),
    #[error("Error parsing Date header: {0}")]
    ChronoError(chrono::format::ParseError),
}
