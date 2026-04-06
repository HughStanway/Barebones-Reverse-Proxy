use std::path::Path;

use barebones_reverse_proxy::runtime_config::load_config_from_path;
use barebones_reverse_proxy::server::Server;

fn main() {
    const CONFIG_PATH: &str = "proxy.conf";

    let _ = rustls::crypto::ring::default_provider().install_default();

    let config =
        load_config_from_path(Path::new(CONFIG_PATH)).expect("Failed to load proxy config");

    barebones_reverse_proxy::log_info!("server_startup", "listen_port" => config.listen_port, "workers" => config.workers);

    let server = Server::new(config, CONFIG_PATH).expect("Failed to initialise server");
    server.start();
}
