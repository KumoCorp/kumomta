use mlua::{MetaMethod, UserData, UserDataFields, UserDataMethods};
use rfc5321::{ForwardPath, ReversePath};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Eq)]
#[serde(transparent)]
pub struct EnvelopeAddress(String);

impl EnvelopeAddress {
    pub fn parse(text: &str) -> anyhow::Result<Self> {
        if text.is_empty() {
            Ok(Self::null_sender())
        } else {
            let fields: Vec<&str> = text.split('@').collect();
            anyhow::ensure!(fields.len() == 2, "expected user@domain");
            // TODO: stronger validation of local part and domain
            Ok(Self(text.to_string()))
        }
    }

    pub fn user(&self) -> &str {
        match self.0.find('@') {
            Some(at) => &self.0[..at],
            None => "",
        }
    }

    pub fn domain(&self) -> &str {
        match self.0.find('@') {
            Some(at) => &self.0[at + 1..],
            None => "",
        }
    }

    pub fn null_sender() -> Self {
        Self("".to_string())
    }

    pub fn to_string(&self) -> String {
        self.0.to_string()
    }
}

impl TryInto<ForwardPath> for EnvelopeAddress {
    type Error = &'static str;
    fn try_into(self) -> Result<ForwardPath, Self::Error> {
        ForwardPath::try_from(self.0.as_str())
    }
}

impl TryInto<ReversePath> for EnvelopeAddress {
    type Error = &'static str;
    fn try_into(self) -> Result<ReversePath, Self::Error> {
        ReversePath::try_from(self.0.as_str())
    }
}

impl UserData for EnvelopeAddress {
    fn add_fields<'lua, F: UserDataFields<'lua, Self>>(fields: &mut F) {
        fields.add_field_method_get("user", |_, this| Ok(this.user().to_string()));
        fields.add_field_method_get("domain", |_, this| Ok(this.domain().to_string()));
    }

    fn add_methods<'lua, M: UserDataMethods<'lua, Self>>(methods: &mut M) {
        methods.add_meta_method(MetaMethod::ToString, |_, this, _: ()| {
            Ok(this.0.to_string())
        });
    }
}
