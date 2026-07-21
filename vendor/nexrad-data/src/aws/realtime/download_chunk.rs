use crate::aws::realtime::{Chunk, ChunkIdentifier, REALTIME_BUCKET};
use crate::aws::s3::download_object;

/// Downloads the specified chunk from the real-time NEXRAD data bucket.
pub async fn download_chunk<'a>(
    site: &str,
    chunk_id: &ChunkIdentifier,
) -> crate::result::Result<(ChunkIdentifier, Chunk<'a>)> {
    let key = format!(
        "{}/{}/{}",
        site,
        chunk_id.volume().as_number(),
        chunk_id.name()
    );

    let downloaded_object = download_object(REALTIME_BUCKET, &key).await?;

    Ok((
        ChunkIdentifier::from_name(
            site.to_string(),
            *chunk_id.volume(),
            chunk_id.name().to_string(),
            downloaded_object.metadata.last_modified,
        )?,
        Chunk::new(downloaded_object.data)?,
    ))
}
