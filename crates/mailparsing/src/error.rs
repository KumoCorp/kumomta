use crate::MessageConformance;
use thiserror::Error;

#[derive(Error, Debug, Clone, PartialEq)]
pub enum MailParsingError {
    #[error("invalid header: {0}")]
    HeaderParse(String),
    #[error("while assigning header '{header_name}': {error}")]
    InvalidHeaderValueDuringAssignment {
        header_name: String,
        error: Box<MailParsingError>,
    },
    #[error("while parsing header '{header_name}': {error}")]
    InvalidHeaderValueDuringGet {
        header_name: String,
        error: Box<MailParsingError>,
    },
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
    #[error("Mime Tree has too many child parts")]
    TooManyParts,
    #[error("8-bit found when 7-bit is required")]
    EightBit,
    #[error("Message has conformance issues: {}", .0.to_string())]
    ConformanceIssues(MessageConformance),
    #[error("Failed to detect the charset: {0}")]
    CharsetDetectionFailed(String),
}
