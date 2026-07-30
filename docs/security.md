# Security and Hardening

This document describes the security and network hardening features implemented in the Barebones Reverse Proxy, focusing on the implementation of **Proxy Protocol v1**, **Source IP Enforcement**, and **Parser guards**.

---

## 1. Proxy Protocol v1 Support

To preserve the client's real source IP address when the reverse proxy sits behind another load balancer (such as HAProxy, AWS ALB, or a GCP Cloud Load Balancer), the proxy supports parsing **Proxy Protocol v1** headers.

### Signature Verification
Before forwarding any payload to the TLS handshake or HTTP stack, the proxy peeks or reads the socket buffer. If Proxy Protocol is enabled and the connection is from a trusted source, it expects a header matching the exact ASCII string format:
```
PROXY TCP4 [ClientIP] [ProxyIP] [ClientPort] [ProxyPort]\r\n
```

### Connection Context Propagation
Upon successful parsing:
- The extracted `ClientIP` is parsed into a Rust `IpAddr` object.
- The `ClientPort` is parsed into a `u16`.
- The connection's peer address is resolved to this client address.
- Downstream HTTP routing and proxy headers (such as `X-Forwarded-For` and `X-Real-IP`) will dynamically use this parsed IP, ensuring that the backend application receives correct, un-spoofed client metadata.

---

## 2. Source IP Enforcement (Spoofing Prevention)

To prevent attackers on public or untrusted networks from injecting fake Proxy Protocol headers and spoofing their client IP, the proxy establishes a strict **Trust Boundary**.

- **Trusted Sources**: Only connections originating precisely from the configured `trusted_upstream` IP address will trigger Proxy Protocol header parsing.
- **Untrusted Sources**:
  - The proxy peeks at the first 5 bytes of the socket buffer.
  - If the connection starts with the string `PROXY`, it is immediately classified as a spoofing attempt and **rejected/dropped**.
  - If it does not start with `PROXY` (e.g., standard HTTP request or TLS ClientHello), it is accepted and handled normally, ensuring standard clients can still connect directly.

---

## 3. Parser Guards (DOS Protection)

Malicious clients or misconfigured bots might open a connection and send nothing (slowloris attack) or send infinite junk bytes trying to crash or exhaust memory in the header parser. The proxy implements two crucial guards to mitigate this:

1. **Byte-Cap Guard**:
   - The proxy caps proxy protocol line reads at exactly **107 bytes** (the theoretical maximum length for a Proxy Protocol v1 line).
   - If the header is not fully parsed (missing `\r\n` terminator) by the time 107 bytes are read, the connection is instantly closed.

2. **Parser Timeout Guard**:
   - The entire handshake inspection process (parsing the header for trusted sources, or peeking for spoofed headers for untrusted sources) is wrapped in a strict timeout (e.g., **200 milliseconds**).
   - If the checks do not complete within the time limit, the socket is dropped.

---

## 4. Configuration

All security hardening features are grouped under the `security` block in the configuration file.

### Settings Reference

| Directive | Description | Type / Values | Default |
| :--- | :--- | :--- | :--- |
| `proxy_protocol` | Toggles Proxy Protocol v1 parsing and spoofing checks. | `on`/`off` or `true`/`false` | `off` |
| `trusted_upstream` | The exact IP address allowed to send Proxy Protocol headers (required if `proxy_protocol` is `on`). | IP Address (IPv4/IPv6) | None |
| `timeout` | Time limit in milliseconds for parsing headers and peeking spoof attempts. | Number (in ms) | `200` |
| `max_tls_failures` | Maximum allowed TLS handshake failures per IP in a rolling 60s window before blacklisting. | Number | `5` |
| `ban_duration` | Time duration in seconds that an IP remains blacklisted after exceeding TLS failure limits. | Number (in seconds) | `3600` (1 hour) |
| `rate_limit_rpm` | Maximum allowed HTTP requests per minute per client IP before returning 429 Too Many Requests. | Number (req/min) | `60` |

### Configuration Example

```protobuf
security {
    proxy_protocol on;
    trusted_upstream 10.0.0.1;
    timeout 200;
    max_tls_failures 5;
    ban_duration 3600;
    rate_limit_rpm 60;
}
```

