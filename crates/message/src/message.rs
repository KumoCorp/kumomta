use crate::address::HeaderAddressList;
#[cfg(feature = "impl")]
use crate::dkim::Signer;
#[cfg(feature = "impl")]
use crate::dkim::SIGN_POOL;
pub use crate::queue_name::QueueNameComponents;
use crate::scheduling::Scheduling;
use crate::EnvelopeAddress;
use anyhow::Context;
use chrono::{DateTime, Utc};
#[cfg(feature = "impl")]
use config::{any_err, from_lua_value, serialize_options};
use futures::FutureExt;
use intrusive_collections::{intrusive_adapter, LinkedList, LinkedListAtomicLink};
use kumo_chrono_helper::*;
use kumo_log_types::rfc3464::Report;
use kumo_log_types::rfc5965::ARFReport;
#[cfg(feature = "impl")]
use mailparsing::{AuthenticationResult, AuthenticationResults, EncodeHeaderValue};
use mailparsing::{DecodedBody, Header, HeaderParseResult, MessageConformance, MimePart};
#[cfg(feature = "impl")]
use mlua::{LuaSerdeExt, UserData, UserDataMethods};
use prometheus::{Histogram, IntGauge};
use serde::{Deserialize, Serialize};
use spool::{get_data_spool, get_meta_spool, Spool, SpoolId};
use std::hash::Hash;
use std::sync::{Arc, LazyLock, Mutex, Weak};
use std::time::{Duration, Instant};
use timeq::TimerEntryWithDelay;

bitflags::bitflags! {
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    struct MessageFlags: u8 {
        /// true if Metadata needs to be saved
        const META_DIRTY = 1;
        /// true if Data needs to be saved
        const DATA_DIRTY = 2;
        /// true if scheduling restrictions are present in the metadata
        const SCHEDULED = 4;
        /// true if high durability writes should always be used
        const FORCE_SYNC = 8;
    }
}

static MESSAGE_COUNT: LazyLock<IntGauge> = LazyLock::new(|| {
    prometheus::register_int_gauge!("message_count", "total number of Message objects").unwrap()
});
static META_COUNT: LazyLock<IntGauge> = LazyLock::new(|| {
    prometheus::register_int_gauge!(
        "message_meta_resident_count",
        "total number of Message objects with metadata loaded"
    )
    .unwrap()
});
static DATA_COUNT: LazyLock<IntGauge> = LazyLock::new(|| {
    prometheus::register_int_gauge!(
        "message_data_resident_count",
        "total number of Message objects with body data loaded"
    )
    .unwrap()
});
static NO_DATA: LazyLock<Arc<Box<[u8]>>> = LazyLock::new(|| Arc::new(vec![].into_boxed_slice()));
static SAVE_HIST: LazyLock<Histogram> = LazyLock::new(|| {
    prometheus::register_histogram!(
        "message_save_latency",
        "how long it takes to save a message to spool"
    )
    .unwrap()
});
static LOAD_DATA_HIST: LazyLock<Histogram> = LazyLock::new(|| {
    prometheus::register_histogram!(
        "message_data_load_latency",
        "how long it takes to load message data from spool"
    )
    .unwrap()
});
static LOAD_META_HIST: LazyLock<Histogram> = LazyLock::new(|| {
    prometheus::register_histogram!(
        "message_meta_load_latency",
        "how long it takes to load message metadata from spool"
    )
    .unwrap()
});

#[derive(Debug)]
struct MessageInner {
    metadata: Option<Box<MetaData>>,
    data: Arc<Box<[u8]>>,
    flags: MessageFlags,
    num_attempts: u16,
    due: Option<DateTime<Utc>>,
}

#[derive(Debug)]
pub(crate) struct MessageWithId {
    id: SpoolId,
    inner: Mutex<MessageInner>,
    link: LinkedListAtomicLink,
}

intrusive_adapter!(
    pub(crate) MessageWithIdAdapter = Arc<MessageWithId>: MessageWithId { link: LinkedListAtomicLink }
);

/// A list of messages with an O(1) list overhead; no additional
/// memory per-message is required to track this list.
/// However, a given Message can only belong to one instance
/// of such a list at a time.
pub struct MessageList {
    list: LinkedList<MessageWithIdAdapter>,
    len: usize,
}

impl Default for MessageList {
    fn default() -> Self {
        Self::new()
    }
}

impl MessageList {
    /// Create a new MessageList
    pub fn new() -> Self {
        Self {
            list: LinkedList::new(MessageWithIdAdapter::default()),
            len: 0,
        }
    }

    /// Returns the number of elements contained in the list
    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Take all of the elements from this list and return them
    /// in a new separate instance of MessageList.
    pub fn take(&mut self) -> Self {
        let new_list = Self {
            list: self.list.take(),
            len: self.len,
        };
        self.len = 0;
        new_list
    }

    /// Push a message to the back of the list
    pub fn push_back(&mut self, message: Message) {
        self.list.push_back(message.msg_and_id);
        self.len += 1;
    }

    /// Pop a message from the front of the list
    pub fn pop_front(&mut self) -> Option<Message> {
        self.list.pop_front().map(|msg_and_id| {
            self.len -= 1;
            Message { msg_and_id }
        })
    }

    /// Pop a message from the back of the list
    pub fn pop_back(&mut self) -> Option<Message> {
        self.list.pop_back().map(|msg_and_id| {
            self.len -= 1;
            Message { msg_and_id }
        })
    }

    /// Pop all of the messages from this list into a vector
    /// of messages.
    /// Memory usage is O(number-of-messages).
    pub fn drain(&mut self) -> Vec<Message> {
        let mut messages = Vec::with_capacity(self.len);
        while let Some(msg) = self.pop_front() {
            messages.push(msg);
        }
        messages
    }

    pub fn extend_from_iter<I>(&mut self, mut iter: I)
    where
        I: Iterator<Item = Message>,
    {
        while let Some(msg) = iter.next() {
            self.push_back(msg)
        }
    }
}

impl IntoIterator for MessageList {
    type Item = Message;
    type IntoIter = MessageListIter;
    fn into_iter(self) -> MessageListIter {
        MessageListIter { list: self.list }
    }
}

pub struct MessageListIter {
    list: LinkedList<MessageWithIdAdapter>,
}

impl Iterator for MessageListIter {
    type Item = Message;
    fn next(&mut self) -> Option<Message> {
        self.list
            .pop_front()
            .map(|msg_and_id| Message { msg_and_id })
    }
}

#[derive(Clone, Debug)]
#[cfg_attr(feature = "impl", derive(mlua::FromLua))]
pub struct Message {
    pub(crate) msg_and_id: Arc<MessageWithId>,
}

impl PartialEq for Message {
    fn eq(&self, other: &Self) -> bool {
        self.id() == other.id()
    }
}
impl Eq for Message {}

