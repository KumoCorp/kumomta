use crate::{
    AddressList, AuthenticationResults, Header, Mailbox, MailboxList, MessageID, MimeParameters,
    Result, SharedString,
};
use chrono::{DateTime, FixedOffset, TimeZone};
use paste::paste;

/// Represents an ordered list of headers.
/// Note that there may be multiple headers with the same name.
/// Derefs to the underlying `Vec<Header>` for mutation,
/// but provides some accessors for retrieving headers by name.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct HeaderMap<'a> {
    pub(crate) headers: Vec<Header<'a>>,
}

impl<'a> std::ops::Deref for HeaderMap<'a> {
    type Target = Vec<Header<'a>>;
    fn deref(&self) -> &Vec<Header<'a>> {
        &self.headers
    }
}

impl<'a> std::ops::DerefMut for HeaderMap<'a> {
    fn deref_mut(&mut self) -> &mut Vec<Header<'a>> {
        &mut self.headers
    }
}

pub trait EncodeHeaderValue {
    fn encode_value(&self) -> SharedString<'static>;
    fn as_header(&self, _name: &str) -> Option<Header<'static>> {
        None
    }
}

impl EncodeHeaderValue for &str {
    fn encode_value(&self) -> SharedString<'static> {
        unimplemented!();
    }

    fn as_header(&self, name: &str) -> Option<Header<'static>> {
        Some(Header::new_unstructured(name.to_string(), self.to_string()))
    }
}

impl<T: TimeZone> EncodeHeaderValue for DateTime<T>
where
    <T as TimeZone>::Offset: std::fmt::Display,
{
    fn encode_value(&self) -> SharedString<'static> {
        (*self).to_rfc2822().into()
    }
}

macro_rules! accessor {
    ($func_name:ident, $header_name:literal, $ty:path, $parser:ident) => {
        pub fn $func_name(&self) -> Result<Option<$ty>> {
            match self.get_first($header_name) {
                None => Ok(None),
                Some(header) => Ok(Some(header.$parser()?)),
            }
        }

        paste! {
            pub fn [<set_ $func_name>](&mut self, v: impl EncodeHeaderValue) {

                if let Some(idx) = self
                    .headers
                    .iter()
                    .position(|header| header.get_name().eq_ignore_ascii_case($header_name))
                {
                    if let Some(hdr) = v.as_header(self.headers[idx].get_name()) {
                        self.headers[idx] = hdr;
                    } else {
                        self.headers[idx].assign(v);
                    }
                } else {
                    if let Some(hdr) = v.as_header($header_name) {
                        self.headers.push(hdr);
                    } else {
                    self.headers
                        .push(Header::with_name_value($header_name, v.encode_value()));
                    }
                }
            }
        }
    };
}

impl<'a> HeaderMap<'a> {
    pub fn new(headers: Vec<Header<'a>>) -> Self {
        Self { headers }
    }

    pub fn prepend<V: Into<SharedString<'a>>>(&mut self, name: &str, v: V) {
        self.headers
            .insert(0, Header::new_unstructured(name.to_string(), v));
    }

    pub fn get_first(&'a self, name: &str) -> Option<&Header<'a>> {
        self.iter_named(name).next()
    }

    pub fn get_first_mut(&'a mut self, name: &str) -> Option<&mut Header<'a>> {
        self.iter_named_mut(name).next()
    }

    pub fn get_last(&'a self, name: &str) -> Option<&Header<'a>> {
        self.iter_named(name).rev().next()
    }

    pub fn get_last_mut(&'a mut self, name: &str) -> Option<&mut Header<'a>> {
        self.iter_named_mut(name).rev().next()
    }

    pub fn iter_named<'name>(
        &'a self,
        name: &'name str,
    ) -> impl DoubleEndedIterator<Item = &'a Header<'a>> + 'name
    where
        'a: 'name,
    {
        self.headers
            .iter()
            .filter(|header| header.get_name().eq_ignore_ascii_case(name))
    }

    pub fn iter_named_mut<'name>(
        &'a mut self,
        name: &'name str,
    ) -> impl DoubleEndedIterator<Item = &'a mut Header<'a>> + 'name
    where
        'a: 'name,
    {
        self.headers
            .iter_mut()
            .filter(|header| header.get_name().eq_ignore_ascii_case(name))
    }

    accessor!(from, "From", MailboxList, as_mailbox_list);
    accessor!(resent_from, "Resent-From", MailboxList, as_mailbox_list);

    accessor!(to, "To", AddressList, as_address_list);
    accessor!(reply_to, "Reply-To", AddressList, as_address_list);
    accessor!(cc, "Cc", AddressList, as_address_list);
    accessor!(bcc, "Bcc", AddressList, as_address_list);
    accessor!(resent_to, "Resent-To", AddressList, as_address_list);
    accessor!(resent_cc, "Resent-Cc", AddressList, as_address_list);
    accessor!(resent_bcc, "Resent-Bcc", AddressList, as_address_list);

    accessor!(date, "Date", DateTime<FixedOffset>, as_date);

    accessor!(sender, "Sender", Mailbox, as_mailbox);
    accessor!(resent_sender, "Resent-Sender", Mailbox, as_mailbox);

    accessor!(message_id, "Message-ID", MessageID, as_message_id);
    accessor!(content_id, "Content-ID", MessageID, as_content_id);
    accessor!(references, "References", Vec<MessageID>, as_message_id_list);

    accessor!(subject, "Subject", String, as_unstructured);
    accessor!(comments, "Comments", String, as_unstructured);
    accessor!(
        content_transfer_encoding,
        "Content-Transfer-Encoding",
        MimeParameters,
        as_content_transfer_encoding
    );
    accessor!(mime_version, "Mime-Version", String, as_unstructured);
    accessor!(
        content_disposition,
        "Content-Disposition",
        MimeParameters,
        as_content_disposition
    );

    accessor!(
        content_type,
        "Content-Type",
        MimeParameters,
        as_content_type
    );

    accessor!(
        authentication_results,
        "Authentication-Results",
        AuthenticationResults,
        as_authentication_results
    );
}
