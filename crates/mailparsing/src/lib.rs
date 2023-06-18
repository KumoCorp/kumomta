mod error;
mod header;
mod headermap;
mod mimepart;
mod strings;

pub use error::MailParsingError;
pub type Result<T> = std::result::Result<T, MailParsingError>;

pub use header::Header;
pub use headermap::HeaderMap;
pub use mimepart::MimePart;
pub use strings::SharedString;
