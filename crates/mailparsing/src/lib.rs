mod builder;
mod charset;
mod conformance;
mod datetime;
mod error;
mod header;
mod headermap;
mod mimepart;
mod normalize;
mod rfc5322_parser;
mod strings;
mod zone_offsets;

pub use error::MailParsingError;
pub type Result<T> = std::result::Result<T, MailParsingError>;

pub use builder::*;
pub use charset::{resolve_charset, CHARSET_ALIASES};
pub use conformance::*;
pub use datetime::{format_rfc2822_date, parse_rfc2822_date};
pub use header::{Header, HeaderParseResult, MessageConformance};
pub use headermap::*;
pub use mimepart::*;
pub use normalize::*;
pub use rfc5322_parser::*;
pub use strings::SharedString;
