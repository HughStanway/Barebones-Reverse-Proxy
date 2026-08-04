from __future__ import annotations

from tests.integration.test_utils import get


def test_lru_cache_hit_and_miss(upstream, make_proxy):
    extra_config = """
    cache {
        enabled on;
        max_capacity_mb 10;
        max_file_size_mb 1;
        default_ttl_sec 300;
    }
    """
    proxy = make_proxy(extra_config=extra_config)
    url = f"http://127.0.0.1:{proxy.port}/static/app.js"

    # 1. First request for static asset -> MISS
    status1, body1, headers1 = get(url, headers={"Host": "example.local"})
    assert status1 == 200
    assert headers1.get("x-proxy-cache") == "MISS"

    # 2. Second request for static asset -> HIT
    status2, body2, headers2 = get(url, headers={"Host": "example.local"})
    assert status2 == 200
    assert headers2.get("x-proxy-cache") == "HIT"
    assert body2 == body1


def test_lru_cache_bypass_non_static(upstream, make_proxy):
    extra_config = """
    cache {
        enabled on;
    }
    """
    proxy = make_proxy(extra_config=extra_config)
    url = f"http://127.0.0.1:{proxy.port}/api/data"

    # Dynamic API endpoint -> BYPASS
    status, body, headers = get(url, headers={"Host": "example.local"})
    assert status == 200
    assert headers.get("x-proxy-cache") == "BYPASS"
