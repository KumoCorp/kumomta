use thiserror::Error;

#[derive(Error, Debug)]
pub enum MailParsingError {
    #[error("invalid header: {0}")]
    HeaderParse(String),
}
