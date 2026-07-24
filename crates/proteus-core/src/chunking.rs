use bytes::Bytes;

use crate::hash::{hash_bytes, ContentId};

pub const DEFAULT_CHUNK_SIZE: usize = 1024 * 1024;

#[derive(Clone, Debug)]
pub struct Chunk {
    pub id: ContentId,
    pub data: Bytes,
    pub index: u64,
}

#[derive(Clone, Debug)]
pub struct Chunker {
    chunk_size: usize,
}

impl Default for Chunker {
    fn default() -> Self {
        Self {
            chunk_size: DEFAULT_CHUNK_SIZE,
        }
    }
}

impl Chunker {
    pub fn new(chunk_size: usize) -> Option<Self> {
        (chunk_size > 0).then_some(Self { chunk_size })
    }

    pub fn chunk(&self, data: &[u8]) -> Vec<Chunk> {
        if data.is_empty() {
            return Vec::new();
        }

        data.chunks(self.chunk_size)
            .enumerate()
            .map(|(index, slice)| Chunk {
                id: hash_bytes(slice),
                data: Bytes::copy_from_slice(slice),
                index: index as u64,
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunks_fixed_size() {
        let chunker = Chunker::new(4).expect("non-zero");
        let chunks = chunker.chunk(b"abcdefghij");
        assert_eq!(chunks.len(), 3);
        assert_eq!(chunks[0].data.as_ref(), b"abcd");
        assert_eq!(chunks[2].data.as_ref(), b"ij");
    }
}
