use serde::{Deserialize, Serialize};

use crate::error::{CoreError, CoreResult};

#[derive(Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ContentId([u8; 32]);

impl ContentId {
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    pub fn from_hex(hex_str: &str) -> CoreResult<Self> {
        let decoded = hex::decode(hex_str)
            .map_err(|e| CoreError::InvalidArgument(format!("invalid content id hex: {e}")))?;
        let bytes: [u8; 32] = decoded.try_into().map_err(|v: Vec<u8>| {
            CoreError::InvalidArgument(format!("content id must be 32 bytes, got {}", v.len()))
        })?;
        Ok(Self(bytes))
    }

    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    pub fn to_hex(&self) -> String {
        hex::encode(self.0)
    }
}

impl std::fmt::Display for ContentId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.to_hex())
    }
}

impl std::fmt::Debug for ContentId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ContentId({})", self.to_hex())
    }
}

pub fn hash_bytes(data: &[u8]) -> ContentId {
    ContentId::from_bytes(*blake3::hash(data).as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_hex() {
        let id = hash_bytes(b"proteus");
        let parsed = ContentId::from_hex(&id.to_hex()).expect("valid hex");
        assert_eq!(id, parsed);
    }
}
