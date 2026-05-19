import os
import signal
import time

from tests.integration.test_utils import get

def test_sighup_reloads_config(upstream, make_proxy):
    # GIVEN
    proxy = make_proxy(request_host="old.local")
    
    status, _, _ = get(f"{proxy.url}/", headers={"Host": "old.local"})
    assert status == 200

    # WHEN
    # Rewrite config to use new.local and remove old.local
    new_config = (
        f"listen {proxy.port};\n"
        f"workers 1;\n"
        f"route http://new.local/ http://127.0.0.1:{upstream.port}/;\n"
    )
    with open(proxy.config_path, "w") as f:
        f.write(new_config)

    # Send SIGHUP to the proxy
    os.kill(proxy._proc.pid, signal.SIGHUP)
    
    # Wait for reload to process (usually instantaneous, but wait a bit)
    time.sleep(0.5)

    # THEN
    # Old route should now return 404
    status_old, _, _ = get(f"{proxy.url}/", headers={"Host": "old.local"})
    assert status_old == 404

    # New route should now return 200
    status_new, _, _ = get(f"{proxy.url}/", headers={"Host": "new.local"})
    assert status_new == 200
