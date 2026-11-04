use super::DataBlockId;
use std::ops::Deref;

/// A wrapper that pairs a data block with its identifier.
///
/// This generic wrapper preserves the `DataBlockId` that precedes each data block
/// in the binary format, allowing consumers to access both the block type identifier
/// and the block data itself.
///
/// This type implements `Deref<Target = T>`, so you can call methods on the inner
/// type directly through the wrapper.
#[derive(Debug, Clone, PartialEq)]
pub struct DataBlock<'a, T> {
    /// The data block identifier that precedes this block in the binary data.
    id: DataBlockId<'a>,

    /// The inner data block.
    inner: T,
}

impl<'a, T> DataBlock<'a, T> {
    /// Creates a new data block wrapper with the given ID and inner data.
    pub(crate) fn new(id: DataBlockId<'a>, inner: T) -> Self {
        Self { id, inner }
    }

    /// Convert this data block to an owned version with `'static` lifetime,
    /// using the provided function to convert the inner value.
    pub fn into_owned_with<U>(self, f: impl FnOnce(T) -> U) -> DataBlock<'static, U> {
        DataBlock {
            id: self.id.into_owned(),
            inner: f(self.inner),
        }
    }

    /// The data block identifier that precedes this block in the binary data.
    pub fn id(&self) -> &DataBlockId<'a> {
        &self.id
    }

    /// The inner data block.
    pub fn inner(&self) -> &T {
        &self.inner
    }

    /// Consume this wrapper and return the inner data block.
    pub fn into_inner(self) -> T {
        self.inner
    }
}

impl<'a, T> Deref for DataBlock<'a, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}
