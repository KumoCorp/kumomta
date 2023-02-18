use crate::EnvelopeAddress;
use chrono::{DateTime, Utc};
use mlua::{LuaSerdeExt, UserData, UserDataMethods};
use serde::{Deserialize, Serialize};
use spool::{Spool, SpoolId};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use timeq::{CancellableTimerEntry, TimerEntryWithDelay};

bitflags::bitflags! {
    struct MessageFlags: u32 {
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
    #[serde(with = "chrono::serde::ts_seconds")]
    created: DateTime<Utc>,
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
                    created: Utc::now(),
                }),
                data,
                flags: MessageFlags::META_DIRTY | MessageFlags::DATA_DIRTY,
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
                due: None,
            })),
        })
    }

    pub fn get_due(&self) -> Option<DateTime<Utc>> {
        let inner = self.inner.lock().unwrap();
        inner.due
    }

    pub fn delay_by(&self, duration: Duration) {
        let due = Utc::now() + chrono::Duration::milliseconds(duration.as_millis() as _);
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

    pub async fn load_meta(&mut self, meta_spool: impl Spool) -> anyhow::Result<()> {
        let id = self.id();
        let data = meta_spool.load(*id).await?;
        let mut inner = self.inner.lock().unwrap();
        let metadata: MetaData = serde_json::from_slice(&data)?;
        inner.metadata.replace(metadata);
        Ok(())
    }

    pub async fn load_data(&mut self, data_spool: impl Spool) -> anyhow::Result<()> {
        let data = data_spool.load(*self.id()).await?;
        let mut inner = self.inner.lock().unwrap();
        inner.data = Arc::new(data.into_boxed_slice());
        Ok(())
    }

    pub fn assign_data(&mut self, data: Vec<u8>) {
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
