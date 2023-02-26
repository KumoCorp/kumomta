use crate::dkim::Signer;
use crate::EnvelopeAddress;
use chrono::{DateTime, Utc};
use futures::FutureExt;
use mail_auth::common::headers::HeaderWriter;
use mailparse::{MailHeader, MailHeaderMap};
use mlua::{LuaSerdeExt, UserData, UserDataMethods};
use prometheus::IntGauge;
use serde::{Deserialize, Serialize};
use spool::{Spool, SpoolId};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use timeq::{CancellableTimerEntry, TimerEntryWithDelay};

bitflags::bitflags! {
    struct MessageFlags: u8 {
        /// true if Metadata needs to be saved
        const META_DIRTY = 1;
        /// true if Data needs to be saved
        const DATA_DIRTY = 2;
    }
}

lazy_static::lazy_static! {
    static ref MESSAGE_COUNT: IntGauge = prometheus::register_int_gauge!(
        "message_count",
        "total number of Message objects"
    ).unwrap();
    static ref META_COUNT: IntGauge = prometheus::register_int_gauge!(
        "message_meta_resident_count",
        "total number of Message objects with metadata loaded"
    ).unwrap();
    static ref DATA_COUNT: IntGauge = prometheus::register_int_gauge!(
        "message_data_resident_count",
        "total number of Message objects with body data loaded"
    ).unwrap();
}

#[derive(Debug)]
struct MessageInner {
    metadata: Option<MetaData>,
    data: Arc<Box<[u8]>>,
    flags: MessageFlags,
    num_attempts: u16,
    due: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug)]
pub struct Message {
    id: SpoolId,
    inner: Arc<Mutex<MessageInner>>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct MetaData {
    sender: EnvelopeAddress,
    recipient: EnvelopeAddress,
    meta: serde_json::Value,
}

impl Drop for MessageInner {
    fn drop(&mut self) {
        if self.metadata.is_some() {
            META_COUNT.dec();
        }
        if !self.data.is_empty() {
            DATA_COUNT.dec();
        }
        MESSAGE_COUNT.dec();
    }
}

impl Message {
    /// Create a new message with the supplied data.
    /// The message meta and data are marked as dirty
    pub fn new_dirty(
        id: SpoolId,
        sender: EnvelopeAddress,
        recipient: EnvelopeAddress,
        meta: serde_json::Value,
        data: Arc<Box<[u8]>>,
    ) -> anyhow::Result<Self> {
        anyhow::ensure!(meta.is_object(), "metadata must be a json object");
        MESSAGE_COUNT.inc();
        DATA_COUNT.inc();
        META_COUNT.inc();
        Ok(Self {
            id,
            inner: Arc::new(Mutex::new(MessageInner {
                metadata: Some(MetaData {
                    sender,
                    recipient,
                    meta,
                }),
                data,
                flags: MessageFlags::META_DIRTY | MessageFlags::DATA_DIRTY,
                num_attempts: 0,
                due: None,
            })),
        })
    }

    /// Helper for creating a message based on spool enumeration.
    /// Given a spool id and the serialized metadata blob, returns
    /// a message holding the deserialized version of that metadata.
    pub fn new_from_spool(id: SpoolId, metadata: Vec<u8>) -> anyhow::Result<Self> {
        let metadata: MetaData = serde_json::from_slice(&metadata)?;
        MESSAGE_COUNT.inc();
        META_COUNT.inc();

        Ok(Self {
            id,
            inner: Arc::new(Mutex::new(MessageInner {
                metadata: Some(metadata),
                data: Arc::new(vec![].into_boxed_slice()),
                flags: MessageFlags::empty(),
                num_attempts: 0,
                due: None,
            })),
        })
    }

    pub fn get_num_attempts(&self) -> u16 {
        let inner = self.inner.lock().unwrap();
        inner.num_attempts
    }

