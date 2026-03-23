use crate::transport::bridge::{AegisError, BatchRequest};

pub fn encode_batch(request: &BatchRequest) -> Result<String, AegisError> {
    serde_json::to_string(request).map_err(AegisError::Serialize)
}
