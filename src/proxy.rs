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
    mut req: Request<Incoming>,
) -> Result<Response<BoxBody<Bytes, BoxError>>, BoxError> {
    let active_config = state.config_reader.load();

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
        .unwrap_or("");

    let path_and_query = req
        .uri()
        .path_and_query()
        .map(|pq| pq.as_str())
        .unwrap_or("/");

    let path = req.uri().path();

    let matched = active_config.router.match_route(host, path);

    match matched {
        Some(matched_route) => {
            // Detect upgrade requests (WebSocket, etc.) before we borrow/move
            // anything from `req` that would prevent calling hyper::upgrade::on.
            let upgrade_requested = is_upgrade_request(&req);

            let upgrade_protocol = if upgrade_requested {
                req.headers()
                    .get(hyper::header::UPGRADE)
                    .and_then(|v| v.to_str().ok())
                    .map(|s| s.to_string())
            } else {
                None
            };

            if upgrade_requested {
                crate::log_info!(
                    "routing_upgrade_request",
                    "config_generation" => active_config.generation,
                    "peer" => peer_addr,
                    "method" => req.method(),
                    "host" => host,
                    "path" => path_and_query,
                    "upstream" => matched_route.upstream_addr,
                    "protocol" => upgrade_protocol.as_deref().unwrap_or("unknown")
                );
            } else {
                crate::log_info!(
                    "routing_request",
                    "config_generation" => active_config.generation,
                    "peer" => peer_addr,
                    "method" => req.method(),
                    "host" => host,
                    "path" => path_and_query,
                    "upstream" => matched_route.upstream_addr
                );
            }

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

            let upstream_uri: hyper::Uri = upstream_uri
                .parse()
                .map_err(|e: hyper::http::uri::InvalidUri| Box::new(e) as BoxError)?;

            // Resolve the original browser-facing host.
            // HTTP/1.1 typically provides Host.
            // HTTP/2 typically provides :authority, which is exposed via req.uri().authority().
            let original_host = if let Some(host) = req.headers().get(hyper::header::HOST) {
                Some(host.clone())
            } else if let Some(authority) = req.uri().authority() {
                Some(
                    hyper::header::HeaderValue::from_str(authority.as_str())
                        .map_err(|e| Box::new(e) as BoxError)?,
                )
            } else {
                None
            };

            // Capture the client-side upgrade future BEFORE consuming the request body.
            // hyper::upgrade::on takes &mut Request and registers a one-shot channel
            // so hyper can hand off the underlying connection once the 101 response
            // has been flushed to the client.
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
            // Skip Host because we want to control exactly what gets forwarded.
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
                // TODO: derive this dynamically instead.
                headers.insert(
                    hyper::header::HeaderName::from_static("x-forwarded-proto"),
                    hyper::header::HeaderValue::from_static("https"),
                );

                // Preserve/append X-Forwarded-For like a normal reverse proxy.
                let client_ip = peer_addr.ip().to_string();

                if let Some(existing) = req.headers().get("x-forwarded-for") {
                    let existing_str = existing.to_str().map_err(|e| Box::new(e) as BoxError)?;
                    let combined = format!("{}, {}", existing_str, client_ip);
                    headers.insert(
                        hyper::header::HeaderName::from_static("x-forwarded-for"),
                        hyper::header::HeaderValue::from_str(&combined)
                            .map_err(|e| Box::new(e) as BoxError)?,
                    );
                } else {
                    headers.insert(
                        hyper::header::HeaderName::from_static("x-forwarded-for"),
                        hyper::header::HeaderValue::from_str(&client_ip)
                            .map_err(|e| Box::new(e) as BoxError)?,
                    );
                }

                headers.insert(
                    hyper::header::HeaderName::from_static("x-real-ip"),
                    hyper::header::HeaderValue::from_str(&client_ip)
                        .map_err(|e| Box::new(e) as BoxError)?,
                );
            }

            let final_req = forwarded_req.body(req.into_body())?;

            match state.client.request(final_req).await {
                Ok(mut resp) => {
                    // If we sent an upgrade request and the upstream agreed (101),
                    // bridge the two upgraded connections with a bidirectional tunnel.
                    if resp.status() == StatusCode::SWITCHING_PROTOCOLS
                        && let Some(client_upgrade) = client_upgrade
                    {
                        let upstream_upgrade = hyper::upgrade::on(&mut resp);
                        let upstream_addr = matched_route.upstream_addr.clone();

                        crate::log_info!(
                            "upgrade_switching_protocols",
                            "peer" => peer_addr,
                            "upstream" => upstream_addr,
                            "protocol" => upgrade_protocol.as_deref().unwrap_or("unknown")
                        );

                        // Spawn the bidirectional copy onto the current worker's
                        // LocalSet so it stays pinned to this thread — no cross-thread
                        // synchronisation overhead.
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

                        // Return the 101 response to the client so hyper flushes it
                        // and hands off the connection to our upgrade future.
                        let (parts, body) = resp.into_parts();
                        let boxed_body = body.map_err(|e| Box::new(e) as BoxError).boxed();
                        return Ok(Response::from_parts(parts, boxed_body));
                    }

                    let (parts, body) = resp.into_parts();
                    let boxed_body = body.map_err(|e| Box::new(e) as BoxError).boxed();
                    Ok(Response::from_parts(parts, boxed_body))
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
    }
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
