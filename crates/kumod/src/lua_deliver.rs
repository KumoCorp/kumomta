use crate::ready_queue::{Dispatcher, QueueDispatcher};
use async_trait::async_trait;
use config::{load_config, LuaConfig};
use message::Message;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct LuaDeliveryProtocol {
    /// The name of an event to fire that will construct
    /// the delivery implementation
    pub constructor: String,
    /// Additional argument to pass to the constructor event
    pub constructor_arg: Option<Value>,
}

#[derive(Debug)]
pub struct LuaQueueDispatcher {
    lua_config: LuaConfig,
}

#[async_trait]
impl QueueDispatcher for LuaQueueDispatcher {
    async fn close_connection(&mut self, dispatcher: &mut Dispatcher) -> anyhow::Result<bool> {
        todo!();
    }

    async fn attempt_connection(&mut self, dispatcher: &mut Dispatcher) -> anyhow::Result<()> {
        todo!();
    }

    async fn have_more_connection_candidates(&mut self, dispatcher: &mut Dispatcher) -> bool {
        false
    }

    async fn deliver_message(
        &mut self,
        msg: Message,
        dispatcher: &mut Dispatcher,
    ) -> anyhow::Result<()> {
        todo!();
    }
}
