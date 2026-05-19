import json
from tests.integration.test_utils import get

def test_proxy_forwards_upstream_status_code(upstream, make_proxy):
    # GIVEN
    proxy = make_proxy()

    # WHEN
    status, _, _ = get(
        f"{proxy.url}/",
        headers={
            "Host": "example.local",
            "X-Mock-Status": "400"
        }
    )

    # THEN
    assert status == 400


def test_proxy_forwards_upstream_headers(upstream, make_proxy):
    # GIVEN
    proxy = make_proxy()
    mock_headers = {
        "X-Custom-Response": "Hello World",
        "Set-Cookie": "session_id=12345"
    }

    # WHEN
    status, _, headers = get(
        f"{proxy.url}/",
        headers={
            "Host": "example.local",
            "X-Mock-Headers": json.dumps(mock_headers)
        }
    )

    # THEN
    assert status == 200
    assert headers.get("x-custom-response") == "Hello World"
    assert headers.get("set-cookie") == "session_id=12345"
