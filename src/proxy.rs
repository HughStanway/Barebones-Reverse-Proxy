use crate::runtime_config::ConfigReader;
use http_body_util::combinators::BoxBody;
use http_body_util::{BodyExt, Full};
use hyper::body::{Bytes, Incoming};
use hyper::{Request, Response, StatusCode};
use hyper_util::client::legacy::Client;
use hyper_util::rt::{TokioExecutor, TokioIo};
use std::net::SocketAddr;
type BoxError = Box<dyn std::error::Error + Send + Sync>;
type HttpClient = Client<
    hyper_rustls::HttpsConnector<hyper_util::client::legacy::connect::HttpConnector>,
    Incoming,
>;

/// Shared state across all connections, cloned per-request.
#[derive(Clone)]
pub struct ProxyState {
    pub config_reader: ConfigReader,
    pub client: HttpClient,
}

impl ProxyState {
    pub fn new(config_reader: ConfigReader) -> Self {
        let https_connector = hyper_rustls::HttpsConnectorBuilder::new()
            .with_native_roots()
            .expect("Failed to load native root certificates")
            .https_or_http()
            .enable_http1()
            .build();

        let client: HttpClient = Client::builder(TokioExecutor::new()).build(https_connector);

        ProxyState {
            config_reader,
            client,
        }
    }
}

/// Check whether an incoming request is an HTTP Upgrade request.
///
/// Returns `true` when the `Connection` header contains the token `upgrade`
/// (case-insensitive) **and** an `Upgrade` header is present, following
/// RFC 7230 §6.7 semantics.
fn is_upgrade_request(req: &Request<Incoming>) -> bool {
    let has_upgrade_header = req.headers().contains_key(hyper::header::UPGRADE);

    let connection_wants_upgrade = req
        .headers()
        .get(hyper::header::CONNECTION)
        .and_then(|v| v.to_str().ok())
        .map(|v| {
            v.split(',')
                .any(|token| token.trim().eq_ignore_ascii_case("upgrade"))
        })
        .unwrap_or(false);

    has_upgrade_header && connection_wants_upgrade
}

