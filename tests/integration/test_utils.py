from __future__ import annotations

import urllib.error
import urllib.request


class NoRedirectHandler(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        return None


def get(url: str, *, headers: dict | None = None, follow_redirects: bool = True) -> tuple[int, bytes, dict]:
    req = urllib.request.Request(url, headers=headers or {})
    opener = urllib.request.build_opener() if follow_redirects else urllib.request.build_opener(NoRedirectHandler)
    try:
        with opener.open(req, timeout=5) as resp:
            return resp.status, resp.read(), dict(resp.headers)
    except urllib.error.HTTPError as e:
        return e.code, e.read(), dict(e.headers)


def post(url: str, body: bytes, *, headers: dict | None = None) -> tuple[int, bytes]:
    req = urllib.request.Request(
        url, data=body, method="POST", headers=headers or {}
    )
    try:
        with urllib.request.urlopen(req, timeout=5) as resp:
            return resp.status, resp.read()
    except urllib.error.HTTPError as e:
        return e.code, e.read()


def method_request(
    method: str, url: str, *, headers: dict | None = None
) -> tuple[int, bytes]:
    req = urllib.request.Request(
        url, method=method, headers=headers or {}
    )
    try:
        with urllib.request.urlopen(req, timeout=5) as resp:
            return resp.status, resp.read()
    except urllib.error.HTTPError as e:
        return e.code, e.read()
