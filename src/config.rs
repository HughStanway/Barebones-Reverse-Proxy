#[derive(Debug)]
pub struct Config {
    pub listen_port: u16,
    pub routes: Vec<Route>,
    pub tls: Option<TlsConfig>,
    pub workers: usize,
}

#[derive(Debug)]
pub struct Route {
    pub request_endpoint: String,
    pub forward_endpoint: String,
}

#[derive(Debug)]
pub struct TlsConfig {
    pub cert_path: String,
    pub key_path: String,
}
