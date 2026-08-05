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

    let accent_color = match code {
        400..=499 if code == 429 || code == 413 => "#fbbf24", // Amber
        400..=499 => "#38bdf8",                               // Cyan
        _ => "#f43f5e",                                       // Rose
    };

    format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8">
  <meta name="viewport" content="width=device-width, initial-scale=1.0">
  <title>{code} {reason}</title>
  <style>
    :root {{
      --accent: {accent_color};
    }}
    * {{ box-sizing: border-box; margin: 0; padding: 0; }}
    body {{
      background-color: #0b0f19;
      color: #f8fafc;
      font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, Helvetica, Arial, sans-serif;
      display: flex;
      align-items: center;
      justify-content: center;
      min-height: 100vh;
      padding: 20px;
    }}
    .card {{
      background: rgba(17, 24, 39, 0.75);
      backdrop-filter: blur(16px);
      -webkit-backdrop-filter: blur(16px);
      border: 1px solid rgba(255, 255, 255, 0.1);
      border-radius: 20px;
      padding: 48px 40px;
      max-width: 520px;
      width: 100%;
      text-align: center;
      box-shadow: 0 25px 50px -12px rgba(0, 0, 0, 0.6);
    }}
    .status-code {{
      font-size: 80px;
      font-weight: 800;
      line-height: 1;
      color: var(--accent);
      letter-spacing: -2px;
      margin-bottom: 12px;
      text-shadow: 0 0 30px rgba(56, 189, 248, 0.2);
    }}
    .status-title {{
      font-size: 22px;
      font-weight: 600;
      color: #f1f5f9;
      margin-bottom: 16px;
    }}
    .description {{
      font-size: 15px;
      color: #94a3b8;
      line-height: 1.6;
      margin-bottom: 32px;
    }}
    .divider {{
      height: 1px;
      background: linear-gradient(90deg, transparent, rgba(255, 255, 255, 0.1), transparent);
      margin-bottom: 24px;
    }}
    .meta {{
      display: flex;
      justify-content: space-between;
      gap: 16px;
      font-size: 12px;
      color: #64748b;
    }}
    .meta-item {{
      background: rgba(255, 255, 255, 0.03);
      padding: 8px 14px;
      border-radius: 8px;
      border: 1px solid rgba(255, 255, 255, 0.05);
      flex: 1;
      word-break: break-all;
    }}
    .meta-label {{
      display: block;
      font-size: 10px;
      text-transform: uppercase;
      letter-spacing: 0.5px;
      color: #475569;
      margin-bottom: 2px;
    }}
  </style>
</head>
<body>
  <div class="card">
    <div class="status-code">{code}</div>
    <div class="status-title">{reason}</div>
    <div class="description">{message}</div>
    <div class="divider"></div>
    <div class="meta">
      <div class="meta-item">
        <span class="meta-label">Host</span>
        {display_host}
      </div>
      <div class="meta-item">
        <span class="meta-label">Client IP</span>
        {display_ip}
      </div>
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
