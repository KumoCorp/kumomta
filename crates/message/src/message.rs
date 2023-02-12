use crate::EnvelopeAddress;
use serde::{Deserialize, Serialize};
use spool::{Spool, SpoolId};

bitflags::bitflags! {
    struct MessageFlags: u32 {
        /// true if Metadata needs to be saved
        const META_DIRTY = 1;
        /// true if Data needs to be saved
        const DATA_DIRTY = 2;
    }
}

pub struct Message {
    id: SpoolId,
    metadata: Option<MetaData>,
    data: Vec<u8>,
    flags: MessageFlags,
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
            id: SpoolId::new(),
            metadata: None,
            data: vec![],
            flags: MessageFlags::META_DIRTY | MessageFlags::DATA_DIRTY,
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
            id: SpoolId::new(),
            metadata: Some(MetaData {
                sender,
                recipient,
                meta,
            }),
            data,
            flags: MessageFlags::META_DIRTY | MessageFlags::DATA_DIRTY,
        })
    }

    /// Helper for creating a message based on spool enumeration.
    /// Given a spool id and the serialized metadata blob, returns
    /// a message holding the deserialized version of that metadata.
    pub fn new_from_spool(id: SpoolId, metadata: Vec<u8>) -> anyhow::Result<Self> {
        let metadata: MetaData = serde_json::from_slice(&metadata)?;

        Ok(Self {
            id,
            metadata: Some(metadata),
            data: vec![],
            flags: MessageFlags::empty(),
        })
    }

    pub async fn save_to(
        &mut self,
        meta_spool: impl Spool,
        data_spool: impl Spool,
    ) -> anyhow::Result<()> {
        if self.flags.contains(MessageFlags::DATA_DIRTY) {
            anyhow::ensure!(!self.data.is_empty(), "message data must not be empty");
            data_spool.store(self.id, &self.data).await?;
            self.flags.remove(MessageFlags::DATA_DIRTY);
        }
        if self.flags.contains(MessageFlags::META_DIRTY) {
            match &self.metadata {
                None => anyhow::bail!("message metadata is both missing and dirty"),
                Some(meta) => {
                    let data = serde_json::to_vec(meta)?;
                    meta_spool.store(self.id, &data).await?;
                    self.flags.remove(MessageFlags::META_DIRTY);
                }
            }
        }
        Ok(())
    }

    pub fn shrink(&mut self) -> anyhow::Result<()> {
        if self.flags.contains(MessageFlags::DATA_DIRTY) {
            anyhow::bail!("Cannot shrink message: DATA_DIRTY");
        }
        if self.flags.contains(MessageFlags::META_DIRTY) {
            anyhow::bail!("Cannot shrink message: META_DIRTY");
        }
        self.metadata.take();
        self.data = vec![];
        Ok(())
    }

    pub fn sender(&self) -> anyhow::Result<EnvelopeAddress> {
        match &self.metadata {
            Some(meta) => Ok(meta.sender.clone()),
            None => anyhow::bail!("metadata is not loaded"),
        }
    }

    pub fn recipient(&self) -> anyhow::Result<EnvelopeAddress> {
        match &self.metadata {
            Some(meta) => Ok(meta.recipient.clone()),
            None => anyhow::bail!("metadata is not loaded"),
        }
    }

    pub async fn load_meta(&mut self, meta_spool: impl Spool) -> anyhow::Result<()> {
        let data = meta_spool.load(self.id).await?;
        let metadata: MetaData = serde_json::from_slice(&data)?;
        self.metadata.replace(metadata);
        Ok(())
    }

    pub async fn load_data(&mut self, data_spool: impl Spool) -> anyhow::Result<()> {
        self.data = data_spool.load(self.id).await?;
        Ok(())
    }

    pub fn assign_data(&mut self, data: Vec<u8>) {
        self.data = data;
        self.flags.set(MessageFlags::DATA_DIRTY, true);
    }

    pub fn set_meta<S: AsRef<str>, V: Into<serde_json::Value>>(
        &mut self,
        key: S,
        value: V,
    ) -> anyhow::Result<()> {
        match &mut self.metadata {
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

                self.flags.set(MessageFlags::META_DIRTY, true);
                Ok(())
            }
        }
    }

    pub fn get_meta<S: serde_json::value::Index>(
        &self,
        key: S,
    ) -> anyhow::Result<serde_json::Value> {
        match &self.metadata {
            None => anyhow::bail!("metadata must be loaded first"),
            Some(meta) => match meta.meta.get(key) {
                Some(value) => Ok(value.clone()),
                None => Ok(serde_json::Value::Null),
            },
        }
    }
}
