mod builder;
mod conformance;
mod error;
mod header;
mod headermap;
mod mimepart;
mod nom_utils;
mod normalize;
mod rfc5322_parser;
mod strings;

pub use error::MailParsingError;
pub type Result<T> = std::result::Result<T, MailParsingError>;

pub use builder::*;
pub use conformance::*;
pub use header::{Header, HeaderParseResult, MessageConformance};
pub use headermap::*;
pub use mimepart::*;
pub use normalize::*;
pub use rfc5322_parser::*;
pub use strings::SharedString;
