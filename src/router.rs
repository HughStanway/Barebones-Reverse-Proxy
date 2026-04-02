use crate::config::Route;

/// Holds the routing table and provides route matching and path rewriting.
pub struct Router {
    routes: Vec<Route>,
}

/// Result of a successful route match, containing the upstream target address
/// and the rewritten request path.
pub struct MatchedRoute {
    pub upstream_addr: String,
    pub rewritten_path: String,
}

impl Router {
    pub fn new(routes: Vec<Route>) -> Self {
        Router { routes }
    }

    /// Match an incoming request against the route table.
    ///
    /// Constructs candidate URLs from the host and path, and checks if any
    /// route's `request_endpoint` is a prefix match.
    pub fn match_route(&self, host: &str, path: &str) -> Option<MatchedRoute> {
        let http_url = format!("http://{}{}", host, path);
        let https_url = format!("https://{}{}", host, path);

        for route in &self.routes {
            if http_url.starts_with(&route.request_endpoint)
                || https_url.starts_with(&route.request_endpoint)
            {
                let req_path_prefix = extract_path(&route.request_endpoint);
                let forward_path_prefix = extract_path(&route.forward_endpoint);

                let rewritten_path = if path.starts_with(&req_path_prefix) {
                    format!("{}{}", forward_path_prefix, &path[req_path_prefix.len()..])
                } else {
                    path.to_string()
                };

                let upstream_addr = extract_host_port(&route.forward_endpoint);

                return Some(MatchedRoute {
                    upstream_addr,
                    rewritten_path,
                });
            }
        }

        None
    }
}

/// Extract host:port from an endpoint URL, defaulting the port based on scheme.
fn extract_host_port(endpoint: &str) -> String {
    let without_scheme = endpoint
        .strip_prefix("http://")
        .or_else(|| endpoint.strip_prefix("https://"))
        .unwrap_or(endpoint);

    let host_port = without_scheme.split('/').next().unwrap_or(without_scheme);

    if host_port.contains(':') {
        host_port.to_string()
    } else if endpoint.starts_with("https://") {
        format!("{}:443", host_port)
    } else {
        format!("{}:80", host_port)
    }
}

/// Extract the path component from an endpoint URL.
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
