use crate::config::TlsConfig;
use crate::error::ProxyError;
use rustls_pemfile::{certs, private_key};
use std::fs::File;
use std::io::BufReader;
use std::sync::Arc;
use tokio_rustls::TlsAcceptor;

pub fn build_tls_acceptor(tls_config: &TlsConfig) -> Result<TlsAcceptor, ProxyError> {
    let cert_file = File::open(&tls_config.cert_path).map_err(|e| {
        ProxyError::TlsError(format!(
            "Failed to open cert file '{}': {}",
            tls_config.cert_path, e
        ))
    })?;
    let key_file = File::open(&tls_config.key_path).map_err(|e| {
        ProxyError::TlsError(format!(
            "Failed to open key file '{}': {}",
            tls_config.key_path, e
        ))
    })?;

    let cert_chain: Vec<_> = certs(&mut BufReader::new(cert_file))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| ProxyError::TlsError(format!("Failed to parse certificates: {}", e)))?;

    let key = private_key(&mut BufReader::new(key_file))
        .map_err(|e| ProxyError::TlsError(format!("Failed to parse private key: {}", e)))?
        .ok_or_else(|| ProxyError::TlsError("No private key found in key file".to_string()))?;

    let mut server_config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(cert_chain, key)
        .map_err(|e| ProxyError::TlsError(format!("TLS config error: {}", e)))?;

    // Enable ALPN for HTTP/2 and HTTP/1.1
    server_config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];

    Ok(TlsAcceptor::from(Arc::new(server_config)))
}
