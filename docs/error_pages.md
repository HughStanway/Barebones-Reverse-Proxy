# Built-in Error Pages & Upstream Error Interception

This document describes the embedded error handling engine in Barebones Reverse Proxy, including the light-mode 8-bit retro theme, content negotiation (HTML vs JSON), and the `intercept_errors` directive.

---

## 1. Overview

By default, the proxy generates clean, consistent error responses whenever an exception occurs at the proxy layer (e.g. `404 Not Found` for unrouted domains, `502 Bad Gateway` when an upstream backend is down, `429 Too Many Requests` when rate limited).

With **Upstream Error Interception** enabled, the proxy can also intercept HTTP 4xx and 5xx errors returned by upstream backend applications and replace them with unified proxy error pages.

---

## 2. Design Aesthetic

The embedded error pages feature a light-themed **8-bit retro computer game / arcade aesthetic**:

* **CRT Scanline Background**: Light slate background (`#f1f5f9`) featuring a subtle horizontal scanline gradient overlay.
* **Blocky Pixel Card**: Pure white card container (`#ffffff`) bounded by a thick 4px solid dark border (`border: 4px solid #0f172a`) and a hard 8-bit offset drop shadow (`box-shadow: 8px 8px 0px #0f172a`).
* **Arcade Monospace Typography**:
  * Top banner badge: `[ PROXY EXCEPTION ]`
  * Chunky Arcade Red status digits (`#dc2626`) with solid 3px drop shadow
  * Monospace font stack (`"Courier New", Courier, Monaco, monospace`)
* **Metadata Boxes**: Dashed pixel separator line with hard-shadow metadata boxes displaying target host and client IP.

---

## 3. Content Negotiation (HTML vs. JSON)

The error renderer inspects the incoming client's `Accept` HTTP header:

* **Browser Requests** (`Accept: text/html` or standard web requests):
  Renders the full HTML5 retro 8-bit error page.
* **API / Automated Clients** (`Accept: application/json`):
  Returns a structured JSON payload:
  ```json
  {
    "status": 404,
    "error": "Not Found",
    "message": "Upstream server returned error: Not Found",
    "host": "api.example.com"
  }
  ```

---

## 4. `intercept_errors` Configuration Directive

The `intercept_errors` setting controls whether the proxy intercepts HTTP $4\text{xx}$ and $5\text{xx}$ responses returned by upstream backends.

### Directive Scope & Inheritance

`intercept_errors` can be declared globally at the top level of `proxy.conf` or overridden per-route inside `route { ... }` blocks:

```nginx
// Global default: Intercept upstream 4xx and 5xx errors
intercept_errors on;

// Route inheriting global setting (intercept_errors on)
route https://dashboard.bigiron.dev/ {
    upstream http://localhost:3000/;
}

// Route overriding global setting to pass raw backend errors
route https://api.bigiron.dev/ {
    upstream http://localhost:4000/;
    intercept_errors off;
}
```

### Supported Values

| Setting | Values | Behavior |
| :--- | :--- | :--- |
| `intercept_errors on;` | `on`, `yes`, `true` | Intercepts status codes $\ge 400$ from upstream backends and replaces response body with proxy error page. |
| `intercept_errors off;` | `off`, `no`, `false` | Passes upstream backend response body and status code directly through to the client. |

---

## 5. Cloudflare & CDN Interoperability Note

When running behind Cloudflare CDN in **Proxied (Orange Cloud)** mode:

* **HTTP 525 (SSL Handshake Failed)**: Occurs at Cloudflare's TLS layer before reaching your server. To resolve, ensure Cloudflare SSL/TLS mode is set to **Full (Strict)** and valid TLS certificates are installed on origin routes.
* **Disabling Cloudflare Error Overrides**:
  * Set Cloudflare DNS record to **DNS Only (Grey Cloud)** for direct proxy connection.
  * Or create a Cloudflare Configuration Rule with **Custom Error Pages $\rightarrow$ Off** to pass origin error pages directly to visitors.
