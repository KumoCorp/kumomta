use config::any_err;
use mailparsing::{AddrSpec, Address, AddressList, EncodeHeaderValue, Mailbox};
#[cfg(feature = "impl")]
use mlua::{MetaMethod, UserData, UserDataFields, UserDataMethods};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HeaderAddressList(Vec<HeaderAddressEntry>);

impl HeaderAddressList {
    /// If the address list is comprised of a single entry,
    /// returns just the email domain from that entry
    pub fn domain(&self) -> anyhow::Result<&str> {
        let addr = self.single_address()?;
        addr.domain()
    }

    /// If the address list is comprised of a single entry,
    /// returns just the email domain from that entry
    pub fn user(&self) -> anyhow::Result<&str> {
        let addr = self.single_address()?;
        addr.user()
    }

    /// If the address list is comprised of a single entry,
    /// returns just the display name portion, if any
    pub fn name(&self) -> anyhow::Result<Option<&str>> {
        let addr = self.single_address()?;
        Ok(addr.name.as_deref())
    }

    pub fn email(&self) -> anyhow::Result<Option<String>> {
        let addr = self.single_address()?;
        Ok(addr.email())
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
            Ok(this.email().map_err(any_err)?)
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

/// Wire format for JSON serialization of HeaderAddress.
/// This preserves the original `{name, address}` JSON shape for
/// backwards compatibility with existing consumers.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct HeaderAddressWire {
    pub name: Option<String>,
    pub address: Option<String>,
}

impl From<HeaderAddress> for HeaderAddressWire {
    fn from(addr: HeaderAddress) -> Self {
        let address = addr.email();
        Self {
            name: addr.name,
            address,
        }
    }
}

impl TryFrom<HeaderAddressWire> for HeaderAddress {
    type Error = anyhow::Error;

    fn try_from(wire: HeaderAddressWire) -> anyhow::Result<Self> {
        let (user, domain) = match &wire.address {
            Some(email) => {
                let parsed = AddrSpec::parse(email)?;
                (Some(parsed.local_part), Some(parsed.domain))
            }
            None => (None, None),
        };
        Ok(Self {
            name: wire.name,
            user,
            domain,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "HeaderAddressWire", into = "HeaderAddressWire")]
pub struct HeaderAddress {
    pub name: Option<String>,
    pub user: Option<String>,
    pub domain: Option<String>,
}

impl From<&Mailbox> for HeaderAddress {
    fn from(mbox: &Mailbox) -> HeaderAddress {
        Self {
            name: mbox.name.clone(),
            user: Some(mbox.address.local_part.clone()),
            domain: Some(mbox.address.domain.clone()),
        }
    }
}

impl HeaderAddress {
    pub fn user(&self) -> anyhow::Result<&str> {
        self.user
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("no address"))
    }
    pub fn domain(&self) -> anyhow::Result<&str> {
        self.domain
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("no address"))
    }
    pub fn email(&self) -> Option<String> {
        match (&self.user, &self.domain) {
            (Some(user), Some(domain)) => {
                Some(AddrSpec::new(user, domain).encode_value().to_string())
            }
            _ => None,
        }
    }
    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
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
        fields.add_field_method_get("email", |_, this| Ok(this.email()));
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

        // user() now returns the decoded local part (without quotes)
        let user = header_addr_list.user().expect("failed to get user");
        k9::assert_equal!(user, "info@");
        let domain = header_addr_list.domain().expect("failed to get domain");
        k9::assert_equal!(domain, "example.com");

        let addr = header_addr_list
            .single_address()
            .expect("expected single address");

        // HeaderAddress stores the decoded local part
        k9::assert_equal!(addr.user().unwrap(), "info@");
        k9::assert_equal!(addr.domain().unwrap(), "example.com");
        // email() re-encodes with quoting as needed
        k9::assert_equal!(addr.email().unwrap(), "\"info@\"@example.com");
    }

    /// Test JSON serialization roundtrip preserves the {name, address} shape
    #[test]
    fn header_address_serde_roundtrip() {
        let addr = HeaderAddress {
            name: Some("Test User".into()),
            user: Some("test".into()),
            domain: Some("example.com".into()),
        };

        let json = serde_json::to_string(&addr).unwrap();
        k9::assert_equal!(
            json,
            r#"{"name":"Test User","address":"test@example.com"}"#
        );

        let roundtripped: HeaderAddress = serde_json::from_str(&json).unwrap();
        k9::assert_equal!(roundtripped, addr);
    }

    /// Test JSON roundtrip with a quoted local part
    #[test]
    fn header_address_serde_roundtrip_quoted() {
        let addr = HeaderAddress {
            name: None,
            user: Some("info@".into()),
            domain: Some("example.com".into()),
        };

        let json = serde_json::to_string(&addr).unwrap();
        k9::assert_equal!(
            json,
            r#"{"name":null,"address":"\"info@\"@example.com"}"#
        );

        let roundtripped: HeaderAddress = serde_json::from_str(&json).unwrap();
        k9::assert_equal!(roundtripped, addr);
    }
}
