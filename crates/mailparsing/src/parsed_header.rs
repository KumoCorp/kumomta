use crate::headermap::EncodeHeaderValue;
use crate::rfc5322_parser::{qp_encode, Parser};
use crate::{
    AddressList, AuthenticationResults, MailParsingError, Mailbox, MailboxList, MessageID,
    MimeParameters, Result, SharedString,
};
use bstr::BString;
use chrono::{DateTime, FixedOffset};

/// The parsed, typed value of a header, as distinct from the raw wire form
/// held by `Header`. A header's name determines the grammar its bytes are
/// parsed with (see `ParsedHeader::structured`), so callers reach a value
/// of the correct type without choosing a parser themselves.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ParsedHeader {
    MailboxList(MailboxList),
    Mailbox(Mailbox),
    AddressList(AddressList),
    MessageId(MessageID),
    MessageIdList(Vec<MessageID>),
    MimeParameters(MimeParameters),
    Date(DateTime<FixedOffset>),
    AuthenticationResults(AuthenticationResults),
    Unstructured(BString),
}

impl ParsedHeader {
    /// Parse `value` as the header named by `name`, selecting the grammar
    /// from the name. An unrecognized name is treated as unstructured.
    pub fn structured(name: &[u8], value: &[u8]) -> Result<Self> {
        Ok(match grammar_for_name(name) {
            Grammar::MailboxList => Self::MailboxList(Parser::parse_mailbox_list_header(value)?),
            Grammar::Mailbox => Self::Mailbox(Parser::parse_mailbox_header(value)?),
            Grammar::AddressList => Self::AddressList(Parser::parse_address_list_header(value)?),
            Grammar::MessageId => Self::MessageId(Parser::parse_msg_id_header(value)?),
            Grammar::ContentId => Self::MessageId(Parser::parse_content_id_header(value)?),
            Grammar::MessageIdList => Self::MessageIdList(Parser::parse_msg_id_header_list(value)?),
            Grammar::ContentType => Self::MimeParameters(Parser::parse_content_type_header(value)?),
            // No dedicated Content-Disposition grammar exists. This reuses the
            // CTE parameter grammar, kept unchanged from the old rebuild()
            // dispatch.
            Grammar::ContentTransferEncoding | Grammar::ContentDisposition => {
                Self::MimeParameters(Parser::parse_content_transfer_encoding_header(value)?)
            }
            Grammar::Date => {
                let value = std::str::from_utf8(value).map_err(|_| MailParsingError::EightBit)?;
                Self::Date(crate::parse_rfc2822_date(value).map_err(MailParsingError::ChronoError)?)
            }
            Grammar::AuthenticationResults => {
                Self::AuthenticationResults(Parser::parse_authentication_results_header(value)?)
            }
            Grammar::Unstructured => Self::Unstructured(Parser::parse_unstructured_header(value)?),
        })
    }
}

impl EncodeHeaderValue for ParsedHeader {
    fn encode_value(&self) -> SharedString<'static> {
        match self {
            Self::MailboxList(v) => v.encode_value(),
            Self::Mailbox(v) => v.encode_value(),
            Self::AddressList(v) => v.encode_value(),
            Self::MessageId(v) => v.encode_value(),
            Self::MessageIdList(v) => v.encode_value(),
            Self::MimeParameters(v) => v.encode_value(),
            Self::Date(v) => v.encode_value(),
            Self::AuthenticationResults(v) => v.encode_value(),
            Self::Unstructured(v) => encode_unstructured_value(v.as_slice()),
        }
    }
}

/// Encode a free-text header value: fold a plain-ASCII value at whitespace,
/// or wrap a value with non-ASCII bytes in an RFC 2047 encoded-word.
/// `Header::new_unstructured` and `ParsedHeader`'s unstructured arm share
/// this so a value encodes the same either way.
pub(crate) fn encode_unstructured_value(value: &[u8]) -> SharedString<'static> {
    match std::str::from_utf8(value) {
        Ok(value) if value.is_ascii() => kumo_wrap::wrap_bytes(value).into(),
        Ok(value) => qp_encode(value.as_bytes()).into(),
        Err(_) => kumo_wrap::wrap_bytes(value).into(),
    }
}

