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

### Configuration Example

```
security {
    proxy_protocol on;
    trusted_upstream 10.0.0.1;
    timeout 200;
}
```
