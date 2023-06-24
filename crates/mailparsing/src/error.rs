use thiserror::Error;

#[derive(Error, Debug, PartialEq)]
pub enum MailParsingError {
    #[error("invalid header: {0}")]
    HeaderParse(String),
    #[error("Header {0} not found in mime part")]
    MissingHeader(String),
}
