use crate::aws::client::client;
use crate::aws::s3::bucket_list_result::BucketListResult;
use crate::aws::s3::bucket_object::BucketObject;
use crate::aws::s3::bucket_object_field::BucketObjectField;
use crate::result::aws::AWSError;
use crate::result::aws::AWSError::S3ListObjects;
use chrono::{DateTime, Utc};
use log::{debug, trace, warn};
use xml::reader::XmlEvent;
use xml::EventReader;

/// Lists objects from a S3 bucket with the specified prefix. A maximum number of keys can be
/// specified to limit the number of objects returned, otherwise it will use AWS's default (1000).
pub async fn list_objects(
    bucket: &str,
    prefix: &str,
    max_keys: Option<usize>,
) -> crate::result::Result<BucketListResult> {
    let mut path = format!("https://{bucket}.s3.amazonaws.com?list-type=2&prefix={prefix}");
    if let Some(max_keys) = max_keys {
        path.push_str(&format!("&max-keys={max_keys}"));
    }
    debug!("Listing objects in bucket \"{bucket}\" with prefix \"{prefix}\"");

    let response = client().get(&path).send().await.map_err(S3ListObjects)?;
    trace!("  List objects response status: {}", response.status());

    let body = response.text().await.map_err(S3ListObjects)?;
    trace!("  List objects response body length: {}", body.len());

    let parser = EventReader::new(body.as_bytes());

    let mut objects = Vec::new();
    let mut truncated = false;
    let mut object: Option<BucketObject> = None;

    let mut field: Option<BucketObjectField> = None;
    for event in parser {
        match event {
            Ok(XmlEvent::StartElement { name, .. }) => match name.local_name.as_ref() {
                "IsTruncated" => field = Some(BucketObjectField::IsTruncated),
                "Contents" => {
                    object = Some(BucketObject {
                        key: String::new(),
                        last_modified: None,
                        size: 0,
                    });
                }
                "Key" => field = Some(BucketObjectField::Key),
                "LastModified" => field = Some(BucketObjectField::LastModified),
                "Size" => field = Some(BucketObjectField::Size),
                _ => field = None,
            },
            Ok(XmlEvent::Characters(chars)) => {
                if let Some(field) = field.as_ref() {
                    if field == &BucketObjectField::IsTruncated {
                        truncated = chars == "true";
                        if truncated {
                            trace!("  List objects truncated: {truncated}");
                        }
                        continue;
                    }

                    let item = object.as_mut().ok_or_else(|| {
                        warn!("Expected item for object field: {field:?}");
                        AWSError::S3ListObjectsDecoding
                    })?;
                    match field {
                        BucketObjectField::Key => item.key.push_str(&chars),
                        BucketObjectField::LastModified => {
                            item.last_modified = DateTime::parse_from_rfc3339(&chars)
                                .ok()
                                .map(|date_time| date_time.with_timezone(&Utc));
                        }
                        BucketObjectField::Size => {
                            item.size = chars.parse().map_err(|_| {
                                warn!("Error parsing object size: {chars}");
                                AWSError::S3ListObjectsDecoding
                            })?;
                        }
                        _ => {}
                    }
                }
            }
            Ok(XmlEvent::EndElement { name }) => {
                if name.local_name.as_str() == "Contents" {
                    if let Some(item) = object.take() {
                        objects.push(item);
                    }
                }
            }
            _ => {}
        }
    }

    trace!("  List objects found: {}", objects.len());

    Ok(BucketListResult { truncated, objects })
}