---

## 5. Active Defensive Rate-Limiting & IP Banning

The proxy features an active, thread-safe in-memory security manager (`SecurityManager`) designed to protect home servers and backend upstreams against automated scanners, brute-force bots, and aggressive web crawlers.

### 1. In-Memory TLS Failure Blacklist (Goal 1)
- **Rolling Window Tracking**: Tracks client TLS handshake failure timestamps within a 60-second window.
- **Automated Blacklisting**: If a specific client IP address triggers more than `max_tls_failures` (default: 5) within 60 seconds (e.g. plain HTTP scans on HTTPS ports or unsupported ciphers), the IP is automatically flagged as malicious and blacklisted.
- **TTL Ban Expiration**: Blacklisted IPs remain banned for `ban_duration` seconds (default: 3600s = 1 hour).
- **Memory Garbage Collection**: Ban expiration timestamps (`banned_until`) and expired failure vectors are automatically evicted on access, keeping process memory bounded over time.
- **SIGHUP Persistence**: The `SecurityManager` state persists across zero-downtime configuration reloads.

### 2. Socket-Level Drop / Zero-CPU Filtering (Goal 2)
- **Pre-TLS Socket Inspection**: Checks if an incoming TCP connection originates from a blacklisted IP immediately after socket accept (and after Proxy-Protocol client IP resolution).
- **Zero-CPU Filtering**: If the IP is blacklisted, the proxy closes the raw TCP socket immediately (`return;`) before initiating the cryptographic TLS handshake (`acceptor.accept()`).
- **Resource Denial**: Denies malicious bots any opportunity to consume server CPU, RAM, or cryptographic worker threads.
- **Structured Logging**: Emits `event=connection_dropped_blacklisted` with client IP details.

### 3. Strict Path & Host Throttling / HTTP 429 (Goal 3)
- **Rolling Request Limit**: Tracks HTTP request timestamps per client IP in a 60-second rolling window.
- **Throttling Threshold**: If a client IP exceeds `rate_limit_rpm` (default: 300 requests/min, set `0` to disable), the proxy immediately rejects the request with an **`HTTP 429 Too Many Requests`** status code.
- **Standard Retry Headers**: Includes a `Retry-After: 60` HTTP header in 429 responses to inform clients when to retry.
- **Structured Logging**: Emits `event=rate_limit_exceeded` and records standard request logs with `status=429`.

---

## 6. Application-Layer Behavior & The "Silent Treatment"

### 1. Catch-All HTTP 444 "No Response" Handler
- **Unmatched Route Starvation**: When automated scanners request unrouted paths or unrouted host headers, the proxy avoids writing error stack traces or returning detailed HTML error pages.
- **HTTP 444 Connection Termination**: Returns `HTTP/1.1 444 Connection Closed Without Response` with a zero-byte body and `Connection: close` header.
- **Data Starvation**: Starves scanning scripts of OS and server framework metadata, causing automated tools to hang up or time out.

#### When It Triggers (Examples):
1. **Scanner Probes for Sensitive Files**: A vulnerability scanner sends `GET /.env`, `GET /robots.txt`, or `GET /wp-login.php` against your configured domain.
2. **Unrouted Host Header**: A bot sends an HTTP request with an unconfigured `Host` header (e.g. `Host: unknown-scanner.com` or `Host: 192.168.1.50`).
3. **Unmapped Endpoint**: Any HTTP request where the `Host` + `Path` pair does not match an explicit `route` directive in `proxy.conf`.

#### Response Behavior:
```http
HTTP/1.1 444 Connection Closed Without Response
connection: close
content-length: 0
```

#### Structured Log Output:
```text
[2026-07-26 17:31:31] [INFO] [worker-thread-0] event=no_matching_route config_generation=1 peer=198.51.100.42:54321 host=grafana.bigiron.dev path=/.env
[2026-07-26 17:31:31] [INFO] [worker-thread-0] event=request peer=198.51.100.42:54321 client_ip=198.51.100.42 method=GET host=grafana.bigiron.dev path=/.env version=HTTP/1.1 status=444 duration_ms=0.150 upstream=- user_agent=zgrab/0.x referer=-
```

