mod builder;
mod error;
mod header;
mod headermap;
mod mimepart;
mod nom_utils;
mod rfc5322_parser;
mod strings;

pub use error::MailParsingError;
pub type Result<T> = std::result::Result<T, MailParsingError>;

pub use builder::*;
pub use header::{Header, HeaderConformance, HeaderParseResult};
pub use headermap::*;
pub use mimepart::*;
pub use rfc5322_parser::*;
pub use strings::SharedString;