/// Handle an incoming HTTP request by routing and proxying it upstream.
pub async fn handle_request(
    state: ProxyState,
    peer_addr: SocketAddr,
    is_proxy_protocol: bool,
    mut req: Request<Incoming>,
) -> Result<Response<BoxBody<Bytes, BoxError>>, BoxError> {
    let start_instant = std::time::Instant::now();
    let active_config = state.config_reader.load();

    let mut client_ip = peer_addr.ip().to_string();

    // Automatically extract true client IP from standard proxy/CDN headers
    // ONLY if the connection was proxied via the trusted upstream.
    if is_proxy_protocol {
        for header_name in &["cf-connecting-ip", "true-client-ip", "x-forwarded-for"] {
            if let Some(header_val) = req.headers().get(*header_name)
                && let Ok(header_str) = header_val.to_str()
            {
                let ip_part = if *header_name == "x-forwarded-for" {
                    header_str.split(',').next().unwrap_or(header_str).trim()
                } else {
                    header_str.trim()
                };
                if ip_part.parse::<std::net::IpAddr>().is_ok() {
                    client_ip = ip_part.to_string();
                    break;
                }
            }
        }
    }

    // Robust host extraction: check URI authority (H2) then fallback to Host header (H1)
    let host = req
        .uri()
        .authority()
        .map(|a| a.as_str())
        .or_else(|| {
            req.headers()
                .get(hyper::header::HOST)
                .and_then(|v| v.to_str().ok())
        })
        .unwrap_or("")
        .to_string();

    let path_and_query = req
        .uri()
        .path_and_query()
        .map(|pq| pq.as_str())
        .unwrap_or("/")
        .to_string();

    let path = req.uri().path().to_string();

    let user_agent = req
        .headers()
        .get(hyper::header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("-")
        .to_string();

    let referer = req
        .headers()
        .get(hyper::header::REFERER)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("-")
        .to_string();

    let http_version = format!("{:?}", req.version());
    let method = req.method().to_string();

    if let Ok(ip_addr) = client_ip.parse::<std::net::IpAddr>() {
        if active_config.security_manager.is_blacklisted(&ip_addr) {
            crate::log_info!(
                "connection_dropped_blacklisted",
                "client_ip" => client_ip,
                "host" => host,
                "path" => path_and_query
            );
            return Ok(no_response_444());
        }

        if let Err(count) = active_config
            .security_manager
            .check_and_record_request(ip_addr)
        {
            crate::log_error!(
                "rate_limit_exceeded",
                "client_ip" => client_ip,
                "path" => path,
                "host" => host,
                "count" => count,
                "limit" => active_config.security.as_ref().map(|s| s.rate_limit_rpm).unwrap_or(60)
            );
            let duration_ms = start_instant.elapsed().as_secs_f64() * 1000.0;
            crate::log_info!(
                "request",
                "peer" => peer_addr,
                "client_ip" => client_ip,
                "method" => method,
                "host" => host,
                "path" => path_and_query,
                "version" => http_version,
                "status" => 429,
                "duration_ms" => format!("{:.3}", duration_ms),
                "upstream" => "-",
                "user_agent" => user_agent,
                "referer" => referer
            );
            let mut resp = error_response(StatusCode::TOO_MANY_REQUESTS, "429 Too Many Requests");
            resp.headers_mut().insert(
                hyper::header::RETRY_AFTER,
                hyper::header::HeaderValue::from_static("60"),
            );
            return Ok(resp);
        }
    }

    let matched = active_config.router.match_route(&host, &path);

    let mut result = match &matched {
        Some(matched_route) => {
            let mut auth_headers = hyper::HeaderMap::new();

            if matched_route.auth_required {
                if let Some(ref auth_provider) = active_config.auth_provider {
                    match auth_provider
                        .authenticate(
                            &peer_addr.to_string(),
                            &client_ip,
                            &method,
                            req.uri(),
                            req.headers(),
                        )
                        .await
                    {
                        Ok(crate::auth::AuthResult::Success { headers }) => {
                            if !headers.is_empty() {
                                crate::log_debug!(
                                    "auth_headers_injected",
                                    "client_ip" => client_ip,
                                    "host" => host,
                                    "count" => headers.len()
                                );
                            }
                            auth_headers = headers;
                        }
                        Ok(crate::auth::AuthResult::Denied { response }) => {
                            crate::log_info!(
                                "auth_denied",
                                "client_ip" => client_ip,
                                "host" => host,
                                "path" => path_and_query,
                                "status" => response.status().as_u16()
                            );
                            return Ok(response);
                        }
                        Err(e) => {
                            crate::log_error!(
                                "auth_error",
                                "client_ip" => client_ip,
                                "error" => e
                            );
                            return Ok(error_response(
                                StatusCode::INTERNAL_SERVER_ERROR,
                                "500 Internal Server Error",
                            ));
                        }
                    }
                } else {
                    crate::log_error!(
                        "auth_misconfiguration",
                        "client_ip" => client_ip,
                        "host" => host,
                        "error" => "route requires authentication ('auth on;') but no 'forward_auth' URL is configured in security block"
                    );
                    return Ok(error_response(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "500 Internal Server Error - Forward Auth Provider Not Configured",
                    ));
                }
            }

            // Detect upgrade requests (WebSocket, etc.) before we borrow/move
            // anything from `req` that would prevent calling hyper::upgrade::on.
            let upgrade_requested = is_upgrade_request(&req);

            // Build the rewritten path, preserving the original query string
            let rewritten_path_and_query = if let Some(query) = req.uri().query() {
                format!("{}?{}", matched_route.rewritten_path, query)
            } else {
                matched_route.rewritten_path.clone()
            };

            let upstream_uri = format!(
                "http://{}{}",
                matched_route.upstream_addr, rewritten_path_and_query
            );

            let upstream_uri: hyper::Uri = match upstream_uri.parse() {
                Ok(uri) => uri,
                Err(e) => return Err(Box::new(e) as BoxError),
            };

            // Resolve the original browser-facing host.
            let original_host = if let Some(host) = req.headers().get(hyper::header::HOST) {
                Some(host.clone())
            } else if let Some(authority) = req.uri().authority() {
                match hyper::header::HeaderValue::from_str(authority.as_str()) {
                    Ok(val) => Some(val),
                    Err(e) => return Err(Box::new(e) as BoxError),
                }
            } else {
                None
            };

            // Capture the client-side upgrade future BEFORE consuming the request body.
            let client_upgrade = if upgrade_requested {
                Some(hyper::upgrade::on(&mut req))
            } else {
                None
            };

            let upgrade_protocol = if upgrade_requested {
                req.headers()
                    .get(hyper::header::UPGRADE)
                    .and_then(|v| v.to_str().ok())
                    .map(|s| s.to_string())
            } else {
                None
            };

            // Build the forwarded request
            let mut forwarded_req = Request::builder()
                .method(req.method().clone())
                .uri(&upstream_uri)
                .version(hyper::Version::HTTP_11);

            // Copy headers from the original request.
            if let Some(headers) = forwarded_req.headers_mut() {
                for (key, value) in req.headers() {
                    if key != hyper::header::HOST && !key.as_str().starts_with(':') {
                        headers.append(key, value.clone());
                    }
                }

                for (k, v) in auth_headers.iter() {
                    headers.insert(k.clone(), v.clone());
                }

                // Preserve the original browser-facing Host header.
                if let Some(host) = original_host.clone() {
                    headers.insert(hyper::header::HOST, host.clone());
                    headers.insert(
                        hyper::header::HeaderName::from_static("x-forwarded-host"),
                        host,
                    );
                }

                // Tell the upstream the original client scheme.
                headers.insert(
                    hyper::header::HeaderName::from_static("x-forwarded-proto"),
                    hyper::header::HeaderValue::from_static("https"),
                );

                // Preserve/append X-Forwarded-For like a normal reverse proxy.
                if let Some(existing) = req.headers().get("x-forwarded-for") {
                    let existing_str = match existing.to_str() {
                        Ok(s) => s,
                        Err(e) => return Err(Box::new(e) as BoxError),
                    };
                    let combined = format!("{}, {}", existing_str, client_ip);
                    headers.insert(
                        hyper::header::HeaderName::from_static("x-forwarded-for"),
                        match hyper::header::HeaderValue::from_str(&combined) {
                            Ok(val) => val,
                            Err(e) => return Err(Box::new(e) as BoxError),
                        },
                    );
                } else {
                    headers.insert(
                        hyper::header::HeaderName::from_static("x-forwarded-for"),
                        match hyper::header::HeaderValue::from_str(&client_ip) {
                            Ok(val) => val,
                            Err(e) => return Err(Box::new(e) as BoxError),
                        },
                    );
                }

                headers.insert(
                    hyper::header::HeaderName::from_static("x-real-ip"),
                    match hyper::header::HeaderValue::from_str(&client_ip) {
                        Ok(val) => val,
                        Err(e) => return Err(Box::new(e) as BoxError),
                    },
                );
            }

            let final_req = match forwarded_req.body(req.into_body()) {
                Ok(req) => req,
                Err(e) => return Err(Box::new(e) as BoxError),
            };

            match state.client.request(final_req).await {
                Ok(mut resp) => {
                    // If we sent an upgrade request and the upstream agreed (101),
                    // bridge the two upgraded connections with a bidirectional tunnel.
                    if resp.status() == StatusCode::SWITCHING_PROTOCOLS
                        && let Some(client_upgrade) = client_upgrade
                    {
                        let upstream_upgrade = hyper::upgrade::on(&mut resp);
                        let upstream_addr = matched_route.upstream_addr.clone();
                        let protocol_str = upgrade_protocol
                            .clone()
                            .unwrap_or_else(|| "unknown".to_string());

                        crate::log_info!(
                            "upgrade_switching_protocols",
                            "peer" => peer_addr,
                            "upstream" => upstream_addr,
                            "protocol" => protocol_str
                        );

                        tokio::task::spawn_local(async move {
                            match tokio::try_join!(client_upgrade, upstream_upgrade) {
                                Ok((client_stream, upstream_stream)) => {
                                    let mut client_io = TokioIo::new(client_stream);
                                    let mut upstream_io = TokioIo::new(upstream_stream);

                                    match tokio::io::copy_bidirectional(
                                        &mut client_io,
                                        &mut upstream_io,
                                    )
                                    .await
                                    {
                                        Ok((to_upstream, to_client)) => {
                                            crate::log_info!(
                                                "upgrade_tunnel_closed",
                                                "peer" => peer_addr,
                                                "upstream" => upstream_addr,
                                                "bytes_to_upstream" => to_upstream,
                                                "bytes_to_client" => to_client
                                            );
                                        }
                                        Err(e) => {
                                            crate::log_error!(
                                                "upgrade_tunnel_error",
                                                "peer" => peer_addr,
                                                "upstream" => upstream_addr,
                                                "error" => e
                                            );
                                        }
                                    }
                                }
                                Err(e) => {
                                    crate::log_error!(
                                        "upgrade_handshake_failed",
                                        "peer" => peer_addr,
                                        "upstream" => upstream_addr,
                                        "error" => e
                                    );
                                }
                            }
                        });

                        let (parts, body) = resp.into_parts();
                        let boxed_body = body.map_err(|e| Box::new(e) as BoxError).boxed();
                        Ok(Response::from_parts(parts, boxed_body))
                    } else {
                        let (parts, body) = resp.into_parts();
                        let boxed_body = body.map_err(|e| Box::new(e) as BoxError).boxed();
                        Ok(Response::from_parts(parts, boxed_body))
                    }
                }
                Err(e) => {
                    crate::log_error!(
                        "upstream_connect_failed",
                        "upstream" => matched_route.upstream_addr,
                        "error" => e
                    );
                    Ok(error_response(StatusCode::BAD_GATEWAY, "502 Bad Gateway"))
                }
            }
        }
        None => {
            crate::log_info!(
                "no_matching_route",
                "config_generation" => active_config.generation,
                "peer" => peer_addr,
                "host" => host,
                "path" => path_and_query
            );
            Ok(no_response_444())
        }
    };

    let duration_ms = start_instant.elapsed().as_secs_f64() * 1000.0;

    let (status_code, upstream_str) = match &result {
        Ok(resp) => {
            let upstream_addr = matched
                .as_ref()
                .map(|m| m.upstream_addr.as_str())
                .unwrap_or("-");
            (resp.status().as_u16(), upstream_addr)
        }
        Err(_) => (500, "-"),
    };

    crate::log_info!(
        "request",
        "peer" => peer_addr,
        "client_ip" => client_ip,
        "method" => method,
        "host" => host,
        "path" => path_and_query,
        "version" => http_version,
        "status" => status_code,
        "duration_ms" => format!("{:.3}", duration_ms),
        "upstream" => upstream_str,
        "user_agent" => user_agent,
        "referer" => referer
    );

    if let Ok(ref mut resp) = result {
        inject_security_headers(resp.headers_mut());
    }

    result
}

fn inject_security_headers(headers: &mut hyper::HeaderMap) {
    headers.insert(
        hyper::header::HeaderName::from_static("strict-transport-security"),
        hyper::header::HeaderValue::from_static("max-age=63072000; includeSubDomains; preload"),
    );
    headers.insert(
        hyper::header::HeaderName::from_static("x-content-type-options"),
        hyper::header::HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        hyper::header::HeaderName::from_static("x-frame-options"),
        hyper::header::HeaderValue::from_static("DENY"),
    );
    headers.insert(
        hyper::header::HeaderName::from_static("x-xss-protection"),
        hyper::header::HeaderValue::from_static("0"),
    );
}

fn no_response_444() -> Response<BoxBody<Bytes, BoxError>> {
    let status = StatusCode::from_u16(444).unwrap_or(StatusCode::NOT_FOUND);
    Response::builder()
        .status(status)
        .header(hyper::header::CONNECTION, "close")
        .body(
            Full::new(Bytes::new())
                .map_err(|e| Box::new(e) as BoxError)
                .boxed(),
        )
        .unwrap()
}

fn error_response(status: StatusCode, body: &str) -> Response<BoxBody<Bytes, BoxError>> {
    Response::builder()
        .status(status)
        .body(
            Full::new(Bytes::from(body.to_string()))
                .map_err(|e| Box::new(e) as BoxError)
                .boxed(),
        )
        .unwrap()
}
