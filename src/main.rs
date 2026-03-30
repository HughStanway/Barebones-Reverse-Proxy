use std::fs;

use barebones_reverse_proxy::parser::parse_proxy_config;

fn main() {
    let contents = fs::read_to_string("proxy.conf").expect("Failed to read proxy config");
    let config = parse_proxy_config(&contents).expect("Failed to parse proxy config");

    println!("{:#?}", config);
}
