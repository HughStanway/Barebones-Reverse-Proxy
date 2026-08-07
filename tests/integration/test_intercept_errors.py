from tests.integration.test_utils import get


def test_intercept_errors_enabled_returns_proxy_error_page(upstream, make_proxy):
    # GIVEN proxy with global intercept_errors on
    proxy = make_proxy(extra_config="intercept_errors on;")

    # WHEN upstream returns 404
    status, body, _ = get(
        f"{proxy.url}/",
        headers={
            "Host": "example.local",
            "X-Mock-Status": "404",
        },
    )

    # THEN response status is 404 and body contains retro proxy error template
    assert status == 404
    assert b"[ PROXY EXCEPTION ]" in body
    assert b"404" in body
    assert b"Not Found" in body


def test_intercept_errors_disabled_returns_raw_upstream_body(upstream, make_proxy):
    # GIVEN proxy with default intercept_errors (off)
    proxy = make_proxy()

    # WHEN upstream returns 404
    status, body, _ = get(
        f"{proxy.url}/",
        headers={
            "Host": "example.local",
            "X-Mock-Status": "404",
        },
    )

    # THEN response status is 404 and body is raw upstream JSON
    assert status == 404
    assert b"[ PROXY EXCEPTION ]" not in body
    assert b'"method":"GET"' in body