---

### 2. Dynamic SNI Verification
- **Exact SNI Domain Matching**: During the TLS ClientHello handshake, `WildcardCertResolver` checks the Server Name Indication (SNI) extension against exact and wildcard domain matches defined in `cert` blocks.
- **Pre-Routing Rejection**: If a client connects using a bare IP address (missing SNI) or an unowned domain string, the proxy aborts the TLS handshake immediately with `no server certificate chain resolved` before any HTTP routing logic is executed.
- **Integration with Blacklisting**: Failed SNI handshakes increment the client's TLS failure counter, causing persistent scanners to be blacklisted and dropped at the raw TCP socket level.

#### When It Triggers (Examples):
1. **Bare IP HTTPS Scan**: A bot connects directly to `https://<YOUR_PUBLIC_IP>/` without sending an SNI hostname extension in the ClientHello message.
2. **Unowned Domain Scan**: An attacker attempts a TLS handshake specifying an unconfigured domain (e.g. `server.victim.com` or `scanner.xyz`) that has no matching `cert` block.

#### Response Behavior:
- The TLS handshake is aborted during ClientHello certificate resolution.
- No TLS session or server certificate chain is provided.
- Zero HTTP routing or application code is evaluated.

#### Structured Log Output:
```text
[2026-07-26 17:31:31] [ERROR] [worker-thread-0] event=tls_handshake_failed peer=203.0.113.88:41234 error=no server certificate chain resolved
```
*(If repeated > `max_tls_failures` times within 60s, automatically triggers blacklisting & zero-CPU socket drop):*
```text
[2026-07-26 17:31:32] [ERROR] [worker-thread-0] event=tls_failure_blacklist_triggered ip=203.0.113.88 failures=6 ban_duration_sec=3600
[2026-07-26 17:31:33] [INFO] [worker-thread-0] event=connection_dropped_blacklisted peer=203.0.113.88:41235
```

---

## 7. Native Response Header Middleware

To protect client web browsers accessing tools through the reverse proxy, the proxy programmatically appends a strict suite of security headers to every outgoing HTTP response payload passed back up the tunnel:

### Headers Suite Reference

| Header | Value | Purpose |
| :--- | :--- | :--- |
| `Strict-Transport-Security` | `max-age=63072000; includeSubDomains; preload` | Enforces permanent HTTPS across all subdomains for 2 years and qualifies for browser HSTS preload lists. |
| `X-Content-Type-Options` | `nosniff` | Prevents browsers from MIME-sniffing responses away from the declared `Content-Type`, stopping disguised file execution. |
| `X-Frame-Options` | `DENY` | Neutralizes clickjacking and iframe injection attacks by preventing any site from embedding your tools in `<frame>`, `<iframe>`, `<embed>`, or `<object>` tags. |
| `X-XSS-Protection` | `0` | Disables legacy, buggy browser XSS filters in favor of modern security. |

#### When It Triggers (Examples):
1. **Normal Proxied Responses**: Appended to all successful `200 OK` upstream responses returned from backend web apps (e.g., Grafana, Speedtest, home services).
2. **Proxy Error & Throttling Responses**: Appended to all proxy-generated responses including `429 Too Many Requests`, `502 Bad Gateway`, and `444 Connection Closed`.

#### Outgoing Response Headers Example:
```http
HTTP/1.1 200 OK
Strict-Transport-Security: max-age=63072000; includeSubDomains; preload
X-Content-Type-Options: nosniff
X-Frame-Options: DENY
X-XSS-Protection: 0
Content-Type: text/html
Content-Length: 1024
```

---

## 8. Automatic Client IP Resolution (CDNs & Proxies)

When `proxy_protocol` is active and the incoming connection is verified as originating from the `trusted_upstream` (e.g., your GCP Load Balancer), the reverse proxy automatically extracts the true client IP from standard proxy/CDN HTTP headers in the following priority order:
1. `CF-Connecting-IP` (Cloudflare)
2. `True-Client-IP` (Enterprise CDNs)
3. `X-Forwarded-For` (Standard Load Balancers)

If the request is made directly bypassing the trusted upstream boundary, these headers are ignored, protecting the proxy against IP spoofing.
