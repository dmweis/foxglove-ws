//! This library provides means to publish messages to the amazing Foxglove UI in Rust. It
//! implements part of the Foxglove WebSocket protocol described in
//! <https://github.com/foxglove/ws-protocol>.
//!
//! On its own the protocol does not fix a specific data scheme for the messages. But for Foxglove
//! to understand the messages it makes sense to follow the well-known serialization schemes
//! <https://mcap.dev/spec/registry>.
//!
//! # Example
//!
//! This is an example with single ROS1 channel/topic with the `std_msgs/String` message type.
//!
//! ```no_run
//! use std::{io::Write, time::SystemTime};
//!
//! fn build_string_message(data: &str) -> anyhow::Result<Vec<u8>> {
//!     let mut msg = vec![0; std::mem::size_of::<u32>() + data.len()];
//!     // ROS 1 message strings are encoded as 4-bytes length and then the byte data.
//!     let mut w = std::io::Cursor::new(&mut msg);
//!     w.write(&(data.len() as u32).to_le_bytes())?;
//!     w.write(data.as_bytes())?;
//!     Ok(msg)
//! }
//!
//! #[tokio::main]
//! async fn main() -> anyhow::Result<()> {
//!     let server = foxglove_ws::FoxgloveWebSocket::default();
//!     tokio::spawn({
//!         let server = server.clone();
//!         async move { server.serve(([127, 0, 0, 1], 8765)).await }
//!     });
//!     let channel = server
//!         .publish(
//!             "/data".to_string(),
//!             "ros1".to_string(),
//!             "std_msgs/String".to_string(),
//!             "string data".to_string(),
//!             "ros1msg".to_string(),
//!             false,
//!         )
//!         .await?;
//!     channel
//!         .send(
//!             SystemTime::now().elapsed().unwrap().as_nanos() as u64,
//!             &build_string_message("Hello!")?,
//!         )
//!         .await?;
//!     Ok(())
//! }
//! ```

mod protocol_types;

use std::{
    collections::HashMap,
    io::{Cursor, Write},
    mem::size_of,
    net::SocketAddr,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
};

use anyhow::anyhow;
use base64::{engine::general_purpose, Engine as _};
use futures_util::{stream::SplitSink, SinkExt, StreamExt, TryFutureExt};
use log::debug;
use tokio::sync::{mpsc, RwLock};
use tokio_stream::wrappers::ReceiverStream;
use uuid::Uuid;
use warp::{
    ws::{Message, WebSocket},
    Filter,
};

use protocol_types::*;

#[derive(Debug)]
struct Client {
    id: Uuid,
    tx: mpsc::Sender<Message>,
    subscriptions: HashMap<usize, ClientChannelId>,
}

type Clients = RwLock<HashMap<Uuid, Client>>;

#[derive(Debug, Default)]
struct ClientState {
    clients: Clients,
}

#[derive(Debug)]
struct MessageData {
    timestamp_ns: u64,
    data: Vec<u8>,
}

impl MessageData {
    fn build_message(&self, subscription_id: u32) -> anyhow::Result<Message> {
        let mut buffer =
            vec![0; size_of::<u8>() + size_of::<u32>() + size_of::<u64>() + self.data.len()];
        {
            let mut w = Cursor::new(&mut buffer);
            // Write op code for the "Message Data" type.
            w.write_all(&1_u8.to_le_bytes())?;
            // Write subscription ID for this client.
            w.write_all(&subscription_id.to_le_bytes())?;
            w.write_all(&self.timestamp_ns.to_le_bytes())?;
            w.write_all(&self.data)?;
        }
        Ok(Message::binary(buffer))
    }
}

/// Represents a channel to send data with.
#[derive(Debug)]
pub struct Channel {
    id: usize,
    topic: String,
    is_latching: bool,

    clients: Arc<ClientState>,
    channels: Arc<ChannelState>,
    pinned_message: Arc<RwLock<Option<MessageData>>>,
    unadvertised: bool,
}

impl Channel {
    /// Sends a message to all subscribed clients for this channel.
    ///
    /// # Arguments
    ///
    /// * `timestamp_ns` - Point in time this message was published/created/logged.
    /// * `data` - Data buffer to publish.
    pub async fn send(&self, timestamp_ns: u64, data: &[u8]) -> anyhow::Result<()> {
        let message_data = MessageData {
            timestamp_ns,
            data: data.to_vec(),
        };
        for client in self.clients.clients.read().await.values() {
            if let Some(subscription_id) = client.subscriptions.get(&self.id) {
                log::debug!(
                    "Send message on {} to client {} ({}).",
                    self.topic,
                    client.id,
                    client.tx.capacity()
                );
                client
                    .tx
                    .try_send(message_data.build_message(*subscription_id)?)?;
            }
        }

        if self.is_latching {
            *self.pinned_message.write().await = Some(message_data);
        }

        Ok(())
    }

