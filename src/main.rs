use std::fs;

use barebones_reverse_proxy::parser::parse_proxy_config;
use barebones_reverse_proxy::server::Server;

fn main() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    let contents = fs::read_to_string("proxy.conf").expect("Failed to read proxy config");
    let config = parse_proxy_config(&contents).expect("Failed to parse proxy config");

    println!("{:#?}", config);
    barebones_reverse_proxy::log_info!("server_startup", "listen_port" => config.listen_port, "workers" => config.workers);

    let server = Server::new(config);
    server.start();
}