    pub fn set_num_attempts(&self, num_attempts: u16) {
        let mut inner = self.inner.lock().unwrap();
        inner.num_attempts = num_attempts;
    }

    pub fn increment_num_attempts(&self) {
        let mut inner = self.inner.lock().unwrap();
        inner.num_attempts += 1;
    }

    pub fn get_due(&self) -> Option<DateTime<Utc>> {
        let inner = self.inner.lock().unwrap();
        inner.due
    }

    pub fn delay_with_jitter(&self, limit: i64) {
        let scale = rand::random::<f32>();
        let value = (scale * limit as f32) as i64;
        self.delay_by(chrono::Duration::seconds(value));
    }

    pub fn delay_by(&self, duration: chrono::Duration) {
        let due = Utc::now() + duration;
        self.set_due(Some(due));
    }

    pub fn set_due(&self, due: Option<DateTime<Utc>>) {
        let mut inner = self.inner.lock().unwrap();
        inner.due = due;
    }

    fn get_data_if_dirty(&self) -> Option<Arc<Box<[u8]>>> {
        let inner = self.inner.lock().unwrap();
        if inner.flags.contains(MessageFlags::DATA_DIRTY) {
            Some(Arc::clone(&inner.data))
        } else {
            None
        }
    }

    fn get_meta_if_dirty(&self) -> Option<MetaData> {
        let inner = self.inner.lock().unwrap();
        if inner.flags.contains(MessageFlags::META_DIRTY) {
            inner.metadata.clone()
        } else {
            None
        }
    }

    pub fn needs_save(&self) -> bool {
        let inner = self.inner.lock().unwrap();
        inner
            .flags
            .contains(MessageFlags::META_DIRTY | MessageFlags::DATA_DIRTY)
    }

    pub async fn save_to(
        &self,
        meta_spool: &(dyn Spool + Send + Sync),
        data_spool: &(dyn Spool + Send + Sync),
    ) -> anyhow::Result<()> {
        let data_holder;
        let data_fut = if let Some(data) = self.get_data_if_dirty() {
            anyhow::ensure!(!data.is_empty(), "message data must not be empty");
            data_holder = data;
            data_spool
                .store(self.id, &data_holder)
                .map(|_| true)
                .boxed()
        } else {
            futures::future::ready(false).boxed()
        };
        let meta_holder;
        let meta_fut = if let Some(meta) = self.get_meta_if_dirty() {
            meta_holder = serde_json::to_vec(&meta)?;
            meta_spool
                .store(self.id, &meta_holder)
                .map(|_| true)
                .boxed()
        } else {
            futures::future::ready(false).boxed()
        };

        let (data_res, meta_res) = tokio::join!(data_fut, meta_fut);

        if data_res {
            self.inner
                .lock()
                .unwrap()
                .flags
                .remove(MessageFlags::DATA_DIRTY);
        }
        if meta_res {
            self.inner
                .lock()
                .unwrap()
                .flags
                .remove(MessageFlags::META_DIRTY);
        }
        Ok(())
    }

    pub fn id(&self) -> &SpoolId {
        &self.id
    }

    pub fn shrink(&self) -> anyhow::Result<()> {
        let mut inner = self.inner.lock().unwrap();
        if inner.flags.contains(MessageFlags::DATA_DIRTY) {
            anyhow::bail!("Cannot shrink message: DATA_DIRTY");
        }
        if inner.flags.contains(MessageFlags::META_DIRTY) {
            anyhow::bail!("Cannot shrink message: META_DIRTY");
        }
        if inner.metadata.take().is_some() {
            META_COUNT.dec();
        }
        if !inner.data.is_empty() {
            DATA_COUNT.dec();
        }
        inner.data = Arc::new(vec![].into_boxed_slice());
        Ok(())
    }

