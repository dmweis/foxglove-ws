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
    #[serde(rename_all = "camelCase")]
    ParameterValues {
        parameters: Vec<ParameterValue>,
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
    },
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
    #[serde(rename_all = "camelCase")]
    GetParameters {
        parameter_names: Vec<String>,
        #[allow(unused)]
        id: String,
    },
    #[serde(rename_all = "camelCase")]
    SetParameters {
        parameters: HashMap<String, String>,
        #[allow(unused)]
        id: String,
    },
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ParameterValue {
    name: String,
    value: String,
    #[serde(rename = "field", skip_serializing_if = "Option::is_none")]
    field_type: Option<String>,
}

impl ParameterValue {
    pub(crate) fn new(name: &str, value: &str) -> Self {
        Self {
            name: name.to_owned(),
            value: value.to_owned(),
            field_type: None,
        }
    }

    #[allow(unused)]
    pub(crate) fn with_type(name: &str, value: &str, field_type: &str) -> Self {
        Self {
            name: name.to_owned(),
            value: value.to_owned(),
            field_type: Some(field_type.to_owned()),
        }
    }
}
