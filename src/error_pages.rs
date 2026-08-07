use http_body_util::combinators::BoxBody;
use http_body_util::{BodyExt, Full};
use hyper::body::Bytes;
use hyper::{HeaderMap, Response, StatusCode};

pub type BoxError = Box<dyn std::error::Error + Send + Sync>;

pub fn render_html_error_page(
    status: StatusCode,
    message: &str,
    client_ip: &str,
    host: &str,
) -> String {
    let code = status.as_u16();
    let reason = status.canonical_reason().unwrap_or("Error");
    let display_host = if host.is_empty() { "-" } else { host };
    let display_ip = if client_ip.is_empty() { "-" } else { client_ip };

    format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8">
  <meta name="viewport" content="width=device-width, initial-scale=1.0">
  <title>{code} {reason}</title>
  <style>
    * {{ box-sizing: border-box; margin: 0; padding: 0; }}
    body {{
      background-color: #f1f5f9;
      background-image: linear-gradient(rgba(15, 23, 42, 0.04) 50%, transparent 50%);
      background-size: 100% 4px;
      color: #0f172a;
      font-family: "Courier New", Courier, Monaco, "Lucida Console", monospace;
      display: flex;
      align-items: center;
      justify-content: center;
      min-height: 100vh;
      padding: 20px;
    }}
    .card {{
      background-color: #ffffff;
      border: 4px solid #0f172a;
      box-shadow: 8px 8px 0px #0f172a;
      padding: 36px 32px;
      max-width: 480px;
      width: 100%;
      text-align: center;
    }}
    .banner {{
      display: inline-block;
      background-color: #0f172a;
      color: #38bdf8;
      font-size: 11px;
      font-weight: 700;
      letter-spacing: 2px;
      padding: 4px 12px;
      margin-bottom: 20px;
      text-transform: uppercase;
      border: 2px solid #0f172a;
    }}
    .status-code {{
      font-size: 64px;
      font-weight: 900;
      line-height: 1;
      color: #dc2626;
      text-shadow: 3px 3px 0px #0f172a;
      margin-bottom: 12px;
      letter-spacing: 2px;
    }}
    .status-title {{
      font-size: 20px;
      font-weight: 700;
      color: #0f172a;
      text-transform: uppercase;
      letter-spacing: 1px;
      margin-bottom: 16px;
    }}
    .description {{
      font-size: 13px;
      color: #334155;
      line-height: 1.6;
      margin-bottom: 28px;
      font-weight: 600;
    }}
    .pixel-divider {{
      border-top: 4px dashed #0f172a;
      margin-bottom: 20px;
    }}
    .meta {{
      display: flex;
      justify-content: space-between;
      gap: 12px;
      font-size: 11px;
    }}
    .meta-box {{
      flex: 1;
      background-color: #f8fafc;
      border: 2px solid #0f172a;
      box-shadow: 3px 3px 0px #0f172a;
      padding: 8px 6px;
      word-break: break-all;
      text-align: left;
    }}
    .meta-label {{
      display: block;
      font-weight: 700;
      color: #64748b;
      font-size: 9px;
      text-transform: uppercase;
      letter-spacing: 1px;
      margin-bottom: 2px;
    }}
    .meta-value {{
      font-weight: 700;
      color: #0f172a;
    }}
  </style>
</head>
<body>
  <div class="card">
    <div class="banner">[ PROXY EXCEPTION ]</div>
    <div class="status-code">{code}</div>
    <div class="status-title">{reason}</div>
    <div class="description">{message}</div>
    <div class="pixel-divider"></div>
    <div class="meta">
      <div class="meta-box"><span class="meta-label">TARGET HOST</span><span class="meta-value">{display_host}</span></div>
      <div class="meta-box"><span class="meta-label">CLIENT IP</span><span class="meta-value">{display_ip}</span></div>
    </div>
  </div>
</body>
</html>
"#
    )
}

pub fn build_error_response(
    status: StatusCode,
    message: &str,
    client_ip: &str,
    host: &str,
    headers: Option<&HeaderMap>,
) -> Response<BoxBody<Bytes, BoxError>> {
    let wants_json = headers
        .and_then(|h| h.get(hyper::header::ACCEPT))
        .and_then(|v| v.to_str().ok())
        .map(|ct| ct.contains("application/json"))
        .unwrap_or(false);

    if wants_json {
        let json_body = format!(
            "{{\"status\":{},\"error\":\"{}\",\"message\":\"{}\",\"host\":\"{}\"}}\n",
            status.as_u16(),
            status.canonical_reason().unwrap_or("Error"),
            message,
            if host.is_empty() { "-" } else { host }
        );
        let body = Full::new(Bytes::from(json_body))
            .map_err(|e| Box::new(e) as BoxError)
            .boxed();

        Response::builder()
            .status(status)
            .header(hyper::header::CONTENT_TYPE, "application/json")
            .body(body)
            .unwrap()
    } else {
        let html_body = render_html_error_page(status, message, client_ip, host);
        let body = Full::new(Bytes::from(html_body))
            .map_err(|e| Box::new(e) as BoxError)
            .boxed();

        Response::builder()
            .status(status)
            .header(hyper::header::CONTENT_TYPE, "text/html; charset=utf-8")
            .body(body)
            .unwrap()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_render_html_error_page() {
        let html = render_html_error_page(
            StatusCode::BAD_GATEWAY,
            "Upstream connection failed",
            "127.0.0.1",
            "example.local",
        );
        assert!(html.contains("502"));
        assert!(html.contains("Bad Gateway"));
        assert!(html.contains("Upstream connection failed"));
        assert!(html.contains("example.local"));
        assert!(html.contains("127.0.0.1"));
    }

    #[test]
    fn test_build_error_response_json() {
        let mut headers = HeaderMap::new();
        headers.insert(
            hyper::header::ACCEPT,
            hyper::header::HeaderValue::from_static("application/json"),
        );

        let resp = build_error_response(
            StatusCode::TOO_MANY_REQUESTS,
            "Rate limit exceeded",
            "1.2.3.4",
            "example.local",
            Some(&headers),
        );

        assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(
            resp.headers().get(hyper::header::CONTENT_TYPE).unwrap(),
            "application/json"
        );
    }
}
