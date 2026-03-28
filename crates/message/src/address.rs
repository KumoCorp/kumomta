use anyhow::anyhow;
use bstr::{BStr, BString, ByteSlice};
use config::any_err;
use mailparsing::{Address, AddressList, EncodeHeaderValue, Mailbox};
#[cfg(feature = "impl")]
use mlua::{FromLua, MetaMethod, UserData, UserDataFields, UserDataMethods};
use rfc5321::{EnvelopeAddress as EnvelopeAddress5321, ForwardPath, ReversePath};
use serde::{Deserialize, Serialize};

#[derive(Clone, PartialEq, Serialize, Deserialize, Eq)]
#[serde(transparent)]
pub struct EnvelopeAddress(EnvelopeAddress5321);

impl std::fmt::Debug for EnvelopeAddress {
    fn fmt(&self, fmt: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(fmt, "<{}>", self.0.to_string())
    }
}

#[cfg(feature = "impl")]
impl FromLua for EnvelopeAddress {
    fn from_lua(value: mlua::Value, lua: &mlua::Lua) -> mlua::Result<Self> {
        match value {
            mlua::Value::String(s) => s
                .to_str()?
                .parse::<EnvelopeAddress5321>()
                .map_err(any_err)
                .map(Self),
            _ => {
                let ud = mlua::UserDataRef::<EnvelopeAddress>::from_lua(value, lua)?;
                Ok(ud.clone())
            }
        }
    }
}

impl EnvelopeAddress {
    pub fn parse(text: &str) -> anyhow::Result<Self> {
        let addr = text
            .parse::<EnvelopeAddress5321>()
            .map_err(|err| anyhow!("{err}"))?;
        Ok(Self(addr))
    }

    pub fn user(&self) -> String {
        match &self.0 {
            EnvelopeAddress5321::Postmaster => "postmaster".to_string(),
            EnvelopeAddress5321::Null => "".to_string(),
            EnvelopeAddress5321::Path(path) => path.mailbox.local_part().into(),
        }
    }

    pub fn domain(&self) -> String {
        match &self.0 {
            EnvelopeAddress5321::Postmaster | EnvelopeAddress5321::Null => "".to_string(),
            EnvelopeAddress5321::Path(path) => path.mailbox.domain.to_string(),
        }
    }

    pub fn null_sender() -> Self {
        Self(EnvelopeAddress5321::Null)
    }

    pub fn to_string(&self) -> String {
        self.0.to_string()
    }
}

impl TryInto<EnvelopeAddress> for &Mailbox {
    type Error = anyhow::Error;
    fn try_into(self) -> anyhow::Result<EnvelopeAddress> {
        if self.address.local_part.is_empty() && self.address.domain.is_empty() {
            Ok(EnvelopeAddress::null_sender())
        } else {
            EnvelopeAddress::parse(&format!(
                "{}@{}",
                self.address.local_part, self.address.domain
            ))
        }
    }
}

impl TryInto<EnvelopeAddress> for &Address {
    type Error = anyhow::Error;
    fn try_into(self) -> anyhow::Result<EnvelopeAddress> {
        match self {
            Address::Mailbox(mbox) => mbox.try_into(),
            Address::Group { name: _, entries } => {
                if entries.len() == 1 {
                    (&entries[0]).try_into()
                } else {
                    anyhow::bail!("Cannot convert an Address::Group to an EnvelopeAddress unless it has exactly one entry");
                }
            }
        }
    }
}

impl TryInto<ForwardPath> for EnvelopeAddress {
    type Error = String;
    fn try_into(self) -> Result<ForwardPath, Self::Error> {
        self.0.try_into()
    }
}

impl TryInto<ReversePath> for EnvelopeAddress {
    type Error = String;
    fn try_into(self) -> Result<ReversePath, Self::Error> {
        self.0.try_into()
    }
}

impl From<ForwardPath> for EnvelopeAddress {
    fn from(fp: ForwardPath) -> EnvelopeAddress {
        EnvelopeAddress(fp.into())
    }
}

#[cfg(feature = "impl")]
impl UserData for EnvelopeAddress {
    fn add_fields<F: UserDataFields<Self>>(fields: &mut F) {
        fields.add_field_method_get("user", |_, this| Ok(this.user().to_string()));
        fields.add_field_method_get("domain", |_, this| Ok(this.domain().to_string()));
        fields.add_field_method_get("email", |_, this| Ok(this.0.to_string()));
    }

    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_meta_method(MetaMethod::ToString, |_, this, _: ()| {
            Ok(this.0.to_string())
        });
    }
}

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
    pub fn name(&self) -> anyhow::Result<Option<&BStr>> {
        let addr = self.single_address()?;
        Ok(addr.name.as_ref().map(|b| b.as_bstr()))
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
            .split_once('@')
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
                        Some(name.to_string())
                    },
                    addresses,
                })
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HeaderAddress {
    pub name: Option<BString>,
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
    pub fn name(&self) -> Option<&BStr> {
        self.name.as_ref().map(|b| b.as_bstr())
    }

    pub fn crack_address(&self) -> anyhow::Result<(&str, &str)> {
        let address = self
            .address
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("no address"))?;

        address
            .split_once('@')
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

    #[test]
    fn no_ports_in_domain() {
        k9::snapshot!(
            EnvelopeAddress::parse("user@example.com:2025").unwrap_err(),
            "
 --> 1:17
  |
1 | user@example.com:2025
  |                 ^---
  |
  = expected EOI, alpha, digit, or utf8_non_ascii
"
        );
    }
}
