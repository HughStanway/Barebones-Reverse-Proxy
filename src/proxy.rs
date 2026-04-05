use crate::router::Router;
use http_body_util::combinators::BoxBody;
use http_body_util::{BodyExt, Full};
use hyper::body::{Bytes, Incoming};
use hyper::{Request, Response, StatusCode};
use hyper_util::client::legacy::Client;
use hyper_util::rt::TokioExecutor;
use std::net::SocketAddr;
use std::sync::Arc;

type BoxError = Box<dyn std::error::Error + Send + Sync>;
type HttpClient = Client<
    hyper_rustls::HttpsConnector<hyper_util::client::legacy::connect::HttpConnector>,
    Incoming,
>;

/// Shared state across all connections, cloned per-request.
#[derive(Clone)]
pub struct ProxyState {
    pub router: Arc<Router>,
    pub client: HttpClient,
}

impl ProxyState {
    pub fn new(router: Arc<Router>) -> Self {
        let https_connector = hyper_rustls::HttpsConnectorBuilder::new()
            .with_native_roots()
            .expect("Failed to load native root certificates")
            .https_or_http()
            .enable_http1()
            .build();

        let client: HttpClient = Client::builder(TokioExecutor::new()).build(https_connector);

        ProxyState { router, client }
    }
}

/// Handle an incoming HTTP request by routing and proxying it upstream.
pub async fn handle_request(
    state: ProxyState,
    peer_addr: SocketAddr,
    req: Request<Incoming>,
) -> Result<Response<BoxBody<Bytes, BoxError>>, BoxError> {
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

    let matched = state.router.match_route(host, path);

    match matched {
        Some(matched_route) => {
            crate::log_info!(
                "routing_request",
                "peer" => peer_addr,
                "method" => req.method(),
                "host" => host,
                "path" => path_and_query,
                "upstream" => matched_route.upstream_addr
            );

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
                Ok(resp) => {
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
