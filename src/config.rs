#[derive(Debug)]
pub struct Config {
    pub listen_port: u16,
    pub routes: Vec<Route>,
}

#[derive(Debug)]
pub struct Route {
    pub request_endpoint: String,
    pub forward_endpoint: String,
}