    /// Unadvertises this channel to the given client.
    pub async fn unadvertise(mut self) -> anyhow::Result<()> {
        let message = ServerMessage::Unadvertise {
            channel_ids: vec![self.id],
        };

        for client in self.clients.clients.read().await.values() {
            if let Some(_subscription_id) = client.subscriptions.get(&self.id) {
                log::debug!(
                    "Send message on {} to client {} ({}).",
                    self.topic,
                    client.id,
                    client.tx.capacity()
                );
                client
                    .tx
                    .send(Message::text(serde_json::to_string(&message)?))
                    .await?;
            }
        }

        // remove self from channels
        self.channels.channels.write().await.remove(&self.id);
        self.unadvertised = true;

        Ok(())
    }
}

impl Drop for Channel {
    fn drop(&mut self) {
        if self.unadvertised {
            let channel_id = self.id;
            let topic = self.topic.clone();
            let clients = self.clients.clone();
            let channels = self.channels.clone();
            let pinned_message = self.pinned_message.clone();
            tokio::spawn(async move {
                if let Err(e) = Channel::unadvertise(Channel {
                    id: channel_id,
                    topic,
                    is_latching: false,
                    clients,
                    channels,
                    pinned_message,
                    unadvertised: false,
                })
                .await
                {
                    log::error!("Failed to unadvertise channel: {}", e);
                }
            });
        }
    }
}

#[derive(Debug)]
struct ChannelMetadata {
    channel_message: ServerChannelMessage,
    pinned_message: Arc<RwLock<Option<MessageData>>>,
}

type Channels = RwLock<HashMap<usize, ChannelMetadata>>;

#[derive(Debug, Default)]
struct ChannelState {
    next_channel_id: AtomicUsize,
    channels: Channels,
}

/// The service WebSocket. It tracks the connected clients and takes care of subscriptions.
#[derive(Clone, Debug, Default)]
pub struct FoxgloveWebSocket {
    clients: Arc<ClientState>,
    channels: Arc<ChannelState>,
    pub parameters: Arc<RwLock<HashMap<String, String>>>,
    server_name: String,
}

async fn initialize_client(
    user_ws_tx: &mut SplitSink<WebSocket, Message>,
    channels: &Channels,
    client_id: &Uuid,
    parameters: Arc<RwLock<HashMap<String, String>>>,
    server_name: &str,
) -> anyhow::Result<()> {
    user_ws_tx
        .send(Message::text(
            serde_json::to_string(&ServerMessage::ServerInfo {
                name: server_name.to_owned(),
                capabilities: vec![String::from("parameters")],
                supported_encodings: vec![],
                metadata: HashMap::default(),
                session_id: client_id.as_hyphenated().to_string(),
            })
            .unwrap(),
        ))
        .await?;

    let channel_messages = channels
        .read()
        .await
        .values()
        .map(|metadata| metadata.channel_message.clone())
        .collect();

    user_ws_tx
        .send(Message::text(
            serde_json::to_string(&ServerMessage::Advertise {
                channels: channel_messages,
            })
            .unwrap(),
        ))
        .await?;

    let parameters = parameters
        .read()
        .await
        .iter()
        .map(|(name, value)| ParameterValue::new(name, value))
        .collect();

    user_ws_tx
        .send(Message::text(
            serde_json::to_string(&ServerMessage::ParameterValues {
                id: None,
                parameters,
            })
            .unwrap(),
        ))
        .await?;

    Ok(())
}

