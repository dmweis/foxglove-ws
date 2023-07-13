//! Types for the foxglove websocket protocol messages.
//! Spec for the protocol can be found here: <https://github.com/foxglove/ws-protocol/blob/main/docs/spec.md>

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ServerChannelMessage {
    pub(crate) id: usize,
    pub(crate) topic: String,
    pub(crate) encoding: String,
    pub(crate) schema_name: String,
    pub(crate) schema: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) schema_encoding: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "op", rename_all = "camelCase")]
pub(crate) enum ServerMessage {
    #[serde(rename_all = "camelCase")]
    ServerInfo {
        name: String,
        capabilities: Vec<String>,
        supported_encodings: Vec<String>,
        metadata: HashMap<String, String>,
        session_id: String,
    },
    #[serde(rename_all = "camelCase")]
    Advertise { channels: Vec<ServerChannelMessage> },
    #[serde(rename_all = "camelCase")]
    Unadvertise { channel_ids: Vec<usize> },
}

pub(crate) type ClientChannelId = u32;

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ClientSubscriptionMessage {
    pub(crate) id: ClientChannelId,
    pub(crate) channel_id: usize,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "op", rename_all = "camelCase")]
pub(crate) enum ClientMessage {
    #[serde(rename_all = "camelCase")]
    Subscribe {
        subscriptions: Vec<ClientSubscriptionMessage>,
    },
    #[serde(rename_all = "camelCase")]
    Unsubscribe {
        subscription_ids: Vec<ClientChannelId>,
    },
}