    pub fn sender(&self) -> anyhow::Result<EnvelopeAddress> {
        let inner = self.inner.lock().unwrap();
        match &inner.metadata {
            Some(meta) => Ok(meta.sender.clone()),
            None => anyhow::bail!("metadata is not loaded"),
        }
    }

    pub fn recipient(&self) -> anyhow::Result<EnvelopeAddress> {
        let inner = self.inner.lock().unwrap();
        match &inner.metadata {
            Some(meta) => Ok(meta.recipient.clone()),
            None => anyhow::bail!("metadata is not loaded"),
        }
    }

    pub fn is_meta_loaded(&self) -> bool {
        self.inner.lock().unwrap().metadata.is_some()
    }

    pub fn is_data_loaded(&self) -> bool {
        !self.inner.lock().unwrap().data.is_empty()
    }

    pub async fn load_meta(&self, meta_spool: &(dyn Spool + Send + Sync)) -> anyhow::Result<()> {
        let id = self.id();
        let data = meta_spool.load(*id).await?;
        let mut inner = self.inner.lock().unwrap();
        let was_not_loaded = inner.metadata.is_none();
        let metadata: MetaData = serde_json::from_slice(&data)?;
        inner.metadata.replace(metadata);
        if was_not_loaded {
            META_COUNT.inc();
        }
        Ok(())
    }

    pub async fn load_data(&self, data_spool: &(dyn Spool + Send + Sync)) -> anyhow::Result<()> {
        let data = data_spool.load(*self.id()).await?;
        let mut inner = self.inner.lock().unwrap();
        let was_empty = inner.data.is_empty();
        inner.data = Arc::new(data.into_boxed_slice());
        if was_empty {
            DATA_COUNT.inc();
        }
        Ok(())
    }

    pub fn assign_data(&self, data: Vec<u8>) {
        let mut inner = self.inner.lock().unwrap();
        let was_empty = inner.data.is_empty();
        inner.data = Arc::new(data.into_boxed_slice());
        inner.flags.set(MessageFlags::DATA_DIRTY, true);
        if was_empty {
            DATA_COUNT.inc();
        }
    }

    pub fn get_data(&self) -> Arc<Box<[u8]>> {
        let inner = self.inner.lock().unwrap();
        inner.data.clone()
    }

    pub fn set_meta<S: AsRef<str>, V: Into<serde_json::Value>>(
        &self,
        key: S,
        value: V,
    ) -> anyhow::Result<()> {
        let mut inner = self.inner.lock().unwrap();
        match &mut inner.metadata {
            None => anyhow::bail!("metadata must be loaded first"),
            Some(meta) => {
                let key = key.as_ref();
                let value = value.into();

                match &mut meta.meta {
                    serde_json::Value::Object(map) => {
                        map.insert(key.to_string(), value);
                    }
                    _ => anyhow::bail!("metadata is somehow not a json object"),
                }

                inner.flags.set(MessageFlags::META_DIRTY, true);
                Ok(())
            }
        }
    }

    /// Retrieve `key` as a String.
    pub fn get_meta_string<S: serde_json::value::Index + std::fmt::Display + Copy>(
        &self,
        key: S,
    ) -> anyhow::Result<Option<String>> {
        match self.get_meta(key) {
            Ok(serde_json::Value::String(value)) => Ok(Some(value.to_string())),
            Ok(serde_json::Value::Null) => Ok(None),
            hmm => {
                anyhow::bail!("expected '{key}' to be a string value, got {hmm:?}");
            }
        }
    }

    pub fn get_meta_obj(&self) -> anyhow::Result<serde_json::Value> {
        let inner = self.inner.lock().unwrap();
        match &inner.metadata {
            None => anyhow::bail!("metadata must be loaded first"),
            Some(meta) => Ok(meta.meta.clone()),
        }
    }

