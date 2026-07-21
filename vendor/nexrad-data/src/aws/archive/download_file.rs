use crate::aws::archive::identifier::Identifier;
use crate::aws::archive::ARCHIVE_BUCKET;
use crate::aws::s3::download_object;
use crate::result::aws::AWSError::{DateTime, InvalidSiteIdentifier};
use crate::volume::File;

/// Download a data file specified by its metadata. Returns the downloaded file's encoded contents
/// which may then need to be decompressed and decoded.
pub async fn download_file(identifier: Identifier) -> crate::result::Result<File> {
    let date = identifier
        .date_time()
        .ok_or_else(|| DateTime(identifier.name().to_string()))?;

    let site = identifier
        .site()
        .ok_or_else(|| InvalidSiteIdentifier(identifier.name().to_string()))?;

    let key = format!("{}/{}/{}", date.format("%Y/%m/%d"), site, identifier.name());
    let downloaded_object = download_object(ARCHIVE_BUCKET, &key).await?;

    Ok(File::new(downloaded_object.data))
}
