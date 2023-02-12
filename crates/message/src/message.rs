use crate::EnvelopeAddress;
use mlua::{LuaSerdeExt, UserData, UserDataMethods};
use serde::{Deserialize, Serialize};
use spool::{Spool, SpoolId};
use std::sync::Arc;
use tokio::sync::Mutex;

bitflags::bitflags! {
    struct MessageFlags: u32 {
        /// true if Metadata needs to be saved
        const META_DIRTY = 1;
        /// true if Data needs to be saved
        const DATA_DIRTY = 2;
    }
}

struct MessageInner {
    id: SpoolId,
    metadata: Option<MetaData>,
    data: Vec<u8>,
    flags: MessageFlags,
}

#[derive(Clone)]
pub struct Message {
    inner: Arc<Mutex<MessageInner>>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct MetaData {
    sender: EnvelopeAddress,
    recipient: EnvelopeAddress,
    meta: serde_json::Value,
}

impl Message {
    /// Create a new message consisting solely of an id
    pub fn new_empty() -> Self {
        Self {
            inner: Arc::new(Mutex::new(MessageInner {
                id: SpoolId::new(),
                metadata: None,
                data: vec![],
                flags: MessageFlags::META_DIRTY | MessageFlags::DATA_DIRTY,
            })),
        }
    }

    /// Create a new message with the supplied data.
    /// The message meta and data are marked as dirty
    pub fn new_dirty(
        sender: EnvelopeAddress,
        recipient: EnvelopeAddress,
        meta: serde_json::Value,
        data: Vec<u8>,
    ) -> anyhow::Result<Self> {
        anyhow::ensure!(meta.is_object(), "metadata must be a json object");
        Ok(Self {
            inner: Arc::new(Mutex::new(MessageInner {
                id: SpoolId::new(),
                metadata: Some(MetaData {
                    sender,
                    recipient,
                    meta,
                }),
                data,
                flags: MessageFlags::META_DIRTY | MessageFlags::DATA_DIRTY,
            })),
        })
    }

    /// Helper for creating a message based on spool enumeration.
    /// Given a spool id and the serialized metadata blob, returns
    /// a message holding the deserialized version of that metadata.
    pub fn new_from_spool(id: SpoolId, metadata: Vec<u8>) -> anyhow::Result<Self> {
        let metadata: MetaData = serde_json::from_slice(&metadata)?;

        Ok(Self {
            inner: Arc::new(Mutex::new(MessageInner {
                id,
                metadata: Some(metadata),
                data: vec![],
                flags: MessageFlags::empty(),
            })),
        })
    }

    pub async fn save_to(
        &self,
        meta_spool: impl Spool,
        data_spool: impl Spool,
    ) -> anyhow::Result<()> {
        let mut inner = self.inner.lock().await;
        if inner.flags.contains(MessageFlags::DATA_DIRTY) {
            anyhow::ensure!(!inner.data.is_empty(), "message data must not be empty");
            data_spool.store(inner.id, &inner.data).await?;
            inner.flags.remove(MessageFlags::DATA_DIRTY);
        }
        if inner.flags.contains(MessageFlags::META_DIRTY) {
            match &inner.metadata {
                None => anyhow::bail!("message metadata is both missing and dirty"),
                Some(meta) => {
                    let data = serde_json::to_vec(meta)?;
                    meta_spool.store(inner.id, &data).await?;
                    inner.flags.remove(MessageFlags::META_DIRTY);
                }
            }
        }
        Ok(())
    }

    pub async fn id(&self) -> SpoolId {
        self.inner.lock().await.id
    }

    pub async fn shrink(&self) -> anyhow::Result<()> {
        let mut inner = self.inner.lock().await;
        if inner.flags.contains(MessageFlags::DATA_DIRTY) {
            anyhow::bail!("Cannot shrink message: DATA_DIRTY");
        }
        if inner.flags.contains(MessageFlags::META_DIRTY) {
            anyhow::bail!("Cannot shrink message: META_DIRTY");
        }
        inner.metadata.take();
        inner.data = vec![];
        Ok(())
    }

    pub async fn sender(&self) -> anyhow::Result<EnvelopeAddress> {
        let inner = self.inner.lock().await;
        match &inner.metadata {
            Some(meta) => Ok(meta.sender.clone()),
            None => anyhow::bail!("metadata is not loaded"),
        }
    }

    pub async fn recipient(&self) -> anyhow::Result<EnvelopeAddress> {
        let inner = self.inner.lock().await;
        match &inner.metadata {
            Some(meta) => Ok(meta.recipient.clone()),
            None => anyhow::bail!("metadata is not loaded"),
        }
    }

    pub async fn load_meta(&mut self, meta_spool: impl Spool) -> anyhow::Result<()> {
        let mut inner = self.inner.lock().await;
        let data = meta_spool.load(inner.id).await?;
        let metadata: MetaData = serde_json::from_slice(&data)?;
        inner.metadata.replace(metadata);
        Ok(())
    }

    pub async fn load_data(&mut self, data_spool: impl Spool) -> anyhow::Result<()> {
        let mut inner = self.inner.lock().await;
        inner.data = data_spool.load(inner.id).await?;
        Ok(())
    }

    pub async fn assign_data(&mut self, data: Vec<u8>) {
        let mut inner = self.inner.lock().await;
        inner.data = data;
        inner.flags.set(MessageFlags::DATA_DIRTY, true);
    }

    pub async fn set_meta<S: AsRef<str>, V: Into<serde_json::Value>>(
        &self,
        key: S,
        value: V,
    ) -> anyhow::Result<()> {
        let mut inner = self.inner.lock().await;
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

    pub async fn get_meta<S: serde_json::value::Index>(
        &self,
        key: S,
    ) -> anyhow::Result<serde_json::Value> {
        let inner = self.inner.lock().await;
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
        methods.add_async_method(
            "set_meta",
            |_, this, (name, value): (String, mlua::Value)| async move {
                let value = serde_json::value::to_value(value)
                    .map_err(|err| mlua::Error::external(format!("{err:#}")))?;
                this.set_meta(name, value)
                    .await
                    .map_err(|err| mlua::Error::external(format!("{err:#}")))?;
                Ok(())
            },
        );
        methods.add_async_method("get_meta", |lua, this, name: String| async move {
            let value = this
                .get_meta(name)
                .await
                .map_err(|err| mlua::Error::external(format!("{err:#}")))?;
            Ok(Some(lua.to_value(&value)?))
        });
        methods.add_async_method("id", |_, this, _: ()| async move {
            Ok(this.id().await.to_string())
        });
        methods.add_async_method("sender", |_, this, _: ()| async move {
            Ok(this
                .sender()
                .await
                .map_err(|err| mlua::Error::external(format!("{err:#}")))?)
        });
        methods.add_async_method("recipient", |_, this, _: ()| async move {
            Ok(this
                .recipient()
                .await
                .map_err(|err| mlua::Error::external(format!("{err:#}")))?)
        });
    }
}