    pub fn get_meta<S: serde_json::value::Index>(
        &self,
        key: S,
    ) -> anyhow::Result<serde_json::Value> {
        let inner = self.inner.lock().unwrap();
        match &inner.metadata {
            None => anyhow::bail!("metadata must be loaded first"),
            Some(meta) => match meta.meta.get(key) {
                Some(value) => Ok(value.clone()),
                None => Ok(serde_json::Value::Null),
            },
        }
    }

    pub fn age(&self, now: DateTime<Utc>) -> chrono::Duration {
        self.id.age(now)
    }

    pub fn get_queue_name(&self) -> anyhow::Result<String> {
        Ok(match self.get_meta_string("queue")? {
            Some(name) => name,
            None => {
                let campaign = self.get_meta_string("campaign")?;
                let tenant = self.get_meta_string("tenant")?;
                let domain = self.recipient()?.domain().to_string().to_lowercase();
                match (campaign, tenant) {
                    (Some(c), Some(t)) => format!("{c}:{t}@{domain}"),
                    (Some(c), None) => format!("{c}:@{domain}"),
                    (None, Some(t)) => format!("{t}@{domain}"),
                    (None, None) => domain,
                }
            }
        })
    }

    pub fn prepend_header(&self, name: Option<&str>, value: &str) {
        let data = self.get_data();
        let mut new_data = Vec::with_capacity(size_header(name, value) + 2 + data.len());
        emit_header(&mut new_data, name, value);
        new_data.extend_from_slice(&data);
        self.assign_data(new_data);
    }

    pub fn append_header(&self, name: Option<&str>, value: &str) {
        let data = self.get_data();
        let mut new_data = Vec::with_capacity(size_header(name, value) + 2 + data.len());
        for (idx, window) in data.windows(4).enumerate() {
            if window == b"\r\n\r\n" {
                let headers = &data[0..idx + 2];
                let body = &data[idx + 2..];

                new_data.extend_from_slice(&headers);
                emit_header(&mut new_data, name, value);
                new_data.extend_from_slice(&body);
                self.assign_data(new_data);
                return;
            }
        }
    }

    pub fn get_first_named_header_value(&self, name: &str) -> anyhow::Result<Option<String>> {
        let data = self.get_data();
        let (headers, _body_offset) = mailparse::parse_headers(&data)?;

        match headers.get_first_header(name) {
            Some(hdr) => Ok(Some(hdr.get_value_utf8()?)),
            None => Ok(None),
        }
    }

    pub fn get_all_named_header_values(&self, name: &str) -> anyhow::Result<Vec<String>> {
        let data = self.get_data();
        let (headers, _body_offset) = mailparse::parse_headers(&data)?;

        let mut values = vec![];
        for hdr in headers.get_all_headers(name) {
            values.push(hdr.get_value_utf8()?);
        }
        Ok(values)
    }

    pub fn get_all_headers(&self) -> anyhow::Result<Vec<(String, String)>> {
        let data = self.get_data();
        let (headers, _body_offset) = mailparse::parse_headers(&data)?;

        let mut values = vec![];
        for hdr in headers {
            values.push((hdr.get_key(), hdr.get_value_utf8()?));
        }
        Ok(values)
    }

    pub fn retain_headers<F: FnMut(&MailHeader) -> bool>(&self, mut func: F) -> anyhow::Result<()> {
        let data = self.get_data();
        let mut new_data = Vec::with_capacity(data.len());
        let (headers, body_offset) = mailparse::parse_headers(&data)?;
        for hdr in headers {
            let retain = (func)(&hdr);
            if !retain {
                continue;
            }
            new_data.extend_from_slice(hdr.get_key_raw());
            new_data.push(b':');
            new_data.push(b' ');
            new_data.extend_from_slice(hdr.get_value_raw());
            new_data.push(b'\r');
            new_data.push(b'\n');
        }
        new_data.push(b'\r');
        new_data.push(b'\n');
        new_data.extend_from_slice(&data[body_offset..]);
        self.assign_data(new_data);
        Ok(())
    }

