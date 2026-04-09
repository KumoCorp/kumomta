use config::any_err;
use mailparsing::{Address, AddressList, EncodeHeaderValue, Mailbox};
#[cfg(feature = "impl")]
use mlua::{MetaMethod, UserData, UserDataFields, UserDataMethods};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HeaderAddressList(Vec<HeaderAddressEntry>);

impl HeaderAddressList {
    /// If the address list is comprised of a single entry,
    /// returns just the email domain from that entry
    pub fn domain(&self) -> anyhow::Result<&str> {
        let (_local, domain) = self.single_address_cracked()?;
        Ok(domain)
    }

    /// If the address list is comprised of a single entry,
    /// returns just the email domain from that entry
    pub fn user(&self) -> anyhow::Result<&str> {
        let (user, _domain) = self.single_address_cracked()?;
        Ok(user)
    }

    /// If the address list is comprised of a single entry,
    /// returns just the display name portion, if any
    pub fn name(&self) -> anyhow::Result<Option<&str>> {
        let addr = self.single_address()?;
        Ok(addr.name.as_deref())
    }

    pub fn email(&self) -> anyhow::Result<Option<&str>> {
        let addr = self.single_address()?;
        Ok(addr.address.as_deref())
    }

    /// Flattens the groups and list and returns a simple list
    /// of addresses
    pub fn flatten(&self) -> Vec<&HeaderAddress> {
        let mut res = vec![];
        for entry in &self.0 {
            match entry {
                HeaderAddressEntry::Address(a) => res.push(a),
                HeaderAddressEntry::Group(group) => {
                    for addr in &group.addresses {
                        res.push(addr);
                    }
                }
            }
        }
        res
    }

    pub fn single_address_cracked(&self) -> anyhow::Result<(&str, &str)> {
        let addr = self.single_address_string()?;
        let tuple = addr
            .rsplit_once('@')
            .ok_or_else(|| anyhow::anyhow!("no @ in address"))?;
        Ok(tuple)
    }

    pub fn single_address_string(&self) -> anyhow::Result<&str> {
        let addr = self.single_address()?;
        match &addr.address {
            None => anyhow::bail!("no address"),
            Some(addr) => Ok(addr),
        }
    }

    pub fn single_address(&self) -> anyhow::Result<&HeaderAddress> {
        match self.0.len() {
            0 => anyhow::bail!("no addresses"),
            1 => match &self.0[0] {
                HeaderAddressEntry::Address(a) => Ok(a),
                _ => anyhow::bail!("is not a simple address"),
            },
            _ => anyhow::bail!("is not a simple single address"),
        }
    }
}

impl From<AddressList> for HeaderAddressList {
    fn from(input: AddressList) -> HeaderAddressList {
        let addresses: Vec<HeaderAddressEntry> = input.0.iter().map(Into::into).collect();
        HeaderAddressList(addresses)
    }
}

#[cfg(feature = "impl")]
impl UserData for HeaderAddressList {
    fn add_fields<F: UserDataFields<Self>>(fields: &mut F) {
        fields.add_field_method_get("user", |_, this| {
            Ok(this.user().map_err(any_err)?.to_string())
        });
        fields.add_field_method_get("domain", |_, this| {
            Ok(this.domain().map_err(any_err)?.to_string())
        });
        fields.add_field_method_get("email", |_, this| {
            Ok(this.email().map_err(any_err)?.map(|s| s.to_string()))
        });
        fields.add_field_method_get("name", |_, this| {
            Ok(this.name().map_err(any_err)?.map(|s| s.to_string()))
        });
        fields.add_field_method_get("list", |_, this| {
            Ok(this
                .flatten()
                .into_iter()
                .cloned()
                .collect::<Vec<HeaderAddress>>())
        });
    }

    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_meta_method(MetaMethod::ToString, |_, this, _: ()| {
            let json = serde_json::to_string(&this.0).map_err(any_err)?;
            Ok(json)
        });
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum HeaderAddressEntry {
    Address(HeaderAddress),
    Group(AddressGroup),
}

impl From<&Address> for HeaderAddressEntry {
    fn from(addr: &Address) -> HeaderAddressEntry {
        match addr {
            Address::Mailbox(mbox) => HeaderAddressEntry::Address(mbox.into()),
            Address::Group { name, entries } => {
                let addresses = entries.0.iter().map(Into::into).collect();
                HeaderAddressEntry::Group(AddressGroup {
                    name: if name.is_empty() {
                        None
                    } else {
                        Some(name.clone())
                    },
                    addresses,
                })
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HeaderAddress {
    pub name: Option<String>,
    pub address: Option<String>,
}

impl From<&Mailbox> for HeaderAddress {
    fn from(mbox: &Mailbox) -> HeaderAddress {
        Self {
            name: mbox.name.clone(),
            address: Some(mbox.address.encode_value().to_string()),
        }
    }
}

impl HeaderAddress {
    pub fn user(&self) -> anyhow::Result<&str> {
        let (user, _domain) = self.crack_address().map_err(any_err)?;
        Ok(user)
    }
    pub fn domain(&self) -> anyhow::Result<&str> {
        let (_user, domain) = self.crack_address().map_err(any_err)?;
        Ok(domain)
    }
    pub fn email(&self) -> Option<&str> {
        self.address.as_deref()
    }
    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    pub fn crack_address(&self) -> anyhow::Result<(&str, &str)> {
        let address = self
            .address
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("no address"))?;

        address
            .rsplit_once('@')
            .ok_or_else(|| anyhow::anyhow!("no @ in address"))
    }
}

#[cfg(feature = "impl")]
impl UserData for HeaderAddress {
    fn add_fields<F: UserDataFields<Self>>(fields: &mut F) {
        fields.add_field_method_get("user", |_, this| {
            Ok(this.user().map_err(any_err)?.to_string())
        });
        fields.add_field_method_get("domain", |_, this| {
            Ok(this.domain().map_err(any_err)?.to_string())
        });
        fields.add_field_method_get("email", |_, this| Ok(this.email().map(|s| s.to_string())));
        fields.add_field_method_get("name", |_, this| Ok(this.name().map(|s| s.to_string())));
    }
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_meta_method(MetaMethod::ToString, |_, this, _: ()| {
            let json = serde_json::to_string(&this).map_err(any_err)?;
            Ok(json)
        });
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AddressGroup {
    pub name: Option<String>,
    pub addresses: Vec<HeaderAddress>,
}

#[cfg(test)]
mod test {
    use super::*;
    use mailparsing::Parser;

    /// Test the unusual "info@"@example.com form via HeaderAddressList
    #[test]
    fn header_address_list_with_at_in_local_part() {
        // Parse the header value that contains the unusual email address
        let header_value = "\"info@\"@example.com";
        let addr_list = Parser::parse_address_list_header(header_value.as_bytes())
            .expect("failed to parse address list");

        // Convert to HeaderAddressList
        let header_addr_list: HeaderAddressList = addr_list.into();

        // Test single_address_cracked() which uses rsplit_once('@')
        let (user, domain) = header_addr_list
            .single_address_cracked()
            .expect("failed to crack address");
        k9::assert_equal!(user, "\"info@\"");
        k9::assert_equal!(domain, "example.com");

        let addr = header_addr_list
            .single_address()
            .expect("expected single address");

        // Now test crack_address() which uses rsplit_once('@')
        let (user, domain) = addr.crack_address().unwrap();
        k9::assert_equal!(user, "\"info@\"");
        k9::assert_equal!(domain, "example.com");
    }
}
