//! Where a radar volume comes from, per target.
//!
//! Native reads through the on-disk cache; the browser reads through an offline chase pack in
//! IndexedDB (`webcache.rs`) before touching the network, which is what makes a saved loop play
//! with no signal at all. Every timeline fetch goes through here so both caches are one seam, not
//! four call sites each.

use wxdata::level2::{self, Identifier, Scan};

/// Fetch and decode one volume, using whatever cache this target has.
///
/// `cache` is the native disk cache directory, or `None` at the live head where the newest object
/// may still be uploading.
pub async fn fetch(id: Identifier, cache: Option<std::path::PathBuf>) -> anyhow::Result<Scan> {
    #[cfg(target_arch = "wasm32")]
    {
        let _ = &cache;
        let name = id.name().to_string();
        if let Some(bytes) = crate::webcache::volume(&name).await {
            log::debug!("volume {name} came from an offline pack");
            return level2::scan_from_volume_bytes(&name, bytes).await;
        }
    }
    level2::download_scan(id, cache).await
}
