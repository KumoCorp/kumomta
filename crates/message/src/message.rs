use crate::EnvelopeAddress;
use chrono::{DateTime, Utc};
use mlua::{LuaSerdeExt, UserData, UserDataMethods};
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

impl Message {
    /// Create a new message with the supplied data.
    /// The message meta and data are marked as dirty
    pub fn new_dirty(
        sender: EnvelopeAddress,
        recipient: EnvelopeAddress,
        meta: serde_json::Value,
        data: Arc<Box<[u8]>>,
    ) -> anyhow::Result<Self> {
        anyhow::ensure!(meta.is_object(), "metadata must be a json object");
        let id = SpoolId::new();
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
        println!("delaying by {value}");
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
        if let Some(data) = self.get_data_if_dirty() {
            anyhow::ensure!(!data.is_empty(), "message data must not be empty");
            data_spool.store(self.id, &data).await?;
            self.inner
                .lock()
                .unwrap()
                .flags
                .remove(MessageFlags::DATA_DIRTY);
        }
        if let Some(meta) = self.get_meta_if_dirty() {
            let data = serde_json::to_vec(&meta)?;
            meta_spool.store(self.id, &data).await?;
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
        inner.metadata.take();
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
        let metadata: MetaData = serde_json::from_slice(&data)?;
        inner.metadata.replace(metadata);
        Ok(())
    }

    pub async fn load_data(&self, data_spool: &(dyn Spool + Send + Sync)) -> anyhow::Result<()> {
        let data = data_spool.load(*self.id()).await?;
        let mut inner = self.inner.lock().unwrap();
        inner.data = Arc::new(data.into_boxed_slice());
        Ok(())
    }

    pub fn assign_data(&self, data: Vec<u8>) {
        let mut inner = self.inner.lock().unwrap();
        inner.data = Arc::new(data.into_boxed_slice());
        inner.flags.set(MessageFlags::DATA_DIRTY, true);
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
