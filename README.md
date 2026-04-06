# Barebones-Reverse-Proxy

A high-performance and modular reverse proxy built in Rust using the `hyper` ecosystem.

## Features

- **HTTP/1.1 & HTTP/2 Support**: Auto-negotiates the best available protocol.
- **HTTPS Termination with SNI**: Selects the correct certificate for each requested hostname during the TLS handshake.
- **Zero-Downtime Config Reload**: Reload routes and hostname-specific TLS certificates with `SIGHUP` without restarting the process.
- **Multi-threaded Worker Pool**: Uses `SO_REUSEPORT` to distribute load across multiple CPU cores with independent acceptor loops.
- **Connection Pooling**: Efficient upstream connection management for minimal latency.
- **Request Rewriting**: Flexible path mapping and automatic header injection (`X-Forwarded-For`, `X-Real-IP`, `Host`).
- **Modular Architecture**: Clean separation of concerns across 9 internal modules.

## Getting Started

### Installation & Execution
1. Clone the repository.
2. Create a `proxy.conf` (see [Configuration](#configuration) below).
3. (Optional) Configure one or more certificate blocks for HTTPS termination.
4. Run the server:
   ```bash
   make run
   ```

## Make Commands

| Command | Description |
|---|---|
| `make build` | Compile the project in debug mode |
| `make run` | Compile and start the proxy server |
| `make reload` | Reload the systemd service config via `systemctl reload` |
| `make test` | Run the unit and integration test suite |
| `make check` | Run a quick compilation check |
| `make lint` | Run Clippy for static analysis |
| `make fmt` | Format the codebase |
| `make release` | Build a production-optimized binary |
| `make clean` | Remove build artifacts |

## Architecture Overview

The system is designed with a "shared-nothing" concurrency model where each worker thread runs its own independent event loop.

```mermaid
graph TD
    A[main.rs] -->|parses config| B[parser.rs / config.rs]
    A -->|builds| C[server.rs]
    C -->|publishes live snapshot| RUNTIME[runtime_config.rs]
    C -->|spawns N threads| W[worker.rs]
    W -->|binds port via SO_REUSEPORT| L[TcpListener]
    W -->|loads live TLS snapshot| D[tls.rs]
    W -->|accepts connections| E[hyper Service]
    E -->|loads live router snapshot| F[router.rs]
    E -->|forwards request| G[proxy.rs]
    G -->|via pooled client| H[Upstream Server]
```

- **server.rs**: Orchestrates the startup and lifecycle of worker threads.
- **runtime_config.rs**: Builds and publishes immutable live config snapshots for workers to read.
- **worker.rs**: Manages a dedicated Tokio runtime and accept loop per thread.
- **proxy.rs**: The core proxy logic implementing the Hyper `Service` trait.
- **router.rs**: Encapsulates prefix-based route matching and URI rewriting logic.
- **tls.rs**: Builds the SNI-aware TLS acceptor and loads hostname-specific certificate/key pairs.

## Documentation

For a deeper dive into the technical internals, see:

- [Architecture Overview](docs/architecture.md)
- [Worker Threads & SO_REUSEPORT](docs/workers.md)
- [Event Loop & Task Scheduling](docs/event_loop.md)

## Configuration

The proxy is configured via `proxy.conf`. Example:

```protobuf
listen 443;
workers 2;

cert dashboard.asahi.tailbce682.ts.net {
    cert /var/lib/tailscale/certs/dashboard.asahi.tailbce682.ts.net.crt;
    key /var/lib/tailscale/certs/dashboard.asahi.tailbce682.ts.net.key;
}

cert grafana.asahi.tailbce682.ts.net {
    cert /var/lib/tailscale/certs/grafana.asahi.tailbce682.ts.net.crt;
    key /var/lib/tailscale/certs/grafana.asahi.tailbce682.ts.net.key;
}

route https://dashboard.asahi.tailbce682.ts.net/ http://localhost:3000/;
route https://grafana.asahi.tailbce682.ts.net/ http://localhost:3001/;
```

## Reloading Config

On Unix systems, the proxy reloads `proxy.conf` on `SIGHUP`.

- Route changes apply to new requests immediately.
- Hostname-specific TLS certificate and key changes apply to new TLS handshakes immediately.
- Existing connections continue running on the config snapshot they started with.
- `listen` and `workers` remain startup-only settings and are rejected during reload.

For a deployed systemd service, use:

```bash
make reload
```

The service unit uses `ExecReload=/bin/kill -HUP $MAINPID`, so `make reload` triggers an in-process config reload instead of a restart.
