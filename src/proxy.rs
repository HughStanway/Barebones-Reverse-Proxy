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
            if let Some(header_val) = req.headers().get(*header_name) {
                if let Ok(header_str) = header_val.to_str() {
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

    let matched = active_config.router.match_route(&host, &path);

    let result = match &matched {
        Some(matched_route) => {
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

                        tokio::task::spawn_local(async move {
                            match tokio::try_join!(client_upgrade, upstream_upgrade) {
                                Ok((client_stream, upstream_stream)) => {
                                    let mut client_io = TokioIo::new(client_stream);
                                    let mut upstream_io = TokioIo::new(upstream_stream);

                                    let _ = tokio::io::copy_bidirectional(
                                        &mut client_io,
                                        &mut upstream_io,
                                    )
                                    .await;
                                }
                                Err(_) => {}
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
            crate::log_error!(
                "no_matching_route",
                "config_generation" => active_config.generation,
                "peer" => peer_addr,
                "host" => host,
                "path" => path_and_query
            );
            Ok(error_response(StatusCode::NOT_FOUND, "404 Not Found"))
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

    result
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
