use crate::{EnvelopeAddress, Message};
use serde::{Deserialize, Serialize};
use spool::SpoolId;

/// This is a wire format, so changes need to
/// be appropriately backwards/forwards compatible
/// or placed into a separate struct with runtime
/// handling for version mismatches.
#[derive(Debug, Serialize, Deserialize, Clone)]
struct MetaDataV1 {
    id: SpoolId,
    sender: EnvelopeAddress,
    recipient: Vec<EnvelopeAddress>,
    meta: serde_json::Value,
}

impl Message {
    pub async fn serialize_for_xfer(
        &self,
        additional_meta: serde_json::Value,
    ) -> anyhow::Result<Vec<u8>> {
        self.load_data_if_needed().await?;
        let id = *self.id();
        let data = self.get_data();
        let mut meta = self.clone_meta_data().await?;

        if let serde_json::Value::Object(src) = additional_meta {
            if let Some(obj) = meta.meta.as_object_mut() {
                for (k, v) in src {
                    obj.insert(k, v);
                }
            }
        }

        let meta = MetaDataV1 {
            id,
            sender: meta.sender,
            recipient: meta.recipient,
            meta: meta.meta,
        };

        let serialized_meta = serde_json::to_string(&meta)?;

        let mut result: Vec<u8> = serialized_meta.into();
        result.push(b'\n');
        result.extend_from_slice(&data);

        Ok(result)
    }

    pub fn deserialize_from_xfer(serialized: &[u8]) -> anyhow::Result<Self> {
        let newline = memchr::memchr(b'\n', serialized)
            .ok_or_else(|| anyhow::anyhow!("invalid xfer payload"))?;

        let (meta_json, data) = serialized.split_at(newline);

        let meta: MetaDataV1 = serde_json::from_slice(&meta_json)?;
        // 1... because split_at includes the newline at the startgg
        let payload: Box<[u8]> = data[1..].to_vec().into_boxed_slice();

        let metadata = crate::message::MetaData {
            sender: meta.sender,
            recipient: meta.recipient,
            meta: meta.meta,
            schedule: None,
        };

        // Create a new id with *this* nodes mac but the source
        // node's timestamp.  This should reduce the chances
        // of a conflict leading to multiple messages alive in
        // the same process with the same id, while preserving
        // the timestamp of the source message.
        // Ideally we would check that the ids are not the same
        // here and raise an error, but for the sake of testing,
        // we allow whatever value is produced to be used and
        // we check that in the tests, and we SHOULD also
        // check this in the xfer logic and have it
        let id = meta.id.derive_new_with_cloned_timestamp();

        Ok(Self::new_from_parts(id, metadata, payload.into()))
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::message::test::new_msg_body;
    use serde_json::json;

    #[tokio::test]
    async fn xfer_serialization() {
        let msg = new_msg_body("Subject: simple message\r\n\r\nHello\r\n");
        msg.set_meta("canary", true).unwrap();
        let serialized = msg
            .serialize_for_xfer(json!({"additional": "meta"}))
            .await
            .unwrap();
        eprintln!("serialized as: {}", String::from_utf8_lossy(&serialized));

        let round_trip = Message::deserialize_from_xfer(&serialized).unwrap();
        assert_eq!(round_trip.get_meta("canary").unwrap(), true);
        assert_eq!(round_trip.get_meta("additional").unwrap(), "meta");
        eprintln!(
            "deserialized message:\n{}",
            String::from_utf8_lossy(&round_trip.get_data())
        );

        eprintln!("id           ={}", msg.id());
        eprintln!("round_trip id={}", round_trip.id());
        assert_ne!(msg.id(), round_trip.id());

        assert_eq!(
            round_trip
                .get_first_named_header_value("subject")
                .unwrap()
                .unwrap(),
            "simple message"
        );
    }
}