impl Hash for Message {
    fn hash<H>(&self, hasher: &mut H)
    where
        H: std::hash::Hasher,
    {
        self.id().hash(hasher)
    }
}

#[derive(Clone, Debug)]
pub struct WeakMessage {
    weak: Weak<MessageWithId>,
}

impl WeakMessage {
    pub fn upgrade(&self) -> Option<Message> {
        Some(Message {
            msg_and_id: self.weak.upgrade()?,
        })
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct MetaData {
    sender: EnvelopeAddress,
    recipient: EnvelopeAddress,
    meta: serde_json::Value,
    #[serde(default)]
    schedule: Option<Scheduling>,
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
            msg_and_id: Arc::new(MessageWithId {
                id,
                inner: Mutex::new(MessageInner {
                    metadata: Some(Box::new(MetaData {
                        sender,
                        recipient,
                        meta,
                        schedule: None,
                    })),
                    data,
                    flags: MessageFlags::META_DIRTY | MessageFlags::DATA_DIRTY,
                    num_attempts: 0,
                    due: None,
                }),
                link: LinkedListAtomicLink::default(),
            }),
        })
    }

    pub fn weak(&self) -> WeakMessage {
        WeakMessage {
            weak: Arc::downgrade(&self.msg_and_id),
        }
    }

    /// Helper for creating a message based on spool enumeration.
    /// Given a spool id and the serialized metadata blob, returns
    /// a message holding the deserialized version of that metadata.
    pub fn new_from_spool(id: SpoolId, metadata: Vec<u8>) -> anyhow::Result<Self> {
        let metadata: MetaData = serde_json::from_slice(&metadata)?;
        MESSAGE_COUNT.inc();
        META_COUNT.inc();

        let flags = if metadata.schedule.is_some() {
            MessageFlags::SCHEDULED
        } else {
            MessageFlags::empty()
        };

        Ok(Self {
            msg_and_id: Arc::new(MessageWithId {
                id,
                inner: Mutex::new(MessageInner {
                    metadata: Some(Box::new(metadata)),
                    data: NO_DATA.clone(),
                    flags,
                    num_attempts: 0,
                    due: None,
                }),
                link: LinkedListAtomicLink::default(),
            }),
        })
    }

    pub async fn new_with_id(id: SpoolId) -> anyhow::Result<Self> {
        let meta_spool = get_meta_spool();
        let data = meta_spool.load(id).await?;
        Self::new_from_spool(id, data)
    }

    pub fn get_num_attempts(&self) -> u16 {
        let inner = self.msg_and_id.inner.lock().unwrap();
        inner.num_attempts
    }

    pub fn set_num_attempts(&self, num_attempts: u16) {
        let mut inner = self.msg_and_id.inner.lock().unwrap();
        inner.num_attempts = num_attempts;
    }

    pub fn increment_num_attempts(&self) {
        let mut inner = self.msg_and_id.inner.lock().unwrap();
        inner.num_attempts += 1;
    }

    pub fn set_scheduling(
        &self,
        scheduling: Option<Scheduling>,
    ) -> anyhow::Result<Option<Scheduling>> {
        let mut inner = self.msg_and_id.inner.lock().unwrap();
        match &mut inner.metadata {
            None => anyhow::bail!("metadata must be loaded first"),
            Some(meta) => {
                meta.schedule = scheduling;
                inner
                    .flags
                    .set(MessageFlags::SCHEDULED, scheduling.is_some());
                if let Some(sched) = scheduling {
                    let due = inner.due.unwrap_or_else(|| Utc::now());
                    inner.due = Some(sched.adjust_for_schedule(due));
                }
                Ok(scheduling)
            }
        }
    }

    pub fn get_scheduling(&self) -> Option<Scheduling> {
        let inner = self.msg_and_id.inner.lock().unwrap();
        inner
            .metadata
            .as_ref()
            .and_then(|meta| meta.schedule.clone())
    }

    pub fn get_due(&self) -> Option<DateTime<Utc>> {
        let inner = self.msg_and_id.inner.lock().unwrap();
        inner.due
    }

    pub async fn delay_with_jitter(&self, limit: i64) -> anyhow::Result<Option<DateTime<Utc>>> {
        let scale = rand::random::<f32>();
        let value = (scale * limit as f32) as i64;
        self.delay_by(seconds(value)?).await
    }

    pub async fn delay_by(
        &self,
        duration: chrono::Duration,
    ) -> anyhow::Result<Option<DateTime<Utc>>> {
        let due = Utc::now() + duration;
        self.set_due(Some(due)).await
    }

    /// Delay by requested duration, and add up to 1 minute of jitter
    pub async fn delay_by_and_jitter(
        &self,
        duration: chrono::Duration,
    ) -> anyhow::Result<Option<DateTime<Utc>>> {
        let scale = rand::random::<f32>();
        let value = (scale * 60.) as i64;
        let due = Utc::now() + duration + seconds(value)?;
        self.set_due(Some(due)).await
    }

    pub async fn set_due(
        &self,
        due: Option<DateTime<Utc>>,
    ) -> anyhow::Result<Option<DateTime<Utc>>> {
        let due = {
            let mut inner = self.msg_and_id.inner.lock().unwrap();

            if !inner.flags.contains(MessageFlags::SCHEDULED) {
                // This is the simple, fast-path, common case
                inner.due = due;
                return Ok(inner.due);
            }

            let due = due.unwrap_or_else(|| Utc::now());

            if let Some(meta) = &inner.metadata {
                inner.due = match &meta.schedule {
                    Some(sched) => Some(sched.adjust_for_schedule(due)),
                    None => Some(due),
                };
                return Ok(inner.due);
            }

            // We'll need to load the metadata to correctly
            // update the schedule for this message
            due
        };

        self.load_meta().await?;

        {
            let mut inner = self.msg_and_id.inner.lock().unwrap();
            match &inner.metadata {
                Some(meta) => {
                    inner.due = match &meta.schedule {
                        Some(sched) => Some(sched.adjust_for_schedule(due)),
                        None => Some(due),
                    };
                    Ok(inner.due)
                }
                None => anyhow::bail!("loaded metadata, but metadata is not set!?"),
            }
        }
    }

    fn get_data_if_dirty(&self) -> Option<Arc<Box<[u8]>>> {
        let inner = self.msg_and_id.inner.lock().unwrap();
        if inner.flags.contains(MessageFlags::DATA_DIRTY) {
            Some(Arc::clone(&inner.data))
        } else {
            None
        }
    }

    fn get_meta_if_dirty(&self) -> Option<MetaData> {
        let inner = self.msg_and_id.inner.lock().unwrap();
        if inner.flags.contains(MessageFlags::META_DIRTY) {
            inner.metadata.as_ref().map(|md| (**md).clone())
        } else {
            None
        }
    }

    pub fn set_force_sync(&self, force: bool) {
        let mut inner = self.msg_and_id.inner.lock().unwrap();
        inner.flags.set(MessageFlags::FORCE_SYNC, force);
    }

    pub fn needs_save(&self) -> bool {
        let inner = self.msg_and_id.inner.lock().unwrap();
        inner.flags.contains(MessageFlags::META_DIRTY)
            || inner.flags.contains(MessageFlags::DATA_DIRTY)
    }

    pub async fn save(&self, deadline: Option<Instant>) -> anyhow::Result<()> {
        let _timer = SAVE_HIST.start_timer();
        self.save_to(&**get_meta_spool(), &**get_data_spool(), deadline)
            .await
    }

    pub async fn save_to(
        &self,
        meta_spool: &(dyn Spool + Send + Sync),
        data_spool: &(dyn Spool + Send + Sync),
        deadline: Option<Instant>,
    ) -> anyhow::Result<()> {
        let force_sync = self
            .msg_and_id
            .inner
            .lock()
            .unwrap()
            .flags
            .contains(MessageFlags::FORCE_SYNC);

        let data_fut = if let Some(data) = self.get_data_if_dirty() {
            anyhow::ensure!(!data.is_empty(), "message data must not be empty");
            data_spool
                .store(self.msg_and_id.id, data, force_sync, deadline)
                .map(|_| true)
                .boxed()
        } else {
            futures::future::ready(false).boxed()
        };
        let meta_fut = if let Some(meta) = self.get_meta_if_dirty() {
            let meta = Arc::new(serde_json::to_vec(&meta)?.into_boxed_slice());
            meta_spool
                .store(self.msg_and_id.id, meta, force_sync, deadline)
                .map(|_| true)
                .boxed()
        } else {
            futures::future::ready(false).boxed()
        };

        // NOTE: if we have a deadline, it is tempting to want to use
        // timeout_at here to enforce it, but the underlying spool
        // futures are not guaranteed to be fully cancel safe, which
        // is why we pass the deadline down to the save method to allow
        // them to handle timeouts internally.
        let (data_res, meta_res) = tokio::join!(data_fut, meta_fut);

        if data_res {
            self.msg_and_id
                .inner
                .lock()
                .unwrap()
                .flags
                .remove(MessageFlags::DATA_DIRTY);
        }
        if meta_res {
            self.msg_and_id
                .inner
                .lock()
                .unwrap()
                .flags
                .remove(MessageFlags::META_DIRTY);
        }
        Ok(())
    }

    pub fn id(&self) -> &SpoolId {
        &self.msg_and_id.id
    }

    /// Save the data+meta if needed, then release both
    pub async fn save_and_shrink(&self) -> anyhow::Result<bool> {
        self.save(None).await?;
        self.shrink()
    }

    /// Save the data+meta if needed, then release just the data
    pub async fn save_and_shrink_data(&self) -> anyhow::Result<bool> {
        self.save(None).await?;
        self.shrink_data()
    }

    pub fn shrink_data(&self) -> anyhow::Result<bool> {
        let mut inner = self.msg_and_id.inner.lock().unwrap();
        let mut did_shrink = false;
        if inner.flags.contains(MessageFlags::DATA_DIRTY) {
            anyhow::bail!("Cannot shrink message: DATA_DIRTY");
        }
        if !inner.data.is_empty() {
            DATA_COUNT.dec();
            did_shrink = true;
        }
        if !inner.data.is_empty() {
            inner.data = NO_DATA.clone();
            did_shrink = true;
        }
        Ok(did_shrink)
    }

    pub fn shrink(&self) -> anyhow::Result<bool> {
        let mut inner = self.msg_and_id.inner.lock().unwrap();
        let mut did_shrink = false;
        if inner.flags.contains(MessageFlags::DATA_DIRTY) {
            anyhow::bail!("Cannot shrink message: DATA_DIRTY");
        }
        if inner.flags.contains(MessageFlags::META_DIRTY) {
            anyhow::bail!("Cannot shrink message: META_DIRTY");
        }
        if inner.metadata.take().is_some() {
            META_COUNT.dec();
            did_shrink = true;
        }
        if !inner.data.is_empty() {
            DATA_COUNT.dec();
            did_shrink = true;
        }
        if !inner.data.is_empty() {
            inner.data = NO_DATA.clone();
            did_shrink = true;
        }
        Ok(did_shrink)
    }

    pub fn sender(&self) -> anyhow::Result<EnvelopeAddress> {
        let inner = self.msg_and_id.inner.lock().unwrap();
        match &inner.metadata {
            Some(meta) => Ok(meta.sender.clone()),
            None => anyhow::bail!("metadata is not loaded"),
        }
    }

    pub fn set_sender(&self, sender: EnvelopeAddress) -> anyhow::Result<()> {
        let mut inner = self.msg_and_id.inner.lock().unwrap();
        match &mut inner.metadata {
            Some(meta) => {
                meta.sender = sender;
                inner.flags.set(MessageFlags::DATA_DIRTY, true);
                Ok(())
            }
            None => anyhow::bail!("metadata is not loaded"),
        }
    }

    pub fn recipient(&self) -> anyhow::Result<EnvelopeAddress> {
        let inner = self.msg_and_id.inner.lock().unwrap();
        match &inner.metadata {
            Some(meta) => Ok(meta.recipient.clone()),
            None => anyhow::bail!("metadata is not loaded"),
        }
    }

    pub fn set_recipient(&self, recipient: EnvelopeAddress) -> anyhow::Result<()> {
        let mut inner = self.msg_and_id.inner.lock().unwrap();
        match &mut inner.metadata {
            Some(meta) => {
                meta.recipient = recipient;
                inner.flags.set(MessageFlags::DATA_DIRTY, true);
                Ok(())
            }
            None => anyhow::bail!("metadata is not loaded"),
        }
    }

    pub fn is_meta_loaded(&self) -> bool {
        self.msg_and_id.inner.lock().unwrap().metadata.is_some()
    }

    pub fn is_data_loaded(&self) -> bool {
        !self.msg_and_id.inner.lock().unwrap().data.is_empty()
    }

    pub async fn load_meta_if_needed(&self) -> anyhow::Result<()> {
        if self.is_meta_loaded() {
            return Ok(());
        }
        self.load_meta().await
    }

    pub async fn load_data_if_needed(&self) -> anyhow::Result<()> {
        if self.is_data_loaded() {
            return Ok(());
        }
        self.load_data().await
    }

    pub async fn load_meta(&self) -> anyhow::Result<()> {
        let _timer = LOAD_META_HIST.start_timer();
        self.load_meta_from(&**get_meta_spool()).await
    }

    pub async fn load_meta_from(
        &self,
        meta_spool: &(dyn Spool + Send + Sync),
    ) -> anyhow::Result<()> {
        let id = self.id();
        let data = meta_spool.load(*id).await?;
        let mut inner = self.msg_and_id.inner.lock().unwrap();
        let was_not_loaded = inner.metadata.is_none();
        let metadata: MetaData = serde_json::from_slice(&data)?;
        inner.metadata.replace(Box::new(metadata));
        if was_not_loaded {
            META_COUNT.inc();
        }
        Ok(())
    }

    pub async fn load_data(&self) -> anyhow::Result<()> {
        let _timer = LOAD_DATA_HIST.start_timer();
        self.load_data_from(&**get_data_spool()).await
    }

    pub async fn load_data_from(
        &self,
        data_spool: &(dyn Spool + Send + Sync),
    ) -> anyhow::Result<()> {
        let data = data_spool.load(*self.id()).await?;
        let mut inner = self.msg_and_id.inner.lock().unwrap();
        let was_empty = inner.data.is_empty();
        inner.data = Arc::new(data.into_boxed_slice());
        if was_empty {
            DATA_COUNT.inc();
        }
        Ok(())
    }

    pub fn assign_data(&self, data: Vec<u8>) {
        let mut inner = self.msg_and_id.inner.lock().unwrap();
        let was_empty = inner.data.is_empty();
        inner.data = Arc::new(data.into_boxed_slice());
        inner.flags.set(MessageFlags::DATA_DIRTY, true);
        if was_empty {
            DATA_COUNT.inc();
        }
    }

    pub fn get_data(&self) -> Arc<Box<[u8]>> {
        let inner = self.msg_and_id.inner.lock().unwrap();
        inner.data.clone()
    }

    pub fn set_meta<S: AsRef<str>, V: Into<serde_json::Value>>(
        &self,
        key: S,
        value: V,
    ) -> anyhow::Result<()> {
        let mut inner = self.msg_and_id.inner.lock().unwrap();
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
        let inner = self.msg_and_id.inner.lock().unwrap();
        match &inner.metadata {
            None => anyhow::bail!("metadata must be loaded first"),
            Some(meta) => Ok(meta.meta.clone()),
        }
    }

    pub fn get_meta<S: serde_json::value::Index>(
        &self,
        key: S,
    ) -> anyhow::Result<serde_json::Value> {
        let inner = self.msg_and_id.inner.lock().unwrap();
        match &inner.metadata {
            None => anyhow::bail!("metadata must be loaded first"),
            Some(meta) => match meta.meta.get(key) {
                Some(value) => Ok(value.clone()),
                None => Ok(serde_json::Value::Null),
            },
        }
    }

    pub fn age(&self, now: DateTime<Utc>) -> chrono::Duration {
        self.msg_and_id.id.age(now)
    }

    pub fn get_queue_name(&self) -> anyhow::Result<String> {
        Ok(match self.get_meta_string("queue")? {
            Some(name) => name,
            None => {
                let name = QueueNameComponents::format(
                    self.get_meta_string("campaign")?,
                    self.get_meta_string("tenant")?,
                    self.recipient()?.domain().to_string().to_lowercase(),
                    self.get_meta_string("routing_domain")?,
                );
                name.to_string()
            }
        })
    }

    #[cfg(feature = "impl")]
    pub async fn dkim_verify(&self) -> anyhow::Result<Vec<AuthenticationResult>> {
        let resolver = dns_resolver::get_resolver();
        let data = self.get_data();
        let bytes = mailparsing::SharedString::try_from(data.as_ref().as_ref())?;

        let parsed = mailparsing::Header::parse_headers(bytes.clone())?;
        if parsed
            .overall_conformance
            .contains(MessageConformance::NON_CANONICAL_LINE_ENDINGS)
        {
            return Ok(vec![AuthenticationResult {
                method: "dkim".to_string(),
                method_version: None,
                result: "permerror".to_string(),
                reason: Some("message has non-canonical line endings".to_string()),
                props: Default::default(),
            }]);
        }
        let message = kumo_dkim::ParsedEmail::HeaderOnlyParse { bytes, parsed };

        let from = message
            .get_headers()
            .from()
            .map_err(any_err)?
            .ok_or_else(|| anyhow::anyhow!("Missing or invalid From header"))?
            .0;
        if from.len() != 1 {
            anyhow::bail!(
                "From header must have a single sender, found {}",
                from.len()
            );
        }
        let from_domain = &from[0].address.domain;

        let results =
            kumo_dkim::verify_email_with_resolver(from_domain, &message, &**resolver).await?;
        Ok(results)
    }

    pub fn parse_rfc3464(&self) -> anyhow::Result<Option<Report>> {
        let data = self.get_data();
        Report::parse(&data)
    }

    pub fn parse_rfc5965(&self) -> anyhow::Result<Option<ARFReport>> {
        let data = self.get_data();
        ARFReport::parse(&data)
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

    pub fn get_address_header(
        &self,
        header_name: &str,
    ) -> anyhow::Result<Option<HeaderAddressList>> {
        let data = self.get_data();
        let HeaderParseResult { headers, .. } =
            mailparsing::Header::parse_headers(data.as_ref().as_ref())?;

        match headers.get_first(header_name) {
            Some(hdr) => {
                let list = hdr.as_address_list()?;
                let result: HeaderAddressList = list.into();
                Ok(Some(result))
            }
            None => Ok(None),
        }
    }

    pub fn get_first_named_header_value(&self, name: &str) -> anyhow::Result<Option<String>> {
        let data = self.get_data();
        let HeaderParseResult { headers, .. } = Header::parse_headers(data.as_ref().as_ref())?;

        match headers.get_first(name) {
            Some(hdr) => Ok(Some(hdr.as_unstructured()?)),
            None => Ok(None),
        }
    }

    pub fn get_all_named_header_values(&self, name: &str) -> anyhow::Result<Vec<String>> {
        let data = self.get_data();
        let HeaderParseResult { headers, .. } = Header::parse_headers(data.as_ref().as_ref())?;

        let mut values = vec![];
        for hdr in headers.iter_named(name) {
            values.push(hdr.as_unstructured()?);
        }
        Ok(values)
    }

    pub fn get_all_headers(&self) -> anyhow::Result<Vec<(String, String)>> {
        let data = self.get_data();
        let HeaderParseResult { headers, .. } = Header::parse_headers(data.as_ref().as_ref())?;

        let mut values = vec![];
        for hdr in headers.iter() {
            values.push((hdr.get_name().to_string(), hdr.as_unstructured()?));
        }
        Ok(values)
    }

    pub fn retain_headers<F: FnMut(&Header) -> bool>(&self, mut func: F) -> anyhow::Result<()> {
        let data = self.get_data();
        let mut new_data = Vec::with_capacity(data.len());
        let HeaderParseResult {
            headers,
            body_offset,
            ..
        } = Header::parse_headers(data.as_ref().as_ref())?;
        for hdr in headers.iter() {
            let retain = (func)(hdr);
            if !retain {
                continue;
            }
            hdr.write_header(&mut new_data)?;
        }
        new_data.extend_from_slice(b"\r\n");
        new_data.extend_from_slice(&data[body_offset..]);
        self.assign_data(new_data);
        Ok(())
    }

    pub fn remove_first_named_header(&self, name: &str) -> anyhow::Result<()> {
        let mut removed = false;
        self.retain_headers(|hdr| {
            if hdr.get_name().eq_ignore_ascii_case(name) && !removed {
                removed = true;
                false
            } else {
                true
            }
        })
    }

    pub fn import_x_headers(&self, names: Vec<String>) -> anyhow::Result<()> {
        let data = self.get_data();
        let HeaderParseResult { headers, .. } = Header::parse_headers(data.as_ref().as_ref())?;

        for hdr in headers.iter() {
            let do_import = if names.is_empty() {
                is_x_header(hdr.get_name())
            } else {
                is_header_in_names_list(hdr.get_name(), &names)
            };
            if do_import {
                let name = imported_header_name(hdr.get_name());
                self.set_meta(name, hdr.as_unstructured()?)?;
            }
        }

        Ok(())
    }

    pub fn remove_x_headers(&self, names: Vec<String>) -> anyhow::Result<()> {
        self.retain_headers(|hdr| {
            if names.is_empty() {
                !is_x_header(hdr.get_name())
            } else {
                !is_header_in_names_list(hdr.get_name(), &names)
            }
        })
    }

    pub fn remove_all_named_headers(&self, name: &str) -> anyhow::Result<()> {
        self.retain_headers(|hdr| !hdr.get_name().eq_ignore_ascii_case(name))
    }

    #[cfg(feature = "impl")]
    pub async fn dkim_sign(&self, signer: Signer) -> anyhow::Result<()> {
        if let Some(runtime) = SIGN_POOL.get() {
            let msg = self.clone();
            runtime
                .spawn_blocking(move || {
                    let data = msg.get_data();
                    let header = signer.sign(&data)?;
                    msg.prepend_header(None, &header);
                    Ok::<(), anyhow::Error>(())
                })
                .await??;
        } else {
            let data = self.get_data();
            let header = signer.sign(&data)?;
            self.prepend_header(None, &header);
        }
        Ok(())
    }

    pub fn import_scheduling_header(
        &self,
        header_name: &str,
        remove: bool,
    ) -> anyhow::Result<Option<Scheduling>> {
        if let Some(value) = self.get_first_named_header_value(header_name)? {
            let sched: Scheduling = serde_json::from_str(&value).with_context(|| {
                format!("{value} from header {header_name} is not a valid Scheduling header")
            })?;
            let result = self.set_scheduling(Some(sched))?;

            if remove {
                self.remove_all_named_headers(header_name)?;
            }

            Ok(result)
        } else {
            Ok(None)
        }
    }

    pub fn append_text_plain(&self, content: &str) -> anyhow::Result<bool> {
        let data = self.get_data();
        let mut msg = MimePart::parse(data.as_ref().as_ref())?;
        let parts = msg.simplified_structure_pointers()?;
        if let Some(p) = parts.text_part.and_then(|p| msg.resolve_ptr_mut(p)) {
            match p.body()? {
                DecodedBody::Text(text) => {
                    let mut text = text.as_str().to_string();
                    text.push_str("\r\n");
                    text.push_str(content);
                    p.replace_text_body("text/plain", &text);

                    let new_data = msg.to_message_string();
                    self.assign_data(new_data.into_bytes());
                    Ok(true)
                }
                DecodedBody::Binary(_) => {
                    anyhow::bail!("expected text/plain part to be text, but it is binary");
                }
            }
        } else {
            Ok(false)
        }
    }

    pub fn append_text_html(&self, content: &str) -> anyhow::Result<bool> {
        let data = self.get_data();
        let mut msg = MimePart::parse(data.as_ref().as_ref())?;
        let parts = msg.simplified_structure_pointers()?;
        if let Some(p) = parts.html_part.and_then(|p| msg.resolve_ptr_mut(p)) {
            match p.body()? {
                DecodedBody::Text(text) => {
                    let mut text = text.as_str().to_string();

                    match text.rfind("</body>").or_else(|| text.rfind("</BODY>")) {
                        Some(idx) => {
                            text.insert_str(idx, content);
                            text.insert_str(idx, "\r\n");
                        }
                        None => {
                            // Just append
                            text.push_str("\r\n");
                            text.push_str(content);
                        }
                    }

                    p.replace_text_body("text/html", &text);

                    let new_data = msg.to_message_string();
                    self.assign_data(new_data.into_bytes());
                    Ok(true)
                }
                DecodedBody::Binary(_) => {
                    anyhow::bail!("expected text/html part to be text, but it is binary");
                }
            }
        } else {
            Ok(false)
        }
    }

    pub fn check_fix_conformance(
        &self,
        check: MessageConformance,
        fix: MessageConformance,
    ) -> anyhow::Result<()> {
        let data = self.get_data();
        let mut msg = MimePart::parse(data.as_ref().as_ref())?;

        let conformance = msg.conformance();

        // Don't raise errors for things that we're going to fix anyway
        let check = check - fix;

        if check.intersects(conformance) {
            let problems = check.intersection(conformance).to_string();
            anyhow::bail!("Message has conformance issues: {problems}");
        }

        if fix.intersects(conformance) {
            let to_fix = fix.intersection(conformance);
            let problems = to_fix.to_string();

            let missing_headers_only = to_fix
                .difference(
                    MessageConformance::MISSING_DATE_HEADER
                        | MessageConformance::MISSING_MIME_VERSION
                        | MessageConformance::MISSING_MESSAGE_ID_HEADER,
                )
                .is_empty();

            if !missing_headers_only {
                msg = msg.rebuild().with_context(|| {
                    format!("Rebuilding message to correct conformance issues: {problems}")
                })?;
            }

            if to_fix.contains(MessageConformance::MISSING_DATE_HEADER) {
                msg.headers_mut().set_date(Utc::now());
            }

            if to_fix.contains(MessageConformance::MISSING_MIME_VERSION) {
                msg.headers_mut().set_mime_version("1.0");
            }

            if to_fix.contains(MessageConformance::MISSING_MESSAGE_ID_HEADER) {
                let sender = self.sender()?;
                let domain = sender.domain();
                let id = *self.id();
                msg.headers_mut()
                    .set_message_id(mailparsing::MessageID(format!("{id}@{domain}")));
            }

            let new_data = msg.to_message_string();
            self.assign_data(new_data.into_bytes());
        }

        Ok(())
    }
}

