from tests.integration.test_utils import get


def test_known_route_returns_200(upstream, make_proxy):
    # GIVEN
    proxy = make_proxy(request_host="app.local", request_path="/")

    # WHEN
    status, _, _ = get(
        f"{proxy.url}/", headers={"Host": "app.local"}
    )

    # THEN
    assert status == 200


def test_unknown_host_returns_444(upstream, make_proxy):
    # GIVEN
    proxy = make_proxy(request_host="app.local", request_path="/")

    # WHEN
    status, body, _ = get(
        f"{proxy.url}/", headers={"Host": "unknown.host"}
    )

    # THEN
    assert status == 444
    assert len(body) == 0


def test_unknown_path_returns_444(upstream, make_proxy):
    # GIVEN
    proxy = make_proxy(request_host="app.local", request_path="/api/")

    # WHEN
    status, body, _ = get(
        f"{proxy.url}/other", headers={"Host": "app.local"}
    )

    # THEN
    assert status == 444
    assert len(body) == 0


def test_scanner_paths_return_444(upstream, make_proxy):
    # GIVEN
    proxy = make_proxy(request_host="app.local", request_path="/app/")

    # WHEN requesting scanner paths
    for path in ["/robots.txt", "/.env", "/wp-login.php", "/admin"]:
        status, body, _ = get(
            f"{proxy.url}{path}", headers={"Host": "app.local"}
        )

        # THEN
        assert status == 444
        assert len(body) == 0


def test_path_is_rewritten_at_upstream(upstream, make_proxy):
    # GIVEN
    proxy = make_proxy(
        request_host="app.local",
        request_path="/api/",
        forward_path="/v1/",
    )

    # WHEN
    get(f"{proxy.url}/api/users", headers={"Host": "app.local"})

    # THEN
    assert upstream.last_request.get("path") == "/v1/users"
