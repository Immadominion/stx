//! Yellowstone gRPC connection and streaming loop.
//!
//! Connects with TLS (native roots), a raised decode limit (the 4 MiB server
//! default silently drops large messages), an optional `x-token`, then
//! subscribes and forwards normalized [`Observation`]s onto an mpsc channel.
//! The stream returned by `subscribe_with_request` is wrapped in Yellowstone
//! 13.1's `AutoReconnect` (slot-tracked replay + dedup), so reconnection across
//! transient drops is handled beneath us; we additionally reply to server pings
//! to keep idle connections past load-balancer timeouts.

use crate::error::IngestError;
use crate::observation::{normalize, Observation};
use futures::{SinkExt, StreamExt};
use tokio::sync::mpsc;
use tonic::transport::ClientTlsConfig;
use yellowstone_grpc_client::GeyserGrpcClient;
use yellowstone_grpc_proto::geyser::{SubscribeRequest, SubscribeRequestPing};

/// Default client receive limit (the server default of 4 MiB is too small for
/// block/large-account messages).
pub const DEFAULT_MAX_DECODING_SIZE: usize = 64 * 1024 * 1024;

#[derive(Debug, Clone)]
pub struct IngestorConfig {
    pub endpoint: String,
    pub x_token: Option<String>,
    pub max_decoding_message_size: usize,
}

impl IngestorConfig {
    pub fn new(endpoint: impl Into<String>, x_token: Option<String>) -> Self {
        Self {
            endpoint: endpoint.into(),
            x_token,
            max_decoding_message_size: DEFAULT_MAX_DECODING_SIZE,
        }
    }
}

/// Establish a connected client (TLS, x-token, raised decode limit).
pub async fn connect(cfg: &IngestorConfig) -> Result<GeyserGrpcClient, IngestError> {
    let client = GeyserGrpcClient::build_from_shared(cfg.endpoint.clone())?
        .x_token(cfg.x_token.clone())?
        .tls_config(ClientTlsConfig::new().with_native_roots())?
        .max_decoding_message_size(cfg.max_decoding_message_size)
        .connect()
        .await?;
    Ok(client)
}

/// Connect, subscribe with `request`, and forward normalized observations onto
/// `tx` until the stream ends or the consumer drops. Replies to server pings.
pub async fn run(
    cfg: IngestorConfig,
    request: SubscribeRequest,
    tx: mpsc::Sender<Observation>,
) -> Result<(), IngestError> {
    let mut client = connect(&cfg).await?;
    let (mut sink, mut stream) = client.subscribe_with_request(Some(request)).await?;

    while let Some(message) = stream.next().await {
        let update = message?;
        let Some(obs) = normalize(update) else {
            continue;
        };
        if matches!(obs, Observation::Ping) {
            // Keep-alive: reply with a ping request (server answers with a pong).
            let _ = sink
                .send(SubscribeRequest {
                    ping: Some(SubscribeRequestPing { id: 1 }),
                    ..Default::default()
                })
                .await;
        }
        if tx.send(obs).await.is_err() {
            break; // consumer dropped; stop ingesting.
        }
    }

    // Keep the client alive for the whole stream lifetime.
    drop(client);
    Ok(())
}