    pub fn remove_first_named_header(&self, name: &str) -> anyhow::Result<()> {
        let mut removed = false;
        self.retain_headers(|hdr| {
            if hdr.get_key_ref().eq_ignore_ascii_case(name) && !removed {
                removed = true;
                false
            } else {
                true
            }
        })
    }

    pub fn import_x_headers(&self, names: Vec<String>) -> anyhow::Result<()> {
        let data = self.get_data();
        let (headers, _body_offset) = mailparse::parse_headers(&data)?;

        for hdr in headers {
            let do_import = if names.is_empty() {
                is_x_header(&hdr)
            } else {
                is_header_in_names_list(&hdr, &names)
            };
            if do_import {
                let name = imported_header_name(&hdr);
                self.set_meta(name, hdr.get_value_utf8()?)?;
            }
        }

        Ok(())
    }

    pub fn remove_x_headers(&self, names: Vec<String>) -> anyhow::Result<()> {
        self.retain_headers(|hdr| {
            if names.is_empty() {
                !is_x_header(hdr)
            } else {
                !is_header_in_names_list(&hdr, &names)
            }
        })
    }

    pub fn remove_all_named_header(&self, name: &str) -> anyhow::Result<()> {
        self.retain_headers(|hdr| !hdr.get_key_ref().eq_ignore_ascii_case(name))
    }

    pub fn dkim_sign(&self, signer: &Signer) -> anyhow::Result<()> {
        let data = self.get_data();
        let signature = signer.sign(&data)?;
        let header = signature.to_header();
        self.prepend_header(None, &header);
        Ok(())
    }
}

fn is_header_in_names_list(hdr: &MailHeader, names: &[String]) -> bool {
    let hdr_name = hdr.get_key_ref();
    for name in names {
        if hdr_name.eq_ignore_ascii_case(name) {
            return true;
        }
    }
    false
}

fn imported_header_name(hdr: &MailHeader) -> String {
    hdr.get_key_ref()
        .chars()
        .map(|c| match c.to_ascii_lowercase() {
            '-' => '_',
            c => c,
        })
        .collect()
}

fn is_x_header(hdr: &MailHeader) -> bool {
    let name = hdr.get_key_ref();
    name.starts_with("X-") || name.starts_with("x-")
}

fn size_header(name: Option<&str>, value: &str) -> usize {
    name.map(|name| name.len() + 2).unwrap_or(0) + value.len()
}

fn emit_header(dest: &mut Vec<u8>, name: Option<&str>, value: &str) {
    if let Some(name) = name {
        dest.extend_from_slice(name.as_bytes());
        dest.extend_from_slice(b": ");
    }
    dest.extend_from_slice(value.as_bytes());
    if !value.ends_with("\r\n") {
        dest.extend_from_slice(b"\r\n");
    }
}

impl UserData for Message {
    fn add_methods<'lua, M: UserDataMethods<'lua, Self>>(methods: &mut M) {
        methods.add_method(
            "set_meta",
            move |_, this, (name, value): (String, mlua::Value)| {
                let value = serde_json::value::to_value(value)
                    .map_err(|err| mlua::Error::external(format!("{err:#}")))?;
                this.set_meta(name, value)
                    .map_err(|err| mlua::Error::external(format!("{err:#}")))?;
                Ok(())
            },
        );
        methods.add_method("get_meta", move |lua, this, name: String| {
            let value = this
                .get_meta(name)
                .map_err(|err| mlua::Error::external(format!("{err:#}")))?;
            Ok(Some(lua.to_value(&value)?))
        });
        methods.add_method("id", move |_, this, _: ()| Ok(this.id().to_string()));
        methods.add_method("sender", move |_, this, _: ()| {
            Ok(this
                .sender()
                .map_err(|err| mlua::Error::external(format!("{err:#}")))?)
        });
        methods.add_method("recipient", move |_, this, _: ()| {
            Ok(this
                .recipient()
                .map_err(|err| mlua::Error::external(format!("{err:#}")))?)
        });
        methods.add_method("dkim_sign", move |_, this, signer: Signer| {
            Ok(this
                .dkim_sign(&signer)
                .map_err(|err| mlua::Error::external(format!("{err:#}")))?)
        });
        methods.add_method(
            "prepend_header",
            move |_, this, (name, value): (String, String)| {
                Ok(this.prepend_header(Some(&name), &value))
            },
        );
        methods.add_method(
            "append_header",
            move |_, this, (name, value): (String, String)| {
                Ok(this.append_header(Some(&name), &value))
            },
        );
        methods.add_method(
            "get_first_named_header_value",
            move |_, this, name: String| {
                Ok(this
                    .get_first_named_header_value(&name)
                    .map_err(|err| mlua::Error::external(format!("{err:#}")))?)
            },
        );
        methods.add_method(
            "get_all_named_header_values",
            move |_, this, name: String| {
                Ok(this
                    .get_all_named_header_values(&name)
                    .map_err(|err| mlua::Error::external(format!("{err:#}")))?)
            },
        );
        methods.add_method("get_all_headers", move |_, this, _: ()| {
            Ok(this
                .get_all_headers()
                .map_err(|err| mlua::Error::external(format!("{err:#}")))?
                .into_iter()
                .map(|(name, value)| vec![name, value])
                .collect::<Vec<Vec<String>>>())
        });
        methods.add_method("get_all_headers", move |_, this, _: ()| {
            Ok(this
                .get_all_headers()
                .map_err(|err| mlua::Error::external(format!("{err:#}")))?
                .into_iter()
                .map(|(name, value)| vec![name, value])
                .collect::<Vec<Vec<String>>>())
        });
        methods.add_method(
            "import_x_headers",
            move |_, this, names: Option<Vec<String>>| {
                Ok(this
                    .import_x_headers(names.unwrap_or_else(|| vec![]))
                    .map_err(|err| mlua::Error::external(format!("{err:#}")))?)
            },
        );

        methods.add_method(
            "remove_x_headers",
            move |_, this, names: Option<Vec<String>>| {
                Ok(this
                    .remove_x_headers(names.unwrap_or_else(|| vec![]))
                    .map_err(|err| mlua::Error::external(format!("{err:#}")))?)
            },
        );
    }
}

impl CancellableTimerEntry for Message {
    type Id = SpoolId;

