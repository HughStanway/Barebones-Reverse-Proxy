import socket

def test_websocket_upgrade_bridging(upgrade_upstream, make_proxy):
    # GIVEN
    proxy = make_proxy(upstream_port=upgrade_upstream.port)

    # WHEN
    # Connect directly with raw socket to proxy
    s = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    s.connect(("127.0.0.1", proxy.port))
    
    upgrade_req = (
        b"GET / HTTP/1.1\r\n"
        b"Host: example.local\r\n"
        b"Connection: Upgrade\r\n"
        b"Upgrade: websocket\r\n\r\n"
    )
    s.sendall(upgrade_req)
    
    resp = s.recv(1024)
    
    # THEN
    assert b"101 Switching Protocols" in resp
    
    # Verify tunnel is established by sending echo payload
    s.sendall(b"Hello WebSocket")
    echo = s.recv(1024)
    assert echo == b"Hello WebSocket"
    
    s.close()
