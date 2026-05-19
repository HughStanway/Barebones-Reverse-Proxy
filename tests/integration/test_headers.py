from tests.integration.test_utils import get


def test_x_forwarded_for_is_set(upstream, make_proxy):
    # GIVEN
    proxy = make_proxy()

    # WHEN
    get(f"{proxy.url}/", headers={"Host": "example.local"})

    # THEN
    xff = upstream.last_request["headers"].get("x-forwarded-for", "")
    assert xff != "", "X-Forwarded-For header was not set"
    assert "127.0.0.1" in xff


def test_x_forwarded_for_is_appended_when_present(upstream, make_proxy):
    # GIVEN
    proxy = make_proxy()

    # WHEN
    get(
        f"{proxy.url}/",
        headers={"Host": "example.local", "X-Forwarded-For": "10.0.0.1"},
    )

    # THEN
    xff = upstream.last_request["headers"].get("x-forwarded-for", "")
    assert "10.0.0.1" in xff, "original X-Forwarded-For entry was dropped"
    assert "127.0.0.1" in xff, "client IP was not appended"


def test_x_forwarded_host_matches_original_host(upstream, make_proxy):
    # GIVEN
    proxy = make_proxy(request_host="app.local")

    # WHEN
    get(f"{proxy.url}/", headers={"Host": "app.local"})

    # THEN
    xfh = upstream.last_request["headers"].get("x-forwarded-host", "")
    assert xfh == "app.local", f"Expected 'app.local', got '{xfh}'"


def test_x_forwarded_proto_is_https(upstream, make_proxy):
    # GIVEN
    proxy = make_proxy()

    # WHEN
    get(f"{proxy.url}/", headers={"Host": "example.local"})

    # THEN
    xfp = upstream.last_request["headers"].get("x-forwarded-proto", "")
    assert xfp == "https", f"Expected 'https', got '{xfp}'"


def test_x_real_ip_is_set(upstream, make_proxy):
    # GIVEN
    proxy = make_proxy()

    # WHEN
    get(f"{proxy.url}/", headers={"Host": "example.local"})

    # THEN
    xri = upstream.last_request["headers"].get("x-real-ip", "")
    assert xri != "", "X-Real-IP header was not set"
    assert "127.0.0.1" in xri


def test_original_host_header_is_preserved(upstream, make_proxy):
    # GIVEN
    proxy = make_proxy(request_host="app.local")

    # WHEN
    get(f"{proxy.url}/", headers={"Host": "app.local"})

    # THEN
    host = upstream.last_request["headers"].get("host", "")
    assert host == "app.local", (
        f"Host header was rewritten to '{host}' instead of 'app.local'"
    )
