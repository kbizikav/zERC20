use serde::{Deserialize, Serialize};

#[derive(Default, Debug, Clone, Copy, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct BitPath {
    length: u32,
    value: u64,
}

impl BitPath {
    pub fn new(length: u32, value: u64) -> Self {
        BitPath { length, value }
    }

    pub fn is_empty(&self) -> bool {
        self.length == 0
    }

    pub fn len(&self) -> u32 {
        self.length
    }

    pub fn value(&self) -> u64 {
        self.value
    }

    pub fn pop(&mut self) -> Option<bool> {
        if self.length == 0 {
            return None;
        }
        let bit = self.value & 1;
        self.value >>= 1;
        self.length -= 1;
        Some(bit == 1)
    }

    pub fn sibling(&self) -> Self {
        // flip the last bit
        let mut path = *self;
        path.value ^= 1;
        path
    }

    pub fn to_bytes(&self) -> [u8; 12] {
        let mut bytes = [0u8; 12];
        bytes[..4].copy_from_slice(&self.length.to_be_bytes());
        bytes[4..].copy_from_slice(&self.value.to_be_bytes());
        bytes
    }

    pub fn from_bytes(bytes: [u8; 12]) -> Self {
        let mut len_bytes = [0u8; 4];
        len_bytes.copy_from_slice(&bytes[..4]);
        let mut value_bytes = [0u8; 8];
        value_bytes.copy_from_slice(&bytes[4..]);
        let length = u32::from_be_bytes(len_bytes);
        let value = u64::from_be_bytes(value_bytes);
        BitPath::new(length, value)
    }
}
