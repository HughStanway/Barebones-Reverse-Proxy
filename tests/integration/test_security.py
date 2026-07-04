from __future__ import annotations

import socket
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
