//! Shared HTTP client for AWS operations.
//!
//! This module provides a singleton HTTP client with connection pooling for
//! efficient reuse across multiple S3 operations.

use once_cell::sync::Lazy;
use reqwest::Client;

// hookecho patch: a hook for bending S3 URLs before they are fetched.
//
// The browser build routes every other feed through its own origin's CORS proxy
// (`wxdata::net::fetch_url`), which is also where edge caching happens — a Level 2 volume that
// one visitor pulls is then free for the next. These two modules were the exception: they build
// bucket URLs and hand them straight to reqwest. Rather than thread a config through every
// call site, the app installs a rewriter once at startup. Native never installs one, so
// `rewrite_url` is the identity function there and nothing changes.
static URL_REWRITER: once_cell::sync::OnceCell<fn(&str) -> String> =
    once_cell::sync::OnceCell::new();

/// Install a rewriter applied to every S3 URL this crate fetches. First call wins.
pub fn set_url_rewriter(f: fn(&str) -> String) {
    let _ = URL_REWRITER.set(f);
}

pub(crate) fn rewrite_url(url: String) -> String {
    match URL_REWRITER.get() {
        Some(f) => f(&url),
        None => url,
    }
}

/// Returns a reference to the shared HTTP client.
///
/// The client is lazily initialized on first use and reused for all subsequent
/// requests. On native targets, it is configured with connection pooling for
/// improved performance when making multiple requests to the same host.
pub fn client() -> &'static Client {
    static CLIENT: Lazy<Client> = Lazy::new(|| {
        #[allow(unused_mut)]
        let mut builder = Client::builder();

        // Connection pooling is only available on native targets
        #[cfg(not(target_arch = "wasm32"))]
        {
            builder = builder.pool_max_idle_per_host(4);
            // hookecho patch: timeouts. Volume fetches are the app's slowest path, and a request
            // that never answers wedges the pane's `loading` flag with no way back.
            builder = builder.connect_timeout(std::time::Duration::from_secs(10));
            builder = builder.timeout(std::time::Duration::from_secs(60));
            // hookecho patch: reqwest 0.13's `rustls` feature wires rustls-platform-verifier as
            // the cert verifier, which on Android needs a bundled Kotlin helper class (and panics
            // if uninitialized). Hand reqwest a fully-built rustls config using webpki roots
            // instead, so the platform verifier is never touched and TLS works on every target.
            builder = builder.use_preconfigured_tls(hookecho_webpki_tls());
        }

        builder
            .build()
            .unwrap_or_else(|e| panic!("Failed to create HTTP client: {e}"))
    });

    &CLIENT
}

// hookecho patch: a rustls ClientConfig trusting the Mozilla webpki root set, with an explicit
// aws-lc-rs provider (both aws-lc-rs and ring are in the graph, so the process default is
// ambiguous — name it). ALPN mirrors reqwest's own rustls setup.
#[cfg(not(target_arch = "wasm32"))]
fn hookecho_webpki_tls() -> rustls::ClientConfig {
    let mut roots = rustls::RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let mut config = rustls::ClientConfig::builder_with_provider(std::sync::Arc::new(
        rustls::crypto::aws_lc_rs::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .expect("aws-lc-rs supports the default protocol versions")
    .with_root_certificates(roots)
    .with_no_client_auth();
    config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
    config
}