/// The grammar a header's value is parsed with. Several names share a
/// grammar, and several grammars yield the same `ParsedHeader` value type,
/// so this stays private: callers key off the header name, never this tag.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Grammar {
    MailboxList,
    Mailbox,
    AddressList,
    MessageId,
    ContentId,
    MessageIdList,
    ContentType,
    ContentTransferEncoding,
    ContentDisposition,
    Date,
    AuthenticationResults,
    Unstructured,
}

/// The header names with a defined grammar or canonical spelling. This is
/// the single source of truth behind `grammar_for_name`,
/// `canonical_header_name`, and `is_address_header_name`. The unstructured
/// entries earn their place by fixing the canonical casing of headers whose
/// value is otherwise free text.
const KNOWN_HEADERS: &[(&str, Grammar)] = &[
    ("From", Grammar::MailboxList),
    ("Resent-From", Grammar::MailboxList),
    ("Sender", Grammar::Mailbox),
    ("Resent-Sender", Grammar::Mailbox),
    ("Reply-To", Grammar::AddressList),
    ("To", Grammar::AddressList),
    ("Cc", Grammar::AddressList),
    ("Bcc", Grammar::AddressList),
    ("Resent-To", Grammar::AddressList),
    ("Resent-Cc", Grammar::AddressList),
    ("Resent-Bcc", Grammar::AddressList),
    ("Date", Grammar::Date),
    ("Message-ID", Grammar::MessageId),
    ("Content-ID", Grammar::ContentId),
    ("References", Grammar::MessageIdList),
    ("Content-Type", Grammar::ContentType),
    (
        "Content-Transfer-Encoding",
        Grammar::ContentTransferEncoding,
    ),
    ("Content-Disposition", Grammar::ContentDisposition),
    ("Authentication-Results", Grammar::AuthenticationResults),
    ("Subject", Grammar::Unstructured),
    ("Comments", Grammar::Unstructured),
    ("Mime-Version", Grammar::Unstructured),
];

fn grammar_for_name(name: &[u8]) -> Grammar {
    KNOWN_HEADERS
        .iter()
        .find(|(known, _)| known.as_bytes().eq_ignore_ascii_case(name))
        .map(|(_, grammar)| *grammar)
        .unwrap_or(Grammar::Unstructured)
}

/// The canonical spelling of `name`, or None when it isn't a known header.
pub fn canonical_header_name(name: &[u8]) -> Option<&'static str> {
    KNOWN_HEADERS
        .iter()
        .find(|(known, _)| known.as_bytes().eq_ignore_ascii_case(name))
        .map(|(known, _)| *known)
}

/// True when `name` names a header whose value is one or more addresses
/// (a mailbox, mailbox list, or address list).
pub fn is_address_header_name(name: &[u8]) -> bool {
    matches!(
        grammar_for_name(name),
        Grammar::MailboxList | Grammar::Mailbox | Grammar::AddressList
    )
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn address_header_names() {
        // Matched case-insensitively; From/To/Bcc are address headers,
        // Subject/Date/unknown are not.
        assert!(is_address_header_name(b"from"));
        assert!(is_address_header_name(b"To"));
        assert!(is_address_header_name(b"BCC"));
        assert!(is_address_header_name(b"Sender"));
        assert!(!is_address_header_name(b"Subject"));
        assert!(!is_address_header_name(b"Date"));
        assert!(!is_address_header_name(b"X-Custom"));
    }

    #[test]
    fn canonical_names() {
        k9::assert_equal!(canonical_header_name(b"message-id"), Some("Message-ID"));
        k9::assert_equal!(canonical_header_name(b"REPLY-TO"), Some("Reply-To"));
        k9::assert_equal!(canonical_header_name(b"X-Custom"), None);
    }

    #[test]
    fn structured_dispatch() {
        // The name selects the grammar: To parses as an address list, an
        // unknown name is unstructured.
        let to = ParsedHeader::structured(b"To", b"a@example.com, b@example.com").unwrap();
        match to {
            ParsedHeader::AddressList(list) => {
                k9::assert_equal!(list.len(), 2);
            }
            other => panic!("expected AddressList, got {other:?}"),
        }

        let custom = ParsedHeader::structured(b"X-Custom", b"anything at all").unwrap();
        assert!(matches!(custom, ParsedHeader::Unstructured(_)));
    }
}
