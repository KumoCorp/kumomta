use config::{any_err, get_or_create_sub_module};
use futures::stream::FuturesOrdered;
use futures::StreamExt;
use mlua::prelude::LuaUserData;
use mlua::{Lua, LuaSerdeExt, UserDataMethods, Value};
use rdkafka::message::{Header, OwnedHeaders};
use rdkafka::producer::{FutureProducer, FutureRecord};
use rdkafka::util::Timeout;
use rdkafka::ClientConfig;
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Clone)]
struct Producer {
    producer: Arc<Mutex<Option<Arc<FutureProducer>>>>,
}

impl Producer {
    fn get_producer(&self) -> mlua::Result<Arc<FutureProducer>> {
        self.producer
            .lock()
            .unwrap()
            .as_ref()
            .map(Arc::clone)
            .ok_or_else(|| mlua::Error::external("client was closed"))
    }
}

#[derive(Deserialize, Debug)]
struct Record {
    /// Required destination topic
    topic: String,
    /// Optional destination partition
    #[serde(default)]
    partition: Option<i32>,
    /// Optional payload
    #[serde(default)]
    payload: Option<String>,
    /// Optional key
    #[serde(default)]
    key: Option<String>,

    /// Optional headers
    #[serde(default)]
    headers: HashMap<String, String>,

    /// Optional timeout. If no timeout is provided, assume 1 minute.
    /// The timeout is how long to keep retrying to submit to kafka
    /// before giving up on this attempt.
    /// Note that the underlying library supports retrying forever,
    /// but in kumomta we don't allow that; we can retry later without
    /// keeping the system occupied for an indefinite time.
    #[serde(default)]
    #[serde(with = "duration_serde")]
    timeout: Option<Duration>,
}

impl LuaUserData for Producer {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_async_method("send", |lua, this, value: Value| async move {
            let record: Record = lua.from_value(value)?;

            let headers = if record.headers.is_empty() {
                None
            } else {
                let mut headers = OwnedHeaders::new();
                for (key, v) in &record.headers {
                    headers = headers.insert(Header {
                        key,
                        value: Some(v),
                    });
                }
                Some(headers)
            };

            let future_record = FutureRecord {
                topic: &record.topic,
                partition: record.partition,
                payload: record.payload.as_ref(),
                key: record.key.as_ref(),
                headers,
                timestamp: None,
            };

            let (partition, offset) = this
                .get_producer()?
                .send(
                    future_record,
                    Timeout::After(record.timeout.unwrap_or(Duration::from_secs(60))),
                )
                .await
                .map_err(|(code, _msg)| any_err(code))?;

            Ok((partition, offset))
        });

        methods.add_async_method("send_batch", |lua, this, values: Vec<Value>| async move {
            let mut tasks = FuturesOrdered::new();
            let producer = this.get_producer()?;

            for value in values {
                let record: Record = lua.from_value(value)?;

                let headers = if record.headers.is_empty() {
                    None
                } else {
                    let mut headers = OwnedHeaders::new();
                    for (key, v) in &record.headers {
                        headers = headers.insert(Header {
                            key,
                            value: Some(v),
                        });
                    }
                    Some(headers)
                };

                let producer = producer.clone();

                tasks.push_back(tokio::spawn(async move {
                    producer
                        .send(
                            FutureRecord {
                                topic: &record.topic,
                                partition: record.partition,
                                payload: record.payload.as_ref(),
                                key: record.key.as_ref(),
                                headers,
                                timestamp: None,
                            },
                            Timeout::After(record.timeout.unwrap_or(Duration::from_secs(60))),
                        )
                        .await
                }));
            }

            let mut failed_indexes = vec![];
            let mut index = 1;

            while let Some(result) = tasks.next().await {
                match result {
                    Ok(Ok(_)) => {}
                    Ok(Err((error, _msg))) => {
                        tracing::error!("Error sending to kafka {:?}", error);
                        failed_indexes.push(index);
                    }
                    Err(error) => {
                        tracing::error!("Error sending to kafka {:?}", error);
                        failed_indexes.push(index)
                    }
                }
                index += 1;
            }
            if failed_indexes.is_empty() {
                Ok(Value::Nil)
            } else {
                Ok(lua.to_value(&failed_indexes)?)
            }
        });

        methods.add_method("close", |_lua, this, _: ()| {
            this.producer.lock().unwrap().take();
            Ok(())
        });
    }
}

pub fn register(lua: &Lua) -> anyhow::Result<()> {
    let kafka_mod = get_or_create_sub_module(lua, "kafka")?;

    kafka_mod.set(
        "build_producer",
        lua.create_async_function(|_, config: HashMap<String, String>| async move {
            let mut builder = ClientConfig::new();
            for (k, v) in config {
                builder.set(k, v);
            }

            let producer = builder.create().map_err(any_err)?;

            Ok(Producer {
                producer: Arc::new(Mutex::new(Some(Arc::new(producer)))),
            })
        })?,
    )?;

    Ok(())
}
