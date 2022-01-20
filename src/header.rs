use crate::parser;
use std::collections::HashMap;

pub(crate) const HEADER: &str = "DKIM-Signature";
pub(crate) const REQUIRED_TAGS: &[&str] = &["v", "a", "b", "bh", "d", "h", "s"];

#[derive(Debug)]
pub struct DKIMHeader<'a> {
    pub(crate) tags: HashMap<String, parser::Tag>,
    pub(crate) raw_bytes: &'a str,
}

impl<'a> DKIMHeader<'a> {
    pub(crate) fn get_tag(&self, name: &str) -> Option<String> {
        self.tags.get(name).map(|v| v.value.clone())
    }

    pub(crate) fn get_raw_tag(&self, name: &str) -> Option<String> {
        self.tags.get(name).map(|v| v.raw_value.clone())
    }

    pub(crate) fn get_required_tag(&self, name: &str) -> String {
        // Required tags are guaranteed by the parser to be present so it's safe
        // to assert and unwrap.
        debug_assert!(REQUIRED_TAGS.contains(&name));
        self.tags.get(name).unwrap().value.clone()
    }
}
