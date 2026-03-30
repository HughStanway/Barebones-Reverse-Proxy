use crate::config::Config;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

pub struct Server {
    config: Arc<Config>,
}

impl Server {
    pub fn new(config: Config) -> Self {
        Server {
            config: Arc::new(config),
        }
    }

    pub async fn start(&self) -> std::io::Result<()> {
        let addr = format!("0.0.0.0:{}", self.config.listen_port);
        let listener = TcpListener::bind(&addr).await?;
        println!("Server listening on {}", addr);

        loop {
            let (stream, peer_addr) = listener.accept().await?;
            let config = Arc::clone(&self.config);

            tokio::spawn(async move {
                if let Err(e) = handle_connection(stream, peer_addr, config).await {
                    eprintln!("Error handling connection from {}: {}", peer_addr, e);
                }
            });
        }
    }
}

async fn handle_connection(
    mut client_stream: TcpStream,
    peer_addr: SocketAddr,
    config: Arc<Config>,
) -> std::io::Result<()> {
    // Read the initial data from the client to determine the route
    let mut buffer = [0; 4096];
    let bytes_read = client_stream.read(&mut buffer).await?;
    if bytes_read == 0 {
        return Ok(()); // Connection closed by client
    }

    let request_data = &buffer[..bytes_read];

    // Try to parse basic HTTP headers to find the Host and Path
    let mut headers = [httparse::EMPTY_HEADER; 64];
    let mut req = httparse::Request::new(&mut headers);

    // We only need a partial parse to get the route
    let parse_result = req.parse(request_data).unwrap_or(httparse::Status::Partial);
    let headers_len = match parse_result {
        httparse::Status::Complete(len) => len,
        _ => {
            // Buffer was too small or request invalid, but for a simple proxy we assume Complete in 4kb
            return Ok(());
        }
    };

    let host = req
        .headers
        .iter()
        .find(|h| h.name.eq_ignore_ascii_case("Host"))
        .map(|h| String::from_utf8_lossy(h.value).to_string());
    let path = req.path.unwrap_or("/");

    let mut matched_route = None;

    if let Some(ref host_str) = host {
        // We preserve the port to correctly match against config routes like `http://localhost:8080/...`
        let http_url = format!("http://{}{}", host_str, path);
        let https_url = format!("https://{}{}", host_str, path);

        for route in &config.routes {
            if http_url.starts_with(&route.request_endpoint)
                || https_url.starts_with(&route.request_endpoint)
            {
                matched_route = Some(route);
                break;
            }
        }
    }

    if let Some(route) = matched_route {
        let target_addr = extract_host_port(&route.forward_endpoint).unwrap();
        println!("Routing request from {} to {}", peer_addr, target_addr);
        if let Ok(mut target_stream) = TcpStream::connect(&target_addr).await {
            let req_path_prefix = extract_path(&route.request_endpoint);
            let forward_path_prefix = extract_path(&route.forward_endpoint);

            let new_path = if path.starts_with(&req_path_prefix) {
                format!("{}{}", forward_path_prefix, &path[req_path_prefix.len()..])
            } else {
                path.to_string()
            };

            let mut new_req_bytes = Vec::new();
            let method = req.method.unwrap_or("GET");
            new_req_bytes
                .extend_from_slice(format!("{} {} HTTP/1.1\r\n", method, new_path).as_bytes());

            let mut host_found = false;
            for header in req.headers.iter() {
                if header.name.is_empty() {
                    continue;
                }
                if header.name.eq_ignore_ascii_case("Host") {
                    new_req_bytes
                        .extend_from_slice(format!("Host: {}\r\n", target_addr).as_bytes());
                    host_found = true;
                } else {
                    new_req_bytes.extend_from_slice(format!("{}: ", header.name).as_bytes());
                    new_req_bytes.extend_from_slice(header.value);
                    new_req_bytes.extend_from_slice(b"\r\n");
                }
            }
            if !host_found {
                new_req_bytes.extend_from_slice(format!("Host: {}\r\n", target_addr).as_bytes());
            }
            new_req_bytes.extend_from_slice(b"\r\n");

            new_req_bytes.extend_from_slice(&request_data[headers_len..]);

            // Send the rewritten request
            target_stream.write_all(&new_req_bytes).await?;

            // Proxy the rest of the connection bidirectionally
            match tokio::io::copy_bidirectional(&mut client_stream, &mut target_stream).await {
                Ok((from_client, from_server)) => {
                    println!(
                        "Client wrote {} bytes and received {} bytes",
                        from_client, from_server
                    );
                }
                Err(e) => {
                    eprintln!("Error copying data bidirectionally: {}", e);
                }
            }
        } else {
            eprintln!("Failed to connect to forward endpoint: {}", target_addr);
            // Send 502 Bad Gateway
            let response = "HTTP/1.1 502 Bad Gateway\r\n\r\n502 Bad Gateway";
            let _ = client_stream.write_all(response.as_bytes()).await;
        }
    } else {
        eprintln!(
            "No matching route found for request from {} (Host: {}, Path: {})",
            peer_addr,
            host.as_deref().unwrap_or_default(),
            path
        );
        // Send 404 Not Found
        let response = "HTTP/1.1 404 Not Found\r\n\r\n404 Not Found";
        let _ = client_stream.write_all(response.as_bytes()).await;
    }

    Ok(())
}

fn extract_host_port(endpoint: &str) -> Option<String> {
    let without_scheme = endpoint
        .strip_prefix("http://")
        .or_else(|| endpoint.strip_prefix("https://"))
        .unwrap_or(endpoint);

    let host_port = without_scheme.split('/').next().unwrap_or(without_scheme);

    if host_port.contains(':') {
        Some(host_port.to_string())
    } else if endpoint.starts_with("https://") {
        Some(format!("{}:443", host_port))
    } else {
        Some(format!("{}:80", host_port))
    }
}

fn extract_path(endpoint: &str) -> String {
    let without_scheme = endpoint
        .strip_prefix("http://")
        .or_else(|| endpoint.strip_prefix("https://"))
        .unwrap_or(endpoint);

    if let Some(idx) = without_scheme.find('/') {
        without_scheme[idx..].to_string()
    } else {
        "/".to_string()
    }
}
