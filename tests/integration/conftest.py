from __future__ import annotations

import json
import os
import socket
import subprocess
import tempfile
import threading
import time
from http.server import BaseHTTPRequestHandler, HTTPServer
from pathlib import Path
from typing import Generator

import pytest

REPO_ROOT = Path(__file__).parent.parent.parent
BINARY = REPO_ROOT / "target" / "release" / "barebones_reverse_proxy"
PROXY_READY_TIMEOUT = 10


def _free_port() -> int:
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


def _wait_for_port(host: str, port: int, timeout: float) -> bool:
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        try:
            with socket.create_connection((host, port), timeout=0.2):
                return True
        except OSError:
            time.sleep(0.1)
    return False


@pytest.fixture(scope="session", autouse=True)
def proxy_binary() -> Path:
    result = subprocess.run(
        ["cargo", "build", "--release"],
        cwd=REPO_ROOT,
        capture_output=True,
        text=True,
    )
    if result.returncode != 0:
        pytest.fail(
            f"cargo build --release failed:\n{result.stderr}"
        )
    assert BINARY.exists(), f"Binary not found at {BINARY}"
    return BINARY


class _CaptureHandler(BaseHTTPRequestHandler):
    def log_message(self, *args, **kwargs) -> None:  # silence access log
        pass

    def _handle(self) -> None:
        length = int(self.headers.get("Content-Length", 0))
        body = self.rfile.read(length) if length else b""

        self.server.last_request = {  # type: ignore[attr-defined]
            "method": self.command,
            "path": self.path,
            "headers": dict(self.headers),
            "body": body,
        }

        response_body = (
            f'{{"method":"{self.command}","path":"{self.path}"}}'
        ).encode()

        status_code = int(self.headers.get("X-Mock-Status", 200))
        mock_headers_str = self.headers.get("X-Mock-Headers", "{}")
        mock_headers = json.loads(mock_headers_str)

        self.send_response(status_code)
        
        has_content_type = False
        for k, v in mock_headers.items():
            self.send_header(k, v)
            if k.lower() == "content-type":
                has_content_type = True
                
        if not has_content_type:
            self.send_header("Content-Type", "application/json")
            
        self.send_header("Content-Length", str(len(response_body)))
        self.end_headers()
        self.wfile.write(response_body)

    do_GET = _handle
    do_POST = _handle
    do_PUT = _handle
    do_DELETE = _handle
    do_PATCH = _handle
    do_HEAD = _handle


class UpstreamServer:
    def __init__(self, port: int) -> None:
        self.port = port
        self._server = HTTPServer(("127.0.0.1", port), _CaptureHandler)
        self._server.last_request: dict = {}  # type: ignore[attr-defined]
        self._thread = threading.Thread(
            target=self._server.serve_forever, daemon=True
        )
        self._thread.start()

    @property
    def url(self) -> str:
        return f"http://127.0.0.1:{self.port}"

    @property
    def last_request(self) -> dict:
        return self._server.last_request  # type: ignore[attr-defined]

    def stop(self) -> None:
        self._server.shutdown()
        self._thread.join(timeout=5)


@pytest.fixture()
def upstream() -> Generator[UpstreamServer, None, None]:
    server = UpstreamServer(_free_port())
    yield server
    server.stop()


class UpgradeUpstreamServer:
    def __init__(self, port: int) -> None:
        self.port = port
        self.sock = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        self.sock.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        self.sock.bind(("127.0.0.1", port))
        self.sock.listen(1)
        self.sock.settimeout(5.0)
        self.thread = threading.Thread(target=self._run, daemon=True)
        self.thread.start()
        
    def _run(self):
        try:
            conn, _ = self.sock.accept()
            with conn:
                conn.settimeout(2.0)
                req = b""
                while b"\r\n\r\n" not in req:
                    chunk = conn.recv(1024)
                    if not chunk:
                        break
                    req += chunk
                
                resp = (
                    b"HTTP/1.1 101 Switching Protocols\r\n"
                    b"Connection: Upgrade\r\n"
                    b"Upgrade: websocket\r\n\r\n"
                )
                conn.sendall(resp)
                
                while True:
                    data = conn.recv(1024)
                    if not data:
                        break
                    conn.sendall(data)
        except (socket.timeout, OSError):
            pass
            
    def stop(self) -> None:
        self.sock.close()
        self.thread.join(timeout=2)


@pytest.fixture()
def upgrade_upstream() -> Generator[UpgradeUpstreamServer, None, None]:
    server = UpgradeUpstreamServer(_free_port())
    yield server
    server.stop()


class ProxyProcess:
    def __init__(self, port: int, config_path: str, proc: subprocess.Popen) -> None:
        self.port = port
        self.config_path = config_path
        self._proc = proc

    @property
    def url(self) -> str:
        return f"http://127.0.0.1:{self.port}"

    def stop(self) -> None:
        self._proc.terminate()
        try:
            self._proc.wait(timeout=5)
        except subprocess.TimeoutExpired:
            self._proc.kill()


def _write_proxy_config(
    config_dir: str,
    proxy_port: int,
    upstream_port: int,
    *,
    request_host: str = "example.local",
    request_path: str = "/",
    forward_path: str = "/",
    extra_routes: list[tuple[str, str]] | None = None,
    security_block: str | None = None,
) -> str:
    primary = (
        f"route http://{request_host}{request_path} "
        f"http://127.0.0.1:{upstream_port}{forward_path};"
    )

    extra = ""
    if extra_routes:
        for req_ep, fwd_ep in extra_routes:
            extra += f"\nroute {req_ep} {fwd_ep};"

    security = f"\n{security_block}\n" if security_block else ""
    content = f"listen {proxy_port};\nworkers 1;\n{primary}{extra}{security}\n"

    config_path = os.path.join(config_dir, "proxy.conf")
    with open(config_path, "w") as f:
        f.write(content)
    return config_path


@pytest.fixture()
def make_proxy(upstream, proxy_binary):
    created: list[ProxyProcess] = []
    tmpdir = tempfile.mkdtemp(prefix="brp-test-")

    def _factory(upstream_port: int | None = None, **kwargs) -> ProxyProcess:
        proxy_port = _free_port()
        config_path = _write_proxy_config(
            tmpdir,
            proxy_port,
            upstream_port if upstream_port is not None else upstream.port,
            **kwargs,
        )
        proc = subprocess.Popen(
            [str(proxy_binary)],
            cwd=tmpdir,
            env={**os.environ, "RUST_LOG": "error"},
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
        if not _wait_for_port("127.0.0.1", proxy_port, PROXY_READY_TIMEOUT):
            proc.kill()
            pytest.fail(
                f"Proxy did not become ready on port {proxy_port} "
                f"within {PROXY_READY_TIMEOUT}s"
            )
        p = ProxyProcess(proxy_port, config_path, proc)
        created.append(p)
        return p

    yield _factory

    for p in created:
        p.stop()
