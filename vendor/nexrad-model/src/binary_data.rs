//! Wrapper for binary data with a concise Debug representation.

use sha2::{Digest, Sha256};
use std::fmt::{self, Debug, Formatter};
use std::ops::{Deref, DerefMut};

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// A wrapper for binary data that provides a concise Debug implementation showing size, hash,
/// and a sample of head/tail bytes instead of the full binary content.
#[repr(transparent)]
#[derive(Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(transparent))]
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

        let mut hasher = Sha256::new();
        hasher.update(bytes);
        let hash = hex::encode(hasher.finalize());

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
