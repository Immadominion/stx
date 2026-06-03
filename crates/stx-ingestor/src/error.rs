//! Errors for the Yellowstone ingestor.

use thiserror::Error;
use yellowstone_grpc_client::{GeyserGrpcBuilderError, GeyserGrpcClientError};

#[derive(Debug, Error)]
pub enum IngestError {
    #[error("gRPC builder/connect error: {0}")]
    Builder(#[from] GeyserGrpcBuilderError),

    #[error("gRPC client error: {0}")]
    Client(#[from] GeyserGrpcClientError),

    #[error("stream status: {0}")]
    Status(#[from] tonic::Status),
}
