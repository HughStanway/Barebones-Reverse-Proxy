from tests.integration.test_utils import method_request, post


def test_post_with_body_is_proxied(upstream, make_proxy):
    # GIVEN
    proxy = make_proxy()
    payload = b'{"hello": "world"}'

    # WHEN
    status, _ = post(
        f"{proxy.url}/",
        payload,
        headers={"Host": "example.local", "Content-Type": "application/json"},
    )

    # THEN
    assert status == 200
    assert upstream.last_request["method"] == "POST"
    assert upstream.last_request["body"] == payload


def test_put_and_delete_methods_are_forwarded(upstream, make_proxy):
    # GIVEN
    proxy = make_proxy()

    # WHEN
    for method in ("PUT", "DELETE"):
        status, _ = method_request(
            method,
            f"{proxy.url}/resource/1",
            headers={"Host": "example.local"},
        )
        
        # THEN
        assert status == 200, f"{method} returned {status} instead of 200"
        assert upstream.last_request["method"] == method
