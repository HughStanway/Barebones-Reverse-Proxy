use http_body_util::combinators::BoxBody;
use http_body_util::{BodyExt, Full};
use hyper::body::Bytes;
use hyper::{HeaderMap, Request, Response, Uri};
use hyper_util::client::legacy::Client;
use std::fmt::Debug;
use std::future::Future;
use std::pin::Pin;

pub type BoxError = Box<dyn std::error::Error + Send + Sync>;

pub type HttpClient = Client<
    hyper_rustls::HttpsConnector<hyper_util::client::legacy::connect::HttpConnector>,
    BoxBody<Bytes, BoxError>,
>;

#[derive(Debug)]
pub enum AuthResult {
    /// Authentication succeeded. Contains response headers returned by the provider
    /// (e.g. Remote-User, Remote-Groups, Remote-Email) to inject into upstream requests.
    Success { headers: HeaderMap },

    /// Authentication failed or redirect required. Contains the response payload
    /// (e.g. 302 Redirect to Authelia login portal, 401 Unauthorized, or 403 Forbidden).
    Denied {
        response: Response<BoxBody<Bytes, BoxError>>,
    },
}

pub trait AuthProvider: Send + Sync + Debug {
    /// Authenticates an incoming HTTP request.
    fn authenticate<'a>(
        &'a self,
        peer_addr: &'a str,
        client_ip: &'a str,
        method: &'a str,
        uri: &'a Uri,
        headers: &'a HeaderMap,
    ) -> Pin<Box<dyn Future<Output = Result<AuthResult, BoxError>> + Send + 'a>>;
}

/// A Forward Auth provider implementation (e.g., Authelia / Authentik / OAuth2-Proxy).
///
/// Sends a lightweight internal sub-request to `auth_url` (e.g., `http://localhost:9091/api/verify`)
/// carrying standard `X-Forwarded-*` headers, session cookies, and Authorization headers.
#[derive(Clone)]
pub struct ForwardAuthProvider {
    auth_url: String,
    client: HttpClient,
}

impl ForwardAuthProvider {
    pub fn new(auth_url: String, client: HttpClient) -> Self {
        ForwardAuthProvider { auth_url, client }
    }
}

impl Debug for ForwardAuthProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ForwardAuthProvider")
            .field("auth_url", &self.auth_url)
            .finish()
    }
}

impl AuthProvider for ForwardAuthProvider {
    fn authenticate<'a>(
        &'a self,
        _peer_addr: &'a str,
        client_ip: &'a str,
        method: &'a str,
        uri: &'a Uri,
        headers: &'a HeaderMap,
    ) -> Pin<Box<dyn Future<Output = Result<AuthResult, BoxError>> + Send + 'a>> {
        Box::pin(async move {
            let auth_uri: Uri = self.auth_url.parse()?;

            let host = uri
                .authority()
                .map(|a| a.as_str())
                .or_else(|| {
                    headers
                        .get(hyper::header::HOST)
                        .and_then(|v| v.to_str().ok())
                })
                .unwrap_or("");

            let path_and_query = uri.path_and_query().map(|pq| pq.as_str()).unwrap_or("/");

            let mut sub_req_builder = Request::builder()
                .method(method)
                .uri(&auth_uri)
                .header("x-forwarded-method", method)
                .header("x-forwarded-proto", "https")
                .header("x-forwarded-host", host)
                .header("x-forwarded-uri", path_and_query)
                .header("x-forwarded-for", client_ip);

            // Forward session cookies, authorization headers, and API keys to the auth provider
            for header_name in &[
                hyper::header::COOKIE.as_str(),
                hyper::header::AUTHORIZATION.as_str(),
                "x-api-key",
                "accept",
            ] {
                if let Some(val) = headers.get(*header_name) {
                    sub_req_builder = sub_req_builder.header(*header_name, val);
                }
            }

            let sub_req = sub_req_builder.body(
                Full::new(Bytes::new())
                    .map_err(|e| Box::new(e) as BoxError)
                    .boxed(),
            )?;

            match self.client.request(sub_req).await {
                Ok(resp) => {
                    let status = resp.status();
                    if status.is_success() {
                        let mut success_headers = HeaderMap::new();
                        for (k, v) in resp.headers().iter() {
                            let k_str = k.as_str();
                            if k_str.starts_with("remote-")
                                || k_str.starts_with("x-forwarded-user")
                                || k_str.starts_with("x-auth-")
                            {
                                success_headers.insert(k.clone(), v.clone());
                            }
                        }
                        Ok(AuthResult::Success {
                            headers: success_headers,
                        })
                    } else {
                        let (parts, body) = resp.into_parts();
                        let boxed_body = body.map_err(|e| Box::new(e) as BoxError).boxed();
                        let denied_resp = Response::from_parts(parts, boxed_body);
                        Ok(AuthResult::Denied {
                            response: denied_resp,
                        })
                    }
                }
                Err(e) => Err(Box::new(e) as BoxError),
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hyper::{HeaderMap, StatusCode};

    #[derive(Debug)]
    struct MockAuthProvider {
        allow: bool,
    }

    impl AuthProvider for MockAuthProvider {
        fn authenticate<'a>(
            &'a self,
            _peer_addr: &'a str,
            _client_ip: &'a str,
            _method: &'a str,
            _uri: &'a Uri,
            _headers: &'a HeaderMap,
        ) -> Pin<Box<dyn Future<Output = Result<AuthResult, BoxError>> + Send + 'a>> {
            Box::pin(async move {
                if self.allow {
                    let mut h = HeaderMap::new();
                    h.insert("remote-user", "testuser".parse().unwrap());
                    Ok(AuthResult::Success { headers: h })
                } else {
                    let resp = Response::builder()
                        .status(StatusCode::FOUND)
                        .header("location", "https://auth.example.com/login")
                        .body(
                            Full::new(Bytes::new())
                                .map_err(|e| Box::new(e) as BoxError)
                                .boxed(),
                        )
                        .unwrap();
                    Ok(AuthResult::Denied { response: resp })
                }
            })
        }
    }

    #[tokio::test]
    async fn test_mock_auth_provider_success() {
        let provider = MockAuthProvider { allow: true };
        let uri: Uri = "https://example.local/dashboard".parse().unwrap();
        let headers = HeaderMap::new();

        let result = provider
            .authenticate("127.0.0.1:1234", "127.0.0.1", "GET", &uri, &headers)
            .await
            .unwrap();

        match result {
            AuthResult::Success { headers } => {
                assert_eq!(headers.get("remote-user").unwrap(), "testuser");
            }
            _ => panic!("Expected AuthResult::Success"),
        }
    }

    #[tokio::test]
    async fn test_mock_auth_provider_denied() {
        let provider = MockAuthProvider { allow: false };
        let uri: Uri = "https://example.local/dashboard".parse().unwrap();
        let headers = HeaderMap::new();

        let result = provider
            .authenticate("127.0.0.1:1234", "127.0.0.1", "GET", &uri, &headers)
            .await
            .unwrap();

        match result {
            AuthResult::Denied { response } => {
                assert_eq!(response.status(), StatusCode::FOUND);
                assert_eq!(
                    response.headers().get("location").unwrap(),
                    "https://auth.example.com/login"
                );
            }
            _ => panic!("Expected AuthResult::Denied"),
        }
    }
}
