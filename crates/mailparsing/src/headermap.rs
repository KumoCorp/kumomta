use crate::{Header, MailParsingError, Mailbox, MailboxList, Result};

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

impl<'a> HeaderMap<'a> {
    pub fn new(headers: Vec<Header<'a>>) -> Self {
        Self { headers }
    }

    pub fn get_first(&'a self, name: &str) -> Option<&Header<'a>> {
        self.iter_named(name).next()
    }

    pub fn get_last(&'a self, name: &str) -> Option<&Header<'a>> {
        self.iter_named(name).rev().next()
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

    pub fn from(&self) -> Result<Option<MailboxList>> {
        match self.get_first("From") {
            None => Ok(None),
            Some(header) => Ok(Some(header.as_mailbox_list()?)),
        }
    }

    pub fn resent_from(&self) -> Result<Option<MailboxList>> {
        match self.get_first("Resent-From") {
            None => Ok(None),
            Some(header) => Ok(Some(header.as_mailbox_list()?)),
        }
    }

    pub fn sender(&self) -> Result<Option<Mailbox>> {
        match self.get_first("Sender") {
            None => Ok(None),
            Some(header) => Ok(Some(header.as_mailbox()?)),
        }
    }

    pub fn resent_sender(&self) -> Result<Option<Mailbox>> {
        match self.get_first("Resent-Sender") {
            None => Ok(None),
            Some(header) => Ok(Some(header.as_mailbox()?)),
        }
    }
}