async fn handle_client_msg(
    tx: &mpsc::Sender<Message>,
    clients: &Arc<ClientState>,
    channels: &Arc<ChannelState>,
    client_id: &Uuid,
    ws_msg: &Message,
) -> anyhow::Result<()> {
    let msg = if ws_msg.is_text() {
        serde_json::from_str::<ClientMessage>(ws_msg.to_str().unwrap())?
    } else if ws_msg.is_binary() {
        return Err(anyhow!("Got binary message: unhandled at the moment."));
    } else if ws_msg.is_close() {
        // Closing the connection is handled in the general loop for the client.
        // Nothing is left to do here.
        return Ok(());
    } else {
        return Err(anyhow!(
            "Got strage message, neither text nor binary: unhandled at the moment. {:?}",
            ws_msg
        ));
    };

    let mut clients = clients.clients.write().await;

    let channels = channels.channels.read().await;

    match msg {
        ClientMessage::Subscribe { ref subscriptions } => {
            let client = clients
                .get_mut(client_id)
                .ok_or(anyhow!("Client gone from client map?"))?;
            for ClientSubscriptionMessage { id, channel_id } in subscriptions {
                log::debug!(
                    "Client {} subscribed to {} with its own {}.",
                    client_id,
                    channel_id,
                    id
                );

                if let Some(channel_metadata) = channels.get(channel_id) {
                    client.subscriptions.insert(*channel_id, *id);
                    if let Some(message_data) =
                        channel_metadata.pinned_message.read().await.as_ref()
                    {
                        log::debug!("Sending latched: client {}.", client_id);
                        tx.send(message_data.build_message(*id)?).await?;
                    }
                }
            }
        }
        ClientMessage::Unsubscribe {
            ref subscription_ids,
        } => {
            let client = clients
                .get_mut(client_id)
                .ok_or(anyhow!("Client gone from client map?"))?;
            log::debug!("Client {} unsubscribes {:?}.", client_id, subscription_ids);
            client
                .subscriptions
                .retain(|_, subscription_id| !subscription_ids.contains(subscription_id));
        }
        ClientMessage::GetParameters {
            parameter_names,
            id: _,
        } => {
            debug!(
                "Client {} requested parameters: {:?}",
                client_id, parameter_names
            );
        }
        ClientMessage::SetParameters { parameters, id: _ } => {
            debug!("Client {} set parameters: {:?}", client_id, parameters);
        }
    }
    Ok(())
}

async fn client_connected(
    ws: WebSocket,
    clients: Arc<ClientState>,
    channels: Arc<ChannelState>,
    parameters: Arc<RwLock<HashMap<String, String>>>,
    server_name: String,
) {
    // Split the socket into a sender and receive of messages.
    let (mut user_ws_tx, mut user_ws_rx) = ws.split();

    let client_id = Uuid::new_v4();
    log::info!("Client {} connected.", client_id);

    // Send server info.
    if let Err(err) = initialize_client(
        &mut user_ws_tx,
        &channels.channels,
        &client_id,
        parameters,
        &server_name,
    )
    .await
    {
        log::error!("Failed to initialize client: {}.", err);
        return;
    }

    // TODO(mkiefel): Add per channel queue sizes.
    let (tx, rx) = mpsc::channel(10);
    let mut rx = ReceiverStream::new(rx);

    // Setup the sender queue task.
    tokio::task::spawn(async move {
        while let Some(message) = rx.next().await {
            user_ws_tx
                .send(message)
                .unwrap_or_else(|e| {
                    log::error!("Failed websocket send: {}.", e);
                })
                .await;
        }
    });

    // Save the sender in our list of connected users.
    clients.clients.write().await.insert(
        client_id,
        Client {
            id: client_id,
            tx: tx.clone(),
            subscriptions: HashMap::new(),
        },
    );

    while let Some(result) = user_ws_rx.next().await {
        let ws_msg = match result {
            Ok(ws_msg) => ws_msg,
            Err(err) => {
                log::error!("Failed receiving, websocket error: {}.", err);
                break;
            }
        };
        if let Err(err) = handle_client_msg(&tx, &clients, &channels, &client_id, &ws_msg).await {
            log::error!("Failed handling client message: {}.", err);
            break;
        }
    }

    log::info!("Client {} closed.", client_id);
    clients.clients.write().await.remove(&client_id);
}

impl FoxgloveWebSocket {
    /// Creates a new Foxglove WebSocket service.
    pub fn new(server_name: &str) -> Self {
        let server_name = server_name.to_owned();
        Self {
            server_name,
            ..Default::default()
        }
    }

    /// Serves connecting clients.
    ///
    /// # Arguments
    ///
    /// `addr` -- Address to listen on.
    pub async fn serve(&self, addr: impl Into<SocketAddr>) {
        let clients = self.clients.clone();
        let clients = warp::any().map(move || clients.clone());
        let channels = self.channels.clone();
        let channels = warp::any().map(move || channels.clone());
        let parameters = self.parameters.clone();
        let parameters = warp::any().map(move || parameters.clone());
        let server_name = self.server_name.to_owned();
        let server_name = warp::any().map(move || server_name.clone());
        let foxglove_ws = warp::path::end().and(
            warp::ws()
                .and(clients)
                .and(channels)
                .and(parameters)
                .and(server_name)
                .map(
                    |ws: warp::ws::Ws, clients, channels, parameters, server_name: String| {
                        ws.on_upgrade(move |socket| {
                            client_connected(socket, clients, channels, parameters, server_name)
                        })
                    },
                )
                .map(|reply| {
                    warp::reply::with_header(
                        reply,
                        "Sec-WebSocket-Protocol",
                        "foxglove.websocket.v1",
                    )
                }),
        );
        warp::serve(foxglove_ws).run(addr).await;
    }

