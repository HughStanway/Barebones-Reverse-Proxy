use std::net::IpAddr;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecurityConfig {
    pub proxy_protocol: bool,
    pub trusted_upstream: IpAddr,
    pub timeout_ms: u64,
}

#[derive(Debug)]
pub struct Config {
    pub listen_port: u16,
    pub routes: Vec<Route>,
    pub certs: Vec<CertConfig>,
    pub workers: usize,
    pub logfile: Option<String>,
    pub security: Option<SecurityConfig>,
}

#[derive(Debug, Clone)]
pub struct Route {
    pub request_endpoint: String,
    pub forward_endpoint: String,
}

#[derive(Debug, Clone)]
pub struct CertConfig {
    pub hostname: String,
    pub cert_path: String,
    pub key_path: String,
}

