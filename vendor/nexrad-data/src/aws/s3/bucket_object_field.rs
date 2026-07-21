/// A field in the S3 list objects response. These are not necessarily part of the same object.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BucketObjectField {
    /// Whether the list of objects is truncated. Child of `ListBucketResult`.
    IsTruncated,
    /// The key of a bucket object. Child of `Contents`.
    Key,
    /// The last modified time of a bucket object. Child of `Contents`.
    LastModified,
    /// The size of a bucket object. Child of `Contents`.
    Size,
}
