from __future__ import annotations

import os
import socket
import ssl
import subprocess
import time
import pytest
from tests.integration.test_utils import get


def send_raw_bytes(port: int, bytes_to_send: bytes) -> bytes:
    s = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    s.settimeout(5.0)
    try:
        s.connect(("127.0.0.1", port))
        s.sendall(bytes_to_send)
        response = b""
        while True:
            chunk = s.recv(1024)
            if not chunk:
                break
            response += chunk
    except (socket.timeout, ConnectionResetError, OSError):
        pass
    finally:
        s.close()
    return response


def test_proxy_protocol_trusted_source_success(upstream, make_proxy):
    # GIVEN
    security_block = """
    security {
        proxy_protocol on;
        trusted_upstream 127.0.0.1;
        timeout 200;
    }
    """
    proxy = make_proxy(security_block=security_block)

    # WHEN
    request = (
        b"PROXY TCP4 192.168.0.99 127.0.0.1 54321 80\r\n"
        b"GET / HTTP/1.1\r\n"
        b"Host: example.local\r\n"
        b"Connection: close\r\n\r\n"
    )
    response = send_raw_bytes(proxy.port, request)

    # THEN
    assert b"200 OK" in response
    xff = upstream.last_request["headers"].get("x-forwarded-for", "")
    xri = upstream.last_request["headers"].get("x-real-ip", "")
    assert xff == "192.168.0.99"
    assert xri == "192.168.0.99"


def test_proxy_protocol_trusted_source_invalid_header_dropped(upstream, make_proxy):
    # GIVEN
    security_block = """
    security {
        proxy_protocol on;
        trusted_upstream 127.0.0.1;
        timeout 200;
    }
    """
    proxy = make_proxy(security_block=security_block)

    # WHEN
    # Send header with invalid IP address format
    request = (
        b"PROXY TCP4 999.999.999.999 127.0.0.1 54321 80\r\n"
        b"GET / HTTP/1.1\r\n"
        b"Host: example.local\r\n"
        b"Connection: close\r\n\r\n"
    )
    response = send_raw_bytes(proxy.port, request)

    # THEN
    assert len(response) == 0


def test_proxy_protocol_untrusted_source_spoof_rejected(upstream, make_proxy):
    # GIVEN
    # We configure a trusted IP of 10.0.0.1, so our connection from 127.0.0.1 is untrusted
    security_block = """
    security {
        proxy_protocol on;
        trusted_upstream 10.0.0.1;
        timeout 200;
    }
    """
    proxy = make_proxy(security_block=security_block)

    # WHEN
    # Try to spoof with a Proxy Protocol header anyway
    request = (
        b"PROXY TCP4 1.2.3.4 5.6.7.8 12345 80\r\n"
        b"GET / HTTP/1.1\r\n"
        b"Host: example.local\r\n"
        b"Connection: close\r\n\r\n"
    )
    response = send_raw_bytes(proxy.port, request)

    # THEN
    assert len(response) == 0


def test_proxy_protocol_untrusted_source_normal_request_passed(upstream, make_proxy):
    # GIVEN
    security_block = """
    security {
        proxy_protocol on;
        trusted_upstream 10.0.0.1;
        timeout 200;
    }
    """
    proxy = make_proxy(security_block=security_block)

    # WHEN
    # Send a standard HTTP GET request without any Proxy Protocol header
    request = (
        b"GET / HTTP/1.1\r\n"
        b"Host: example.local\r\n"
        b"Connection: close\r\n\r\n"
    )
    response = send_raw_bytes(proxy.port, request)

    # THEN
    assert b"200 OK" in response
    xff = upstream.last_request["headers"].get("x-forwarded-for", "")
    xri = upstream.last_request["headers"].get("x-real-ip", "")
    assert "127.0.0.1" in xff
    assert xri == "127.0.0.1"


