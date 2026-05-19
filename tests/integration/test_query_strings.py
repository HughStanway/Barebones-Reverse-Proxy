from tests.integration.test_utils import get


def test_query_string_is_preserved(upstream, make_proxy):
    # GIVEN
    proxy = make_proxy()

    # WHEN
    get(
        f"{proxy.url}/?foo=bar&baz=qux",
        headers={"Host": "example.local"},
    )

    # THEN
    assert upstream.last_request.get("path") == "/?foo=bar&baz=qux"
