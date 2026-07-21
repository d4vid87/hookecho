use crate::aws::archive::identifier::Identifier;
use crate::aws::archive::ARCHIVE_BUCKET;
use crate::aws::s3::list_objects;
use crate::result::aws::AWSError::TruncatedListObjectsResponse;
use crate::result::Error::AWS;
use chrono::NaiveDate;

/// List data files for the specified site and date. This effectively returns an index of data files
/// which can then be individually downloaded.
pub async fn list_files(site: &str, date: &NaiveDate) -> crate::result::Result<Vec<Identifier>> {
    let prefix = format!("{}/{}", date.format("%Y/%m/%d"), site);
    let list_result = list_objects(ARCHIVE_BUCKET, &prefix, None).await?;
    if list_result.truncated {
        return Err(AWS(TruncatedListObjectsResponse));
    }

    let metas = list_result
        .objects
        .iter()
        .map(|object| {
            let key_parts = object.key.split('/');
            let name = key_parts.skip(4).collect::<String>();

            Identifier::new(name)
        })
        .collect();

    Ok(metas)
}