def test_proxy_protocol_timeout(upstream, make_proxy):
    # GIVEN
    security_block = """
    security {
        proxy_protocol on;
        trusted_upstream 127.0.0.1;
        timeout 50;
    }
    """
    proxy = make_proxy(security_block=security_block)

    # WHEN
    s = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    s.settimeout(5.0)
    s.connect(("127.0.0.1", proxy.port))
    time.sleep(0.15)  # Wait for longer than the 50ms timeout

    # Check if the connection has been dropped
    try:
        # A read on a closed/dropped socket should return empty bytes (EOF)
        data = s.recv(1024)
        assert len(data) == 0
    finally:
        s.close()


def test_real_ip_header_extraction(upstream, make_proxy):
    # GIVEN
    security_block = """
    security {
        proxy_protocol on;
        trusted_upstream 127.0.0.1;
        timeout 200;
    }
    """
    proxy = make_proxy(security_block=security_block)

    # WHEN
    request = (
        b"PROXY TCP4 192.168.0.99 127.0.0.1 54321 80\r\n"
        b"GET / HTTP/1.1\r\n"
        b"Host: example.local\r\n"
        b"CF-Connecting-IP: 203.0.113.5\r\n"
        b"Connection: close\r\n\r\n"
    )
    response = send_raw_bytes(proxy.port, request)

    # THEN
    assert b"200 OK" in response
    xff = upstream.last_request["headers"].get("x-forwarded-for", "")
    xri = upstream.last_request["headers"].get("x-real-ip", "")
    assert xff == "203.0.113.5"
    assert xri == "203.0.113.5"


def test_tls_failure_blacklisting(upstream, make_proxy, tmp_path):
    cert_file = str(tmp_path / "cert.pem")
    key_file = str(tmp_path / "key.pem")
    subprocess.run(
        [
            "openssl",
            "req",
            "-x509",
            "-newkey",
            "rsa:2048",
            "-keyout",
            key_file,
            "-out",
            cert_file,
            "-days",
            "1",
            "-nodes",
            "-subj",
            "/CN=example.local",
        ],
        check=True,
        capture_output=True,
    )

    log_file_path = str(tmp_path / "proxy.log")
    extra_config = f"""
    logfile {log_file_path};
    cert example.local {{
        cert {cert_file};
        key {key_file};
    }}
    """
    proxy = make_proxy(extra_config=extra_config)

    # Send 6 invalid TLS handshakes (plain HTTP text to HTTPS listener)
    for _ in range(6):
        send_raw_bytes(proxy.port, b"GET / HTTP/1.1\r\nHost: example.local\r\n\r\n")

    time.sleep(0.1)
    with open(log_file_path, "r") as f:
        logs = f.read()

    assert "event=tls_handshake_failed" in logs
    assert "event=tls_failure_blacklist_triggered" in logs
    assert "ip=127.0.0.1" in logs


def test_tls_failure_blacklist_custom_config(upstream, make_proxy, tmp_path):
    cert_file = str(tmp_path / "cert.pem")
    key_file = str(tmp_path / "key.pem")
    subprocess.run(
        [
            "openssl",
            "req",
            "-x509",
            "-newkey",
            "rsa:2048",
            "-keyout",
            key_file,
            "-out",
            cert_file,
            "-days",
            "1",
            "-nodes",
            "-subj",
            "/CN=example.local",
        ],
        check=True,
        capture_output=True,
    )

    log_file_path = str(tmp_path / "proxy_custom.log")
    extra_config = f"""
    logfile {log_file_path};
    cert example.local {{
        cert {cert_file};
        key {key_file};
    }}
    security {{
        max_tls_failures 3;
        ban_duration 120;
    }}
    """
    proxy = make_proxy(extra_config=extra_config)

    # Send invalid TLS handshakes to trigger the blacklist
    for _ in range(6):
        send_raw_bytes(proxy.port, b"GET / HTTP/1.1\r\nHost: example.local\r\n\r\n")

    time.sleep(0.1)
    with open(log_file_path, "r") as f:
        logs = f.read()

    assert "event=tls_failure_blacklist_triggered" in logs
    assert "ban_duration_sec=120" in logs