fn is_header_in_names_list(hdr_name: &str, names: &[String]) -> bool {
    for name in names {
        if hdr_name.eq_ignore_ascii_case(name) {
            return true;
        }
    }
    false
}

fn imported_header_name(name: &str) -> String {
    name.chars()
        .map(|c| match c.to_ascii_lowercase() {
            '-' => '_',
            c => c,
        })
        .collect()
}

fn is_x_header(name: &str) -> bool {
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

#[cfg(feature = "impl")]
impl UserData for Message {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method(
            "set_meta",
            move |_, this, (name, value): (String, mlua::Value)| {
                let value = serde_json::value::to_value(value).map_err(any_err)?;
                this.set_meta(name, value).map_err(any_err)?;
                Ok(())
            },
        );
        methods.add_method("get_meta", move |lua, this, name: String| {
            let value = this.get_meta(name).map_err(any_err)?;
            Ok(Some(lua.to_value_with(&value, serialize_options())?))
        });
        methods.add_method("get_data", move |lua, this, _: ()| {
            let data = this.get_data();
            lua.create_string(&*data)
        });
        methods.add_method("set_data", move |_lua, this, data: mlua::String| {
            this.assign_data(data.as_bytes().to_vec());
            Ok(())
        });

        methods.add_method("append_text_plain", move |_lua, this, data: String| {
            this.append_text_plain(&data).map_err(any_err)
        });

        methods.add_method("append_text_html", move |_lua, this, data: String| {
            this.append_text_html(&data).map_err(any_err)
        });

        methods.add_method("id", move |_, this, _: ()| Ok(this.id().to_string()));
        methods.add_method("sender", move |_, this, _: ()| {
            Ok(this.sender().map_err(any_err)?)
        });

        methods.add_method("num_attempts", move |_, this, _: ()| {
            Ok(this.get_num_attempts())
        });

        methods.add_method("queue_name", move |_, this, _: ()| {
            Ok(this.get_queue_name().map_err(any_err)?)
        });

        methods.add_async_method("set_due", move |lua, this, due: mlua::Value| async move {
            let due: Option<DateTime<Utc>> = lua.from_value(due)?;
            let revised_due = this.set_due(due).await.map_err(any_err)?;
            lua.to_value(&revised_due)
        });

        methods.add_method("set_sender", move |lua, this, value: mlua::Value| {
            let sender = match value {
                mlua::Value::String(s) => {
                    let s = s.to_str()?;
                    EnvelopeAddress::parse(&s).map_err(any_err)?
                }
                _ => lua.from_value::<EnvelopeAddress>(value.clone())?,
            };
            Ok(this.set_sender(sender).map_err(any_err)?)
        });

        methods.add_method("recipient", move |_, this, _: ()| {
            Ok(this.recipient().map_err(any_err)?)
        });

        methods.add_method("set_recipient", move |lua, this, value: mlua::Value| {
            let recipient = match value {
                mlua::Value::String(s) => {
                    let s = s.to_str()?;
                    EnvelopeAddress::parse(&s).map_err(any_err)?
                }
                _ => lua.from_value::<EnvelopeAddress>(value.clone())?,
            };
            Ok(this.set_recipient(recipient).map_err(any_err)?)
        });

        #[cfg(feature = "impl")]
        methods.add_async_method("dkim_sign", |_, this, signer: Signer| async move {
            Ok(this.dkim_sign(signer).await.map_err(any_err)?)
        });

        methods.add_async_method("shrink", |_, this, _: ()| async move {
            if this.needs_save() {
                this.save(None).await.map_err(any_err)?;
            }
            this.shrink().map_err(any_err)
        });

        methods.add_async_method("shrink_data", |_, this, _: ()| async move {
            if this.needs_save() {
                this.save(None).await.map_err(any_err)?;
            }
            this.shrink_data().map_err(any_err)
        });

        methods.add_method(
            "add_authentication_results",
            move |lua, this, (serv_id, results): (String, mlua::Value)| {
                let results: Vec<AuthenticationResult> = lua.from_value(results)?;
                let results = AuthenticationResults {
                    serv_id,
                    version: None,
                    results,
                };

                this.prepend_header(Some("Authentication-Results"), &results.encode_value());

                Ok(())
            },
        );

        #[cfg(feature = "impl")]
        methods.add_async_method("dkim_verify", |lua, this, ()| async move {
            let results = this.dkim_verify().await.map_err(any_err)?;
            lua.to_value_with(&results, serialize_options())
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
        methods.add_method("get_address_header", move |_, this, name: String| {
            Ok(this.get_address_header(&name).map_err(any_err)?)
        });
        methods.add_method("from_header", move |_, this, ()| {
            Ok(this.get_address_header("From").map_err(any_err)?)
        });
        methods.add_method("to_header", move |_, this, ()| {
            Ok(this.get_address_header("To").map_err(any_err)?)
        });

        methods.add_method(
            "get_first_named_header_value",
            move |_, this, name: String| {
                Ok(this.get_first_named_header_value(&name).map_err(any_err)?)
            },
        );
        methods.add_method(
            "get_all_named_header_values",
            move |_, this, name: String| {
                Ok(this.get_all_named_header_values(&name).map_err(any_err)?)
            },
        );
        methods.add_method("get_all_headers", move |_, this, _: ()| {
            Ok(this
                .get_all_headers()
                .map_err(any_err)?
                .into_iter()
                .map(|(name, value)| vec![name, value])
                .collect::<Vec<Vec<String>>>())
        });
        methods.add_method("get_all_headers", move |_, this, _: ()| {
            Ok(this
                .get_all_headers()
                .map_err(any_err)?
                .into_iter()
                .map(|(name, value)| vec![name, value])
                .collect::<Vec<Vec<String>>>())
        });
        methods.add_method(
            "import_x_headers",
            move |_, this, names: Option<Vec<String>>| {
                Ok(this
                    .import_x_headers(names.unwrap_or_else(|| vec![]))
                    .map_err(any_err)?)
            },
        );

        methods.add_method(
            "remove_x_headers",
            move |_, this, names: Option<Vec<String>>| {
                Ok(this
                    .remove_x_headers(names.unwrap_or_else(|| vec![]))
                    .map_err(any_err)?)
            },
        );
        methods.add_method("remove_all_named_headers", move |_, this, name: String| {
            Ok(this.remove_all_named_headers(&name).map_err(any_err)?)
        });

        methods.add_method(
            "import_scheduling_header",
            move |lua, this, (header_name, remove): (String, bool)| {
                let opt_schedule = this
                    .import_scheduling_header(&header_name, remove)
                    .map_err(any_err)?;
                lua.to_value(&opt_schedule)
            },
        );

        methods.add_method("set_scheduling", move |lua, this, params: mlua::Value| {
            let sched: Option<Scheduling> = from_lua_value(lua, params)?;
            let opt_schedule = this.set_scheduling(sched).map_err(any_err)?;
            lua.to_value(&opt_schedule)
        });

        methods.add_method("parse_rfc3464", move |lua, this, _: ()| {
            let report = this.parse_rfc3464().map_err(any_err)?;
            match report {
                Some(report) => lua.to_value_with(&report, serialize_options()),
                None => Ok(mlua::Value::Nil),
            }
        });

        methods.add_method("parse_rfc5965", move |lua, this, _: ()| {
            let report = this.parse_rfc5965().map_err(any_err)?;
            match report {
                Some(report) => lua.to_value_with(&report, serialize_options()),
                None => Ok(mlua::Value::Nil),
            }
        });

        methods.add_async_method("save", |_, this, ()| async move {
            this.save(None).await.map_err(any_err)
        });

        methods.add_method("set_force_sync", move |_, this, force: bool| {
            this.set_force_sync(force);
            Ok(())
        });

        methods.add_async_method(
            "check_fix_conformance",
            |_, this, (check, fix): (String, String)| async move {
                use std::str::FromStr;
                let check = MessageConformance::from_str(&check).map_err(any_err)?;
                let fix = MessageConformance::from_str(&fix).map_err(any_err)?;

                match this.check_fix_conformance(check, fix) {
                    Ok(_) => Ok(None),
                    Err(err) => Ok(Some(format!("{err:#}"))),
                }
            },
        );
    }
}

impl TimerEntryWithDelay for WeakMessage {
    fn delay(&self) -> Duration {
        match self.upgrade() {
            None => {
                // Dangling/Cancelled. Make it appear due immediately
                Duration::from_millis(0)
            }
            Some(msg) => msg.delay(),
        }
    }
}

impl TimerEntryWithDelay for Message {
    fn delay(&self) -> Duration {
        let inner = self.msg_and_id.inner.lock().unwrap();
        match inner.due {
            Some(time) => {
                let now = Utc::now();
                let delta = time - now;
                delta.to_std().unwrap_or(Duration::from_millis(0))
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
    fn meta_and_nil() {
        let msg = new_msg_body(X_HDR_CONTENT);
        // Ensure that json null round-trips
        msg.set_meta("test", serde_json::Value::Null).unwrap();
        k9::assert_equal!(msg.get_meta("test").unwrap(), serde_json::Value::Null);

        // and that it is exposed to lua as nil
        let lua = mlua::Lua::new();
        lua.globals().set("msg", msg).unwrap();
        lua.load("assert(msg:get_meta('test') == nil)")
            .exec()
            .unwrap();
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
            "Subject: Hello\r\nFrom :Someone\r\n\r\nBody"
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
        "X-Hello: there\r\nX-Header: value\r\nSubject: Hello\r\nX-Header: another value\r\nFrom :Someone@somewhere\r\n\r\nBody";

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
            "X-Hello: there\r\nSubject: Hello\r\nX-Header: another value\r\nFrom :Someone@somewhere\r\n\r\nBody"
        );
    }

    #[test]
    fn remove_all() {
        let msg = new_msg_body(MULTI_HEADER_CONTENT);
        msg.remove_all_named_headers("X-header").unwrap();
        k9::assert_equal!(
            data_as_string(&msg),
            "X-Hello: there\r\nSubject: Hello\r\nFrom :Someone@somewhere\r\n\r\nBody"
        );
    }

    #[test]
    fn append_text_plain() {
        let msg = new_msg_body(MULTI_HEADER_CONTENT);
        msg.append_text_plain("I am at the bottom").unwrap();
        k9::assert_equal!(
            data_as_string(&msg),
            "X-Hello: there\r\n\
             X-Header: value\r\n\
             Subject: Hello\r\n\
             X-Header: another value\r\n\
             From :Someone@somewhere\r\n\
             Content-Type: text/plain;\r\n\
             \tcharset=\"us-ascii\"\r\n\
             \r\n\
             Body\r\n\
             I am at the bottom\r\n"
        );
    }

    const MIXED_CONTENT: &str = "Content-Type: multipart/mixed;\r\n\
\tboundary=\"my-boundary\"\r\n\
\r\n\
--my-boundary\r\n\
Content-Type: text/plain;\r\n\
\tcharset=\"us-ascii\"\r\n\
\r\n\
plain text\r\n\
--my-boundary\r\n\
Content-Type: text/html;\r\n\
\tcharset=\"us-ascii\"\r\n\
\r\n\
<b>rich</b> text\r\n\
--my-boundary\r\n\
Content-Type: application/octet-stream\r\n\
Content-Transfer-Encoding: base64\r\n\
Content-Disposition: attachment;\r\n\
\tfilename=\"woot.bin\"\r\n\
Content-ID: <woot.id@somewhere>\r\n\
\r\n\
AAECAw==\r\n\
--my-boundary--\r\n\
\r\n";

    const MIXED_CONTENT_ENCLOSING_BODY: &str = "Content-Type: multipart/mixed;\r\n\
\tboundary=\"my-boundary\"\r\n\
\r\n\
--my-boundary\r\n\
Content-Type: text/plain;\r\n\
\tcharset=\"us-ascii\"\r\n\
\r\n\
plain text\r\n\
--my-boundary\r\n\
Content-Type: text/html;\r\n\
\tcharset=\"us-ascii\"\r\n\
\r\n\
<BODY>\r\n\
<b>rich</b> text\r\n\
</BODY>\r\n\
--my-boundary\r\n\
Content-Type: application/octet-stream\r\n\
Content-Transfer-Encoding: base64\r\n\
Content-Disposition: attachment;\r\n\
\tfilename=\"woot.bin\"\r\n\
Content-ID: <woot.id>\r\n\
\r\n\
AAECAw==\r\n\
--my-boundary--\r\n\
\r\n";

    #[test]
    fn append_text_html() {
        let msg = new_msg_body(MIXED_CONTENT);
        msg.append_text_html("bottom html").unwrap();
        k9::snapshot!(
            data_as_string(&msg),
            r#"
Content-Type: multipart/mixed;\r
\tboundary="my-boundary"\r
\r
--my-boundary\r
Content-Type: text/plain;\r
\tcharset="us-ascii"\r
\r
plain text\r
--my-boundary\r
Content-Type: text/html;\r
\tcharset="us-ascii"\r
\r
<b>rich</b> text\r
\r
bottom html\r
--my-boundary\r
Content-Type: application/octet-stream\r
Content-Transfer-Encoding: base64\r
Content-Disposition: attachment;\r
\tfilename="woot.bin"\r
Content-ID: <woot.id@somewhere>\r
\r
AAECAw==\r
--my-boundary--\r
\r

"#
        );

        let msg = new_msg_body(MIXED_CONTENT_ENCLOSING_BODY);
        msg.append_text_html("bottom html 👻").unwrap();
        k9::snapshot!(
            data_as_string(&msg),
            r#"
Content-Type: multipart/mixed;\r
\tboundary="my-boundary"\r
\r
--my-boundary\r
Content-Type: text/plain;\r
\tcharset="us-ascii"\r
\r
plain text\r
--my-boundary\r
Content-Type: text/html;\r
\tcharset="utf-8"\r
Content-Transfer-Encoding: quoted-printable\r
\r
<BODY>\r
<b>rich</b> text\r
\r
bottom html =F0=9F=91=BB</BODY>\r
--my-boundary\r
Content-Type: application/octet-stream\r
Content-Transfer-Encoding: base64\r
Content-Disposition: attachment;\r
\tfilename="woot.bin"\r
Content-ID: <woot.id>\r
\r
AAECAw==\r
--my-boundary--\r
\r

"#
        );
    }

    #[test]
    fn append_text_plain_mixed() {
        let msg = new_msg_body(MIXED_CONTENT);
        msg.append_text_plain("bottom text 👾").unwrap();
        k9::snapshot!(
            data_as_string(&msg),
            r#"
Content-Type: multipart/mixed;\r
\tboundary="my-boundary"\r
\r
--my-boundary\r
Content-Type: text/plain;\r
\tcharset="utf-8"\r
Content-Transfer-Encoding: quoted-printable\r
\r
plain text\r
\r
bottom text =F0=9F=91=BE\r
--my-boundary\r
Content-Type: text/html;\r
\tcharset="us-ascii"\r
\r
<b>rich</b> text\r
--my-boundary\r
Content-Type: application/octet-stream\r
Content-Transfer-Encoding: base64\r
Content-Disposition: attachment;\r
\tfilename="woot.bin"\r
Content-ID: <woot.id@somewhere>\r
\r
AAECAw==\r
--my-boundary--\r
\r

"#
        );
    }

    #[test]
    fn check_conformance_angle_msg_id() {
        const DOUBLE_ANGLE_ONLY: &str = "Subject: hello\r
Message-ID: <<1234@example.com>>\r
\r
Hello";
        let msg = new_msg_body(DOUBLE_ANGLE_ONLY);
        k9::snapshot!(
            msg.check_fix_conformance(
                MessageConformance::MISSING_MESSAGE_ID_HEADER,
                MessageConformance::empty(),
            )
            .unwrap_err(),
            "Message has conformance issues: MISSING_MESSAGE_ID_HEADER"
        );

        msg.check_fix_conformance(
            MessageConformance::MISSING_MESSAGE_ID_HEADER,
            MessageConformance::MISSING_MESSAGE_ID_HEADER,
        )
        .unwrap();

        // Can't use a snapshot test here because the fixed header
        // has a unique random component
        /*
                k9::snapshot!(
                    data_as_string(&msg),
                    r#"
        Subject: hello\r
        Message-ID: <4106566d2ce911ef9dcd0242289ea0df@example.com>\r
        \r
        Hello
        "#
                );
        */

        const DOUBLE_ANGLE_AND_LONG_LINE: &str = "Subject: hello\r
Message-ID: <<1234@example.com>>\r
\r
Hello this is a really long line Hello this is a really long line \
Hello this is a really long line Hello this is a really long line \
Hello this is a really long line Hello this is a really long line \
Hello this is a really long line Hello this is a really long line \
Hello this is a really long line Hello this is a really long line \
Hello this is a really long line Hello this is a really long line \
Hello this is a really long line Hello this is a really long line
";
        let msg = new_msg_body(DOUBLE_ANGLE_AND_LONG_LINE);
        msg.check_fix_conformance(
            MessageConformance::MISSING_COLON_VALUE,
            MessageConformance::MISSING_MESSAGE_ID_HEADER | MessageConformance::LINE_TOO_LONG,
        )
        .unwrap();

        // Can't use a snapshot test here because the fixed header
        // has a random component
        /*
                k9::snapshot!(
                    data_as_string(&msg),
                    r#"
        Content-Type: text/plain;\r
        \tcharset="us-ascii"\r
        Content-Transfer-Encoding: quoted-printable\r
        Subject: hello\r
        Message-ID: <749fc87e2cea11ef96a50242289ea0df@example.com>\r
        \r
        Hello this is a really long line Hello this is a really long line Hello thi=\r
        s is a really long line Hello this is a really long line Hello this is a re=\r
        ally long line Hello this is a really long line Hello this is a really long=\r
         line Hello this is a really long line Hello this is a really long line Hel=\r
        lo this is a really long line Hello this is a really long line Hello this i=\r
        s a really long line Hello this is a really long line Hello this is a reall=\r
        y long line=0A\r

        "#
                );
        */
    }

    #[test]
    fn check_conformance() {
        let msg = new_msg_body(MULTI_HEADER_CONTENT);
        msg.check_fix_conformance(
            MessageConformance::default(),
            MessageConformance::MISSING_MIME_VERSION,
        )
        .unwrap();
        k9::snapshot!(
            data_as_string(&msg),
            r#"
X-Hello: there\r
X-Header: value\r
Subject: Hello\r
X-Header: another value\r
From :Someone@somewhere\r
Mime-Version: 1.0\r
\r
Body
"#
        );

        let msg = new_msg_body(MULTI_HEADER_CONTENT);
        msg.check_fix_conformance(
            MessageConformance::default(),
            MessageConformance::MISSING_MIME_VERSION | MessageConformance::NAME_ENDS_WITH_SPACE,
        )
        .unwrap();
        k9::snapshot!(
            data_as_string(&msg),
            r#"
Content-Type: text/plain;\r
\tcharset="us-ascii"\r
X-Hello: there\r
X-Header: value\r
Subject: Hello\r
X-Header: another value\r
From: <Someone@somewhere>\r
Mime-Version: 1.0\r
\r
Body\r

"#
        );
    }

    #[test]
    fn set_scheduling() -> anyhow::Result<()> {
        let msg = new_msg_body(MULTI_HEADER_CONTENT);
        assert!(msg.get_due().is_none(), "due is implicitly now");

        let now = Utc::now();
        let one_day = chrono::Duration::try_days(1).expect("1 day to be valid");

        msg.set_scheduling(Some(Scheduling {
            restriction: None,
            first_attempt: Some((now + one_day).into()),
            expires: None,
        }))?;

        let due = msg.get_due().expect("due to now be set");
        assert!(due - now >= one_day, "due time is at least 1 day away");

        Ok(())
    }

    #[cfg(all(test, target_pointer_width = "64"))]
    #[test]
    fn sizes() {
        assert_eq!(std::mem::size_of::<Message>(), 8);
        assert_eq!(std::mem::size_of::<MessageInner>(), 32);
    }
}
