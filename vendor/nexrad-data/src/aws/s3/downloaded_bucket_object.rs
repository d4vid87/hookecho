use crate::aws::s3::bucket_object::BucketObject;

/// A bucket object returned from an S3 list objects request.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DownloadedBucketObject {
    /// The metadata of the object.
    pub metadata: BucketObject,
    /// The object data.
    pub data: Vec<u8>,
}