def test_socket_level_drop_blacklisted_ip(upstream, make_proxy, tmp_path):
    cert_file = str(tmp_path / "cert.pem")
    key_file = str(tmp_path / "key.pem")
    subprocess.run(
        [
            "openssl",
            "req",
            "-x509",
            "-newkey",
            "rsa:2048",
            "-keyout",
            key_file,
            "-out",
            cert_file,
            "-days",
            "1",
            "-nodes",
            "-subj",
            "/CN=example.local",
        ],
        check=True,
        capture_output=True,
    )

    log_file_path = str(tmp_path / "proxy_drop.log")
    extra_config = f"""
    logfile {log_file_path};
    cert example.local {{
        cert {cert_file};
        key {key_file};
    }}
    security {{
        max_tls_failures 2;
        ban_duration 300;
    }}
    """
    proxy = make_proxy(extra_config=extra_config)

    # 1. Trigger blacklist with invalid TLS handshakes
    for _ in range(6):
        send_raw_bytes(proxy.port, b"GET / HTTP/1.1\r\nHost: example.local\r\n\r\n")

    time.sleep(0.1)

    # 2. Open a new raw TCP connection from the now-blacklisted IP (127.0.0.1)
    s = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    s.settimeout(2.0)
    s.connect(("127.0.0.1", proxy.port))
    # Read should return empty bytes immediately because proxy dropped the socket!
    data = s.recv(1024)
    s.close()

    assert len(data) == 0

    with open(log_file_path, "r") as f:
        logs = f.read()

    assert "event=tls_failure_blacklist_triggered" in logs
    assert "event=connection_dropped_blacklisted" in logs
    assert "peer=127.0.0.1" in logs


def test_request_rate_limiting_429(upstream, make_proxy):
    security_block = """
    security {
        rate_limit_rpm 3;
    }
    """
    proxy = make_proxy(security_block=security_block)

    # First 3 requests within threshold should succeed
    for _ in range(3):
        status, body, headers = get(f"{proxy.url}/", headers={"Host": "example.local"})
        assert status == 200

    # 4th request exceeds rate limit threshold -> 429 Too Many Requests
    status, body, headers = get(f"{proxy.url}/", headers={"Host": "example.local"})
    assert status == 429
    assert b"429 Too Many Requests" in body
    assert headers.get("retry-after") == "60"


def test_sni_verification_rejects_unowned_domain_and_bare_ip(upstream, make_proxy, tmp_path):
    cert_file = str(tmp_path / "cert.pem")
    key_file = str(tmp_path / "key.pem")
    subprocess.run(
        [
            "openssl",
            "req",
            "-x509",
            "-newkey",
            "rsa:2048",
            "-keyout",
            key_file,
            "-out",
            cert_file,
            "-days",
            "1",
            "-nodes",
            "-subj",
            "/CN=example.local",
        ],
        check=True,
        capture_output=True,
    )

    log_file_path = str(tmp_path / "proxy_sni.log")
    extra_config = f"""
    logfile {log_file_path};
    cert example.local {{
        cert {cert_file};
        key {key_file};
    }}
    """
    proxy = make_proxy(extra_config=extra_config)

    ctx = ssl.create_default_context()
    ctx.check_hostname = False
    ctx.verify_mode = ssl.CERT_NONE

    # 1. Connect with unowned SNI hostname -> TLS handshake fails
    with pytest.raises((ssl.SSLError, OSError)):
        with socket.create_connection(("127.0.0.1", proxy.port), timeout=2.0) as raw_sock:
            with ctx.wrap_socket(raw_sock, server_hostname="unowned.domain.com") as tls_sock:
                tls_sock.sendall(b"GET / HTTP/1.1\r\nHost: unowned.domain.com\r\n\r\n")

    # 2. Connect to bare IP (no SNI hostname) -> TLS handshake fails
    with pytest.raises((ssl.SSLError, OSError)):
        with socket.create_connection(("127.0.0.1", proxy.port), timeout=2.0) as raw_sock:
            with ctx.wrap_socket(raw_sock) as tls_sock:
                tls_sock.sendall(b"GET / HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n")

    time.sleep(0.1)
    with open(log_file_path, "r") as f:
        logs = f.read()

    assert "event=tls_handshake_failed" in logs
    assert "no server certificate chain resolved" in logs





