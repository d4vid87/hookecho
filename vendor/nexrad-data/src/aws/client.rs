//! Shared HTTP client for AWS operations.
//!
//! This module provides a singleton HTTP client with connection pooling for
//! efficient reuse across multiple S3 operations.

use once_cell::sync::Lazy;
use reqwest::Client;

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
