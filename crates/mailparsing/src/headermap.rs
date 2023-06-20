use crate::{AddressList, Header, Mailbox, MailboxList, Result};

/// Represents an ordered list of headers.
/// Note that there may be multiple headers with the same name.
/// Derefs to the underlying `Vec<Header>` for mutation,
/// but provides some accessors for retrieving headers by name.
#[derive(Debug, Clone)]
pub struct HeaderMap<'a> {
    headers: Vec<Header<'a>>,
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

macro_rules! accessor {
    ($func_name:ident, $header_name:literal, $ty:path, $parser:ident) => {
        pub fn $func_name(&self) -> Result<Option<$ty>> {
            match self.get_first($header_name) {
                None => Ok(None),
                Some(header) => Ok(Some(header.$parser()?)),
            }
        }
    };
}

impl<'a> HeaderMap<'a> {
    pub fn new(headers: Vec<Header<'a>>) -> Self {
        Self { headers }
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
    accessor!(cc, "Cc", AddressList, as_address_list);
    accessor!(bcc, "Bcc", AddressList, as_address_list);
    accessor!(resent_to, "Resent-To", AddressList, as_address_list);
    accessor!(resent_cc, "Resent-Cc", AddressList, as_address_list);
    accessor!(resent_bcc, "Resent-Bcc", AddressList, as_address_list);

    accessor!(sender, "Sender", Mailbox, as_mailbox);
    accessor!(resent_sender, "Resent-Sender", Mailbox, as_mailbox);

    accessor!(message_id, "Message-ID", String, as_message_id);
    accessor!(references, "References", Vec<String>, as_message_id_list);

    accessor!(subject, "Subject", String, as_unstructured);
    accessor!(comments, "Comments", String, as_unstructured);
}
