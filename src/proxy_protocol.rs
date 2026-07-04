use std::io;
use std::net::{IpAddr, SocketAddr};
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, ReadBuf};
use tokio::net::TcpStream;
use crate::config::SecurityConfig;

pub struct ProxyStream {
    inner: TcpStream,
    buffer: Option<std::io::Cursor<Vec<u8>>>,
}

impl ProxyStream {
    pub fn new(inner: TcpStream, buffer: Option<Vec<u8>>) -> Self {
        let buffer = buffer.map(std::io::Cursor::new);
        Self { inner, buffer }
    }
}

impl AsyncRead for ProxyStream {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        if let Some(ref mut cursor) = this.buffer {
            let before = buf.remaining();
            if before > 0 {
                let mut temp_buf = vec![0; before];
                match std::io::Read::read(cursor, &mut temp_buf) {
                    Ok(n) => {
                        if n > 0 {
                            buf.put_slice(&temp_buf[..n]);
                            if cursor.position() >= cursor.get_ref().len() as u64 {
                                this.buffer = None;
                            }
                            return Poll::Ready(Ok(()));
                        }
                    }
                    Err(e) => return Poll::Ready(Err(e)),
                }
            }
            this.buffer = None;
        }
        Pin::new(&mut this.inner).poll_read(cx, buf)
    }
}

impl AsyncWrite for ProxyStream {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<Result<usize, io::Error>> {
        Pin::new(&mut self.get_mut().inner).poll_write(cx, buf)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), io::Error>> {
        Pin::new(&mut self.get_mut().inner).poll_flush(cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), io::Error>> {
        Pin::new(&mut self.get_mut().inner).poll_shutdown(cx)
    }
}

/// Handle proxy protocol logic for an incoming stream.
///
/// If proxy protocol is disabled: returns the stream and original peer_addr unchanged.
/// If proxy protocol is enabled:
/// - If peer_addr matches the trusted_upstream: attempts to parse the Proxy Protocol v1 header.
///   If parsing fails or times out, returns an error.
/// - If peer_addr is untrusted: checks if the stream starts with a fake Proxy Protocol header.
///   If so, rejects the connection. Otherwise, proceeds with the connection, prepending any
///   peeked bytes.
pub async fn handle_proxy_protocol(
    mut stream: TcpStream,
    peer_addr: SocketAddr,
    security_config: &SecurityConfig,
) -> Result<(ProxyStream, SocketAddr), String> {
    if !security_config.proxy_protocol {
        return Ok((ProxyStream::new(stream, None), peer_addr));
    }

    let is_trusted = peer_addr.ip() == security_config.trusted_upstream;
    let timeout_duration = Duration::from_millis(security_config.timeout_ms);

    if is_trusted {
        let parse_future = async {
            let mut header_buf = Vec::new();
            let mut temp = [0u8; 1];
            while header_buf.len() < 107 {
                let n = stream.read(&mut temp).await.map_err(|e| e.to_string())?;
                if n == 0 {
                    return Err("EOF before proxy header finished".to_string());
                }
                header_buf.push(temp[0]);
                if header_buf.ends_with(b"\r\n") {
                    break;
                }
            }
            if !header_buf.ends_with(b"\r\n") {
                return Err("Proxy header exceeded max length or missing CRLF".to_string());
            }
            let header_str = std::str::from_utf8(&header_buf)
                .map_err(|_| "Proxy header is not valid UTF-8".to_string())?;

            let (client_ip, client_port) = parse_proxy_protocol_v1(header_str)
                .ok_or_else(|| "Failed to parse Proxy Protocol header".to_string())?;

            let client_addr = SocketAddr::new(client_ip, client_port);
            crate::log_info!("proxy_protocol_parsed", "peer" => peer_addr, "client" => client_addr);
            Ok::<_, String>((ProxyStream::new(stream, None), client_addr))
        };

        match tokio::time::timeout(timeout_duration, parse_future).await {
            Ok(Ok(res)) => Ok(res),
            Ok(Err(err)) => {
                crate::log_error!("proxy_protocol_parse_failed", "peer" => peer_addr, "error" => err);
                Err(err)
            }
            Err(_) => {
                crate::log_error!("proxy_protocol_timeout", "peer" => peer_addr);
                Err("Timeout reading Proxy Protocol header".to_string())
            }
        }
    } else {
        let check_future = async {
            let mut header_buf = Vec::new();
            let mut temp = [0u8; 1];
            let prefix = b"PROXY";

            while header_buf.len() < prefix.len() {
                let n = stream.read(&mut temp).await.map_err(|e| e.to_string())?;
                if n == 0 {
                    break;
                }
                header_buf.push(temp[0]);
                if header_buf[header_buf.len() - 1] != prefix[header_buf.len() - 1] {
                    break;
                }
            }

            if header_buf.starts_with(prefix) {
                crate::log_error!("proxy_protocol_spoof_rejected", "peer" => peer_addr);
                return Err("Spoofing attempt: Proxy Protocol header injected from untrusted source".to_string());
            }

            Ok::<_, String>(ProxyStream::new(stream, Some(header_buf)))
        };

        match tokio::time::timeout(timeout_duration, check_future).await {
            Ok(Ok(ps)) => Ok((ps, peer_addr)),
            Ok(Err(err)) => Err(err),
            Err(_) => {
                crate::log_error!("proxy_protocol_timeout", "peer" => peer_addr);
                Err("Timeout checking for fake Proxy Protocol header".to_string())
            }
        }
    }
}

fn parse_proxy_protocol_v1(header: &str) -> Option<(IpAddr, u16)> {
    if !header.starts_with("PROXY TCP4 ") {
        return None;
    }
    if !header.ends_with("\r\n") {
        return None;
    }
    let payload = &header[11..header.len() - 2];
    let parts: Vec<&str> = payload.split(' ').collect();
    if parts.len() != 4 {
        return None;
    }
    let client_ip = parts[0].parse::<std::net::Ipv4Addr>().ok()?;
    let _proxy_ip = parts[1].parse::<std::net::Ipv4Addr>().ok()?;
    let client_port = parts[2].parse::<u16>().ok()?;
    let _proxy_port = parts[3].parse::<u16>().ok()?;

    Some((IpAddr::V4(client_ip), client_port))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_parse_proxy_protocol_v1_valid() {
        let header = "PROXY TCP4 1.2.3.4 5.6.7.8 12345 80\r\n";
        let parsed = parse_proxy_protocol_v1(header).unwrap();
        assert_eq!(parsed.0, "1.2.3.4".parse::<IpAddr>().unwrap());
        assert_eq!(parsed.1, 12345);
    }

    #[tokio::test]
    async fn test_parse_proxy_protocol_v1_invalid() {
        assert!(parse_proxy_protocol_v1("PROXY TCP4 1.2.3.4\r\n").is_none());
        assert!(parse_proxy_protocol_v1("PROXY TCP4 1.2.3.4 5.6.7.8 12345\r\n").is_none());
        assert!(parse_proxy_protocol_v1("PROXY TCP6 1.2.3.4 5.6.7.8 12345 80\r\n").is_none());
        assert!(parse_proxy_protocol_v1("PROXY TCP4 1.2.3.4 5.6.7.8 12345 80").is_none());
    }
}
