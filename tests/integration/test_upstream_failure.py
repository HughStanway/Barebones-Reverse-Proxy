import socket

from tests.integration.test_utils import get


def test_unreachable_upstream_returns_502(make_proxy):
    # GIVEN
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as s:
        s.bind(("127.0.0.1", 0))
        dead_port = s.getsockname()[1]

    proxy = make_proxy(upstream_port=dead_port)

    # WHEN
    status, body, _ = get(
        f"{proxy.url}/",
        headers={"Host": "example.local"},
    )
    
    # THEN
    assert status == 502, f"Expected 502, got {status}"
    assert b"502" in body
