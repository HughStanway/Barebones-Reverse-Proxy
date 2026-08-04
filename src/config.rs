use std::net::IpAddr;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecurityConfig {
    pub proxy_protocol: bool,
    pub trusted_upstream: IpAddr,
    pub timeout_ms: u64,
    pub max_tls_failures: usize,
    pub ban_duration_sec: u64,
    pub rate_limit_rpm: usize,
    pub forward_auth: Option<String>,
    pub max_body_size: usize,
    pub max_header_size: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CacheConfig {
    pub enabled: bool,
    pub max_capacity_bytes: usize,
    pub max_file_size_bytes: usize,
    pub default_ttl_sec: u64,
}

#[derive(Debug)]
pub struct Config {
    pub listen_port: u16,
    pub routes: Vec<Route>,
    pub workers: usize,
    pub logfile: Option<String>,
    pub security: Option<SecurityConfig>,
    pub cache: Option<CacheConfig>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Route {
    pub request_endpoint: String,
    pub forward_endpoint: String,
    pub auth_required: bool,
    pub cert_path: Option<String>,
    pub key_path: Option<String>,
    pub cache: Option<bool>,
}
