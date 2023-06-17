mod error;
mod header;
mod mimepart;
mod strings;

pub use error::MailParsingError;
pub type Result<T> = std::result::Result<T, MailParsingError>;

pub use header::Header;
pub use mimepart::MimePart;
pub use strings::SharedString;
