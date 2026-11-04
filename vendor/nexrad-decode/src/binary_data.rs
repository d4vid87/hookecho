use sha2::{Digest, Sha256};
use std::fmt::{self, Debug, Formatter};
use std::ops::{Deref, DerefMut};
use zerocopy::{FromBytes, Immutable, KnownLayout};

/// A wrapper for binary data that provides a concise Debug implementation showing size, hash,
/// and a sample of head/tail bytes instead of the full binary content.
#[repr(transparent)]
#[derive(Clone, PartialEq, Eq, Hash, FromBytes, Immutable, KnownLayout)]
pub struct BinaryData<T>(pub T);

impl<T> BinaryData<T> {
    /// Creates a new BinaryData wrapper.
    pub fn new(data: T) -> Self {
        Self(data)
    }

    /// Unwraps the inner data.
    pub fn into_inner(self) -> T {
        self.0
    }
}

impl<T: AsRef<[u8]>> Debug for BinaryData<T> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        let bytes = self.0.as_ref();
        let len = bytes.len();

        // Compute SHA-256 hash
        let mut hasher = Sha256::new();
        hasher.update(bytes);
        let hash = hex::encode(hasher.finalize());

        // Get head (first 4 bytes) and tail (last 4 bytes)
        let head: Vec<u8> = bytes.iter().take(4).copied().collect();
        let tail: Vec<u8> = if len > 4 {
            bytes.iter().rev().take(4).rev().copied().collect()
        } else {
            Vec::new()
        };

        f.debug_struct("BinaryData")
            .field("len", &len)
            .field("sha256", &hash)
            .field("head", &head)
            .field("tail", &tail)
            .finish()
    }
}

impl<T> Deref for BinaryData<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<T> DerefMut for BinaryData<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl<T: AsRef<[u8]>> AsRef<[u8]> for BinaryData<T> {
    fn as_ref(&self) -> &[u8] {
        self.0.as_ref()
    }
}

impl<T> From<T> for BinaryData<T> {
    fn from(data: T) -> Self {
        Self(data)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_debug_vec() {
        let data = vec![0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08];
        let binary_data = BinaryData::new(data);
        let debug_str = format!("{:?}", binary_data);

        assert!(debug_str.contains("len: 8"));
        assert!(debug_str.contains("sha256:"));
        assert!(debug_str.contains("head: [1, 2, 3, 4]"));
        assert!(debug_str.contains("tail: [5, 6, 7, 8]"));
    }

    #[test]
    fn test_debug_array() {
        let data = [0xAA, 0xBB, 0xCC];
        let binary_data = BinaryData::new(data);
        let debug_str = format!("{:?}", binary_data);

        assert!(debug_str.contains("len: 3"));
        assert!(debug_str.contains("head: [170, 187, 204]"));
        assert!(debug_str.contains("tail: []")); // Less than 4 bytes
    }

    #[test]
    fn test_deref() {
        let data = vec![1, 2, 3];
        let binary_data = BinaryData::new(data);
        assert_eq!(binary_data.len(), 3);
        assert_eq!(&binary_data[0], &1);
    }

    #[test]
    fn test_as_ref() {
        let data = [1, 2, 3, 4];
        let binary_data = BinaryData::new(data);
        let slice: &[u8] = binary_data.as_ref();
        assert_eq!(slice, &[1, 2, 3, 4]);
    }
}