    /// Advertise a new publisher.
    ///
    /// There are several different message encoding schemes that are supported by Foxglove.
    /// <https://mcap.dev/spec/registry> contains more information on how to set the arguments to
    /// this function.
    ///
    /// # Arguments
    ///
    /// * `topic` - Name of the topic of this new channel.
    /// * `encoding` - Channel message encoding.
    /// * `schema_name` - Name of the schema.
    /// * `schema` - Schema describing the message format.
    /// * `scheme_encoding` - Optional type of encoding used for schema encoding. May be used if the schema encoding can't be uniquely deduced from the message encoding.
    /// * `is_latching` - Whether messages sent of this channel are sticky. Each newly connecting
    ///    client will be message the last sticky message that was sent on this channel.
    pub async fn create_publisher<S: Into<SchemaDescriptor>>(
        &self,
        topic: &str,
        encoding: &str,
        schema_name: &str,
        schema: S,
        schema_encoding: Option<&str>,
        is_latching: bool,
    ) -> anyhow::Result<Channel> {
        let channel_id = self
            .channels
            .next_channel_id
            .fetch_add(1, Ordering::Relaxed);
        log::debug!("Publishing new channel {}: {}.", topic, channel_id);
        let channel = Channel {
            id: channel_id,
            topic: topic.to_owned(),
            is_latching,
            clients: self.clients.clone(),
            channels: self.channels.clone(),
            pinned_message: Arc::default(),
            unadvertised: false,
        };
        let channel_message = ServerChannelMessage {
            id: channel_id,
            topic: topic.to_owned(),
            encoding: encoding.to_owned(),
            schema_name: schema_name.to_owned(),
            schema: schema.into().0,
            schema_encoding: schema_encoding.map(|s| s.to_owned()),
        };

        // Advertise the newly created channel.
        for client in self.clients.clients.read().await.values() {
            client
                .tx
                .send(Message::text(
                    serde_json::to_string(&ServerMessage::Advertise {
                        channels: vec![channel_message.clone()],
                    })
                    .unwrap(),
                ))
                .await?;
        }

        self.channels.channels.write().await.insert(
            channel_id,
            ChannelMetadata {
                channel_message,
                pinned_message: channel.pinned_message.clone(),
            },
        );

        Ok(channel)
    }

    /// Advertise a new publisher.
    ///
    /// There are several different message encoding schemes that are supported by Foxglove.
    /// <https://mcap.dev/spec/registry> contains more information on how to set the arguments to
    /// this function.
    ///
    /// # Arguments
    ///
    /// * `topic` - Name of the topic of this new channel.
    /// * `encoding` - Channel message encoding.
    /// * `schema_name` - Name of the schema.
    /// * `schema` - Schema describing the message format.
    /// * `scheme_encoding` - Encoding of this channel's schema.
    /// * `is_latching` - Whether messages sent of this channel are sticky. Each newly connecting
    ///    client will be message the last sticky message that was sent on this channel.
    #[deprecated(note = "Please use `create_publisher` instead")]
    pub async fn publish(
        &self,
        topic: String,
        encoding: String,
        schema_name: String,
        schema: String,
        schema_encoding: String,
        is_latching: bool,
    ) -> anyhow::Result<Channel> {
        let channel = self
            .create_publisher(
                &topic,
                &encoding,
                &schema_name,
                schema,
                Some(&schema_encoding),
                is_latching,
            )
            .await?;
        Ok(channel)
    }
}

/// Wrapper around different types of schema descriptors.
/// Binary descriptors will get base64 encoded.
pub struct SchemaDescriptor(String);

impl From<String> for SchemaDescriptor {
    fn from(content: String) -> Self {
        SchemaDescriptor(content)
    }
}

impl From<&str> for SchemaDescriptor {
    fn from(content: &str) -> Self {
        SchemaDescriptor(content.to_owned())
    }
}

impl From<Vec<u8>> for SchemaDescriptor {
    fn from(data: Vec<u8>) -> Self {
        let encoded: String = general_purpose::STANDARD_NO_PAD.encode(data);
        SchemaDescriptor(encoded)
    }
}

impl From<&[u8]> for SchemaDescriptor {
    fn from(data: &[u8]) -> Self {
        let encoded: String = general_purpose::STANDARD_NO_PAD.encode(data);
        SchemaDescriptor(encoded)
    }
}
