import http.server
import socketserver
import threading
import time
from tests.integration.test_utils import get


class DummyAuthServer:
    def __init__(self, port=0):
        self.requested_port = port
        self.port = port
        self.allow = True
        self.last_headers = {}
        self._server = None
        self._thread = None

    def start(self):
        auth_self = self

        class AuthHandler(http.server.BaseHTTPRequestHandler):
            def do_GET(self):
                auth_self.last_headers = {k.lower(): v for k, v in self.headers.items()}
                if auth_self.allow:
                    self.send_response(200)
                    self.send_header("Remote-User", "hugh")
                    self.send_header("Remote-Email", "hugh@example.com")
                    self.end_headers()
                else:
                    self.send_response(302)
                    self.send_header("Location", "https://auth.example.com/login")
                    self.end_headers()

            def log_message(self, format, *args):
                pass

        self._server = socketserver.TCPServer(("127.0.0.1", self.requested_port), AuthHandler)
        self.port = self._server.server_address[1]
        self._thread = threading.Thread(target=self._server.serve_forever, daemon=True)
        self._thread.start()

    def stop(self):
        if self._server:
            self._server.shutdown()
            self._server.server_close()


def test_forward_auth_success_and_header_injection(upstream, make_proxy):
    auth_server = DummyAuthServer(port=0)
    auth_server.start()
    try:
        extra_config = f"""
        security {{
            forward_auth http://127.0.0.1:{auth_server.port}/api/verify;
        }}
        route https://protected.local/ {{
            upstream http://localhost:{upstream.port}/;
            auth on;
        }}
        route https://public.local/ {{
            upstream http://localhost:{upstream.port}/;
            auth off;
        }}
        """
        proxy = make_proxy(extra_config=extra_config)

        # 1. Protected route with auth_server.allow = True -> 200 OK and remote-user header passed to upstream
        auth_server.allow = True
        status, _, _ = get(f"{proxy.url}/", headers={"Host": "protected.local", "Authorization": "Bearer secret_token"})
        assert status == 200
        assert upstream.last_request["headers"].get("remote-user") == "hugh"
        assert auth_server.last_headers.get("x-forwarded-host") == "protected.local"
        assert auth_server.last_headers.get("authorization") == "Bearer secret_token"

        # 2. Protected route with auth_server.allow = False -> 302 Redirect to auth portal
        auth_server.allow = False
        status, _, resp_headers = get(f"{proxy.url}/", headers={"Host": "protected.local"}, follow_redirects=False)
        assert status == 302
        assert resp_headers.get("location") == "https://auth.example.com/login"

        # 3. Public route -> Bypasses auth even when allow = False
        status, _, _ = get(f"{proxy.url}/", headers={"Host": "public.local"})
        assert status == 200

    finally:
        auth_server.stop()
