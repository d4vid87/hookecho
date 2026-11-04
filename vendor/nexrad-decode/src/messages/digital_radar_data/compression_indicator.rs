/// Indicates whether the message is compressed and what type of compression was used.
#[derive(Debug, PartialEq, Eq, Clone, Copy, Hash)]
pub enum CompressionIndicator {
    /// Data is not compressed.
    Uncompressed,
    /// Data is compressed using BZIP2 algorithm.
    CompressedBZIP2,
    /// Data is compressed using ZLIB algorithm.
    CompressedZLIB,
    /// Reserved for future compression methods.
    FutureUse,
}
