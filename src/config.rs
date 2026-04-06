#[derive(Debug)]
pub struct Config {
    pub listen_port: u16,
    pub routes: Vec<Route>,
    pub certs: Vec<CertConfig>,
    pub workers: usize,
}

#[derive(Debug)]
pub struct Route {
    pub request_endpoint: String,
    pub forward_endpoint: String,
}

#[derive(Debug)]
pub struct CertConfig {
    pub hostname: String,
    pub cert_path: String,
    pub key_path: String,
}
