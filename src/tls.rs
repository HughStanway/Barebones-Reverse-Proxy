use crate::config::Route;
use crate::error::ProxyError;
use rustls::server::{ClientHello, ResolvesServerCert};
use rustls::sign::CertifiedKey;
use rustls_pemfile::{certs, private_key};
use std::collections::HashMap;
use std::fs::File;
use std::io::BufReader;
use std::sync::Arc;
use tokio_rustls::TlsAcceptor;

#[derive(Debug, Default)]
pub struct ExactCertResolver {
    exact_match: HashMap<String, Arc<CertifiedKey>>,
}

impl ExactCertResolver {
    pub fn new() -> Self {
        Self {
            exact_match: HashMap::new(),
        }
    }

    pub fn add(&mut self, hostname: String, key: CertifiedKey) {
        self.exact_match.insert(hostname, Arc::new(key));
    }
}

impl ResolvesServerCert for ExactCertResolver {
    fn resolve(&self, client_hello: ClientHello) -> Option<Arc<CertifiedKey>> {
        let sni = client_hello.server_name()?;
        self.exact_match.get(sni).cloned()
    }
}

fn extract_hostname_from_endpoint(endpoint: &str) -> Option<&str> {
    if let Some(stripped) = endpoint.strip_prefix("https://") {
        let host_and_port = stripped.split('/').next()?;
        let host = host_and_port.split(':').next()?;
        if !host.is_empty() { Some(host) } else { None }
    } else {
        None
    }
}

fn load_certified_key(
    cert_path: &str,
    key_path: &str,
    provider: &rustls::crypto::CryptoProvider,
) -> Result<CertifiedKey, ProxyError> {
    let cert_file = File::open(cert_path).map_err(|e| {
        ProxyError::TlsError(format!("Failed to open cert file '{}': {}", cert_path, e))
    })?;
    let key_file = File::open(key_path).map_err(|e| {
        ProxyError::TlsError(format!("Failed to open key file '{}': {}", key_path, e))
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

pub fn build_tls_acceptor(routes: &[Route]) -> Result<Option<TlsAcceptor>, ProxyError> {
    let https_routes: Vec<_> = routes
        .iter()
        .filter(|r| r.request_endpoint.starts_with("https://"))
        .collect();

    if https_routes.is_empty() {
        return Ok(None);
    }

    let builder = rustls::ServerConfig::builder().with_no_client_auth();
    let mut resolver = ExactCertResolver::new();

    for route in https_routes {
        let hostname =
            extract_hostname_from_endpoint(&route.request_endpoint).ok_or_else(|| {
                ProxyError::TlsError(format!(
                    "Invalid HTTPS endpoint: {}",
                    route.request_endpoint
                ))
            })?;

        let cert_path = route.cert_path.as_ref().ok_or_else(|| {
            ProxyError::TlsError(format!(
                "Missing cert directive for route: {}",
                route.request_endpoint
            ))
        })?;
        let key_path = route.key_path.as_ref().ok_or_else(|| {
            ProxyError::TlsError(format!(
                "Missing key directive for route: {}",
                route.request_endpoint
            ))
        })?;

        let certified_key =
            load_certified_key(cert_path, key_path, builder.crypto_provider().as_ref())?;
        resolver.add(hostname.to_string(), certified_key);
    }

    let mut server_config = builder.with_cert_resolver(Arc::new(resolver));

    // Enable ALPN for HTTP/2 and HTTP/1.1
    server_config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];

    Ok(Some(TlsAcceptor::from(Arc::new(server_config))))
}
