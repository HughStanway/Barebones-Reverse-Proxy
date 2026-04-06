use crate::config::CertConfig;
use crate::error::ProxyError;
use rustls::server::ResolvesServerCertUsingSni;
use rustls::sign::CertifiedKey;
use rustls_pemfile::{certs, private_key};
use std::fs::File;
use std::io::BufReader;
use std::sync::Arc;
use tokio_rustls::TlsAcceptor;

fn load_certified_key(
    cert_config: &CertConfig,
    provider: &rustls::crypto::CryptoProvider,
) -> Result<CertifiedKey, ProxyError> {
    let cert_file = File::open(&cert_config.cert_path).map_err(|e| {
        ProxyError::TlsError(format!(
            "Failed to open cert file '{}': {}",
            cert_config.cert_path, e
        ))
    })?;
    let key_file = File::open(&cert_config.key_path).map_err(|e| {
        ProxyError::TlsError(format!(
            "Failed to open key file '{}': {}",
            cert_config.key_path, e
        ))
    })?;

    let cert_chain: Vec<_> = certs(&mut BufReader::new(cert_file))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| ProxyError::TlsError(format!("Failed to parse certificates: {}", e)))?;

    let key = private_key(&mut BufReader::new(key_file))
        .map_err(|e| ProxyError::TlsError(format!("Failed to parse private key: {}", e)))?
        .ok_or_else(|| ProxyError::TlsError("No private key found in key file".to_string()))?;

    CertifiedKey::from_der(cert_chain, key, provider)
        .map_err(|e| ProxyError::TlsError(format!("TLS config error: {}", e)))
}

pub fn build_tls_acceptor(cert_configs: &[CertConfig]) -> Result<Option<TlsAcceptor>, ProxyError> {
    if cert_configs.is_empty() {
        return Ok(None);
    }

    let builder = rustls::ServerConfig::builder().with_no_client_auth();
    let mut resolver = ResolvesServerCertUsingSni::new();

    for cert_config in cert_configs {
        let certified_key = load_certified_key(cert_config, builder.crypto_provider().as_ref())?;
        resolver
            .add(&cert_config.hostname, certified_key)
            .map_err(|e| {
                ProxyError::TlsError(format!(
                    "TLS config error for hostname '{}': {}",
                    cert_config.hostname, e
                ))
            })?;
    }

    let mut server_config = builder.with_cert_resolver(Arc::new(resolver));

    // Enable ALPN for HTTP/2 and HTTP/1.1
    server_config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];

    Ok(Some(TlsAcceptor::from(Arc::new(server_config))))
}
