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
            println!(
                "Routing request from {} to {}",
                peer_addr, matched_route.upstream_addr
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

            // Build the forwarded request
            let mut forwarded_req = Request::builder()
                .method(req.method().clone())
                .uri(&upstream_uri)
                .version(hyper::Version::HTTP_11);

            // Copy all headers from the original request, except H2 pseudo-headers
            if let Some(headers) = forwarded_req.headers_mut() {
                for (key, value) in req.headers() {
                    if !key.as_str().starts_with(':') {
                        headers.append(key, value.clone());
                    }
                }
            }

            let mut final_req = forwarded_req.body(req.into_body())?;

            // 1. Overwrite Host header with the target upstream authority
            final_req.headers_mut().insert(
                hyper::header::HOST,
                matched_route.upstream_addr.parse().map_err(|e| Box::new(e) as BoxError)?
            );

            // 3. Add proxy headers
            final_req.headers_mut().insert(
                "X-Forwarded-For",
                peer_addr.ip().to_string().parse().map_err(|e| Box::new(e) as BoxError)?
            );
            final_req.headers_mut().insert(
                "X-Real-IP",
                peer_addr.ip().to_string().parse().map_err(|e| Box::new(e) as BoxError)?
            );

            // Debug: print headers being sent to upstream
            println!("Forwarding to {} (Host: {}):", upstream_uri, matched_route.upstream_addr);
            for (key, value) in final_req.headers() {
                println!("  {}: {:?}", key, value);
            }

            match state.client.request(final_req).await {
                Ok(resp) => {
                    let (parts, body) = resp.into_parts();
                    let boxed_body = body.map_err(|e| Box::new(e) as BoxError).boxed();
                    Ok(Response::from_parts(parts, boxed_body))
                }
                Err(e) => {
                    eprintln!(
                        "Failed to connect to upstream {}: {}",
                        matched_route.upstream_addr, e
                    );
                    Ok(error_response(StatusCode::BAD_GATEWAY, "502 Bad Gateway"))
                }
            }
        }
        None => {
            eprintln!(
                "No matching route for request from {} (Host: {}, Path: {})",
                peer_addr, host, path_and_query
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