    fn id(&self) -> &Self::Id {
        self.id()
    }
}

impl TimerEntryWithDelay for Message {
    fn delay(&self) -> Duration {
        let inner = self.inner.lock().unwrap();
        match inner.due {
            Some(time) => {
                let now = Utc::now();
                if time <= now {
                    Duration::from_millis(0)
                } else {
                    Duration::from_millis((time - now).num_milliseconds() as u64)
                }
            }
            None => Duration::from_millis(0),
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use serde_json::json;

    fn new_msg_body<S: AsRef<str>>(s: S) -> Message {
        Message::new_dirty(
            SpoolId::new(),
            EnvelopeAddress::parse("sender@example.com").unwrap(),
            EnvelopeAddress::parse("recip@example.com").unwrap(),
            serde_json::json!({}),
            Arc::new(s.as_ref().as_bytes().to_vec().into_boxed_slice()),
        )
        .unwrap()
    }

    fn data_as_string(msg: &Message) -> String {
        String::from_utf8(msg.get_data().to_vec()).unwrap()
    }

    const X_HDR_CONTENT: &str =
        "X-Hello: there\r\nX-Header: value\r\nSubject: Hello\r\nFrom :Someone\r\n\r\nBody";

    #[test]
    fn import_all_x_headers() {
        let msg = new_msg_body(X_HDR_CONTENT);

        msg.import_x_headers(vec![]).unwrap();
        k9::assert_equal!(
            msg.get_meta_obj().unwrap(),
            json!({
                "x_hello": "there",
                "x_header": "value",
            })
        );
    }

    #[test]
    fn import_some_x_headers() {
        let msg = new_msg_body(X_HDR_CONTENT);

        msg.import_x_headers(vec!["x-hello".to_string()]).unwrap();
        k9::assert_equal!(
            msg.get_meta_obj().unwrap(),
            json!({
                "x_hello": "there",
            })
        );
    }

    #[test]
    fn remove_all_x_headers() {
        let msg = new_msg_body(X_HDR_CONTENT);

        msg.remove_x_headers(vec![]).unwrap();
        k9::assert_equal!(
            data_as_string(&msg),
            "Subject: Hello\r\nFrom : Someone\r\n\r\nBody"
        );
    }

    #[test]
    fn prepend_header_2_params() {
        let msg = new_msg_body(X_HDR_CONTENT);

        msg.prepend_header(Some("Date"), "Today");
        k9::assert_equal!(
            data_as_string(&msg),
            "Date: Today\r\nX-Hello: there\r\nX-Header: value\r\nSubject: Hello\r\nFrom :Someone\r\n\r\nBody"
        );
    }

    #[test]
    fn prepend_header_1_params() {
        let msg = new_msg_body(X_HDR_CONTENT);

        msg.prepend_header(None, "Date: Today");
        k9::assert_equal!(
            data_as_string(&msg),
            "Date: Today\r\nX-Hello: there\r\nX-Header: value\r\nSubject: Hello\r\nFrom :Someone\r\n\r\nBody"
        );
    }

    #[test]
    fn append_header_2_params() {
        let msg = new_msg_body(X_HDR_CONTENT);

        msg.append_header(Some("Date"), "Today");
        k9::assert_equal!(
            data_as_string(&msg),
            "X-Hello: there\r\nX-Header: value\r\nSubject: Hello\r\nFrom :Someone\r\nDate: Today\r\n\r\nBody"
        );
    }

    #[test]
    fn append_header_1_params() {
        let msg = new_msg_body(X_HDR_CONTENT);

        msg.append_header(None, "Date: Today");
        k9::assert_equal!(
            data_as_string(&msg),
            "X-Hello: there\r\nX-Header: value\r\nSubject: Hello\r\nFrom :Someone\r\nDate: Today\r\n\r\nBody"
        );
    }

    const MULTI_HEADER_CONTENT: &str =
        "X-Hello: there\r\nX-Header: value\r\nSubject: Hello\r\nX-Header: another value\r\nFrom :Someone\r\n\r\nBody";

    #[test]
    fn get_first_header() {
        let msg = new_msg_body(MULTI_HEADER_CONTENT);
        k9::assert_equal!(
            msg.get_first_named_header_value("X-header")
                .unwrap()
                .unwrap(),
            "value"
        );
    }

    #[test]
    fn get_all_header() {
        let msg = new_msg_body(MULTI_HEADER_CONTENT);
        k9::assert_equal!(
            msg.get_all_named_header_values("X-header").unwrap(),
            vec!["value".to_string(), "another value".to_string()]
        );
    }

    #[test]
    fn remove_first() {
        let msg = new_msg_body(MULTI_HEADER_CONTENT);
        msg.remove_first_named_header("X-header").unwrap();
        k9::assert_equal!(
            data_as_string(&msg),
            "X-Hello: there\r\nSubject: Hello\r\nX-Header: another value\r\nFrom : Someone\r\n\r\nBody"
        );
    }

    #[test]
    fn remove_all() {
        let msg = new_msg_body(MULTI_HEADER_CONTENT);
        msg.remove_all_named_header("X-header").unwrap();
        k9::assert_equal!(
            data_as_string(&msg),
            "X-Hello: there\r\nSubject: Hello\r\nFrom : Someone\r\n\r\nBody"
        );
    }
}
