use crate::proxy::{ProxyState, handle_request};
use crate::runtime_config::ConfigReader;
use hyper::Request;
use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper_util::rt::{TokioExecutor, TokioIo};
use hyper_util::server::conn::auto::Builder as ServerBuilder;
use socket2::{Domain, Protocol, Socket, Type};
use std::net::{SocketAddr, TcpListener as StdTcpListener};
use tokio::net::TcpListener;

/// Run a single worker thread.
///
/// Each worker builds its own single-threaded Tokio runtime, binds a
/// TcpListener to the shared address via SO_REUSEPORT, and runs an
/// independent accept loop.
pub fn run_worker(id: usize, addr: SocketAddr, config_reader: ConfigReader) {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("Failed to build Tokio runtime for worker");

    let local = tokio::task::LocalSet::new();

    local.block_on(&rt, async move {
        let listener = bind_reuseport(addr).expect("Failed to bind listener with SO_REUSEPORT");
        crate::log_info!("worker_listening", "id" => id, "addr" => addr);

        let state = ProxyState::new(config_reader.clone());

        loop {
            let (stream, peer_addr) = match listener.accept().await {
                Ok(conn) => conn,
                Err(e) => {
                    crate::log_error!("accept_error", "id" => id, "error" => e);
                    continue;
                }
            };

            let active_config = config_reader.load();
            let state = state.clone();
            let tls_acceptor = active_config.tls_acceptor.clone();

            tokio::task::spawn_local(async move {
                if let Some(acceptor) = tls_acceptor {
                    match acceptor.accept(stream).await {
                        Ok(tls_stream) => {
                            serve_connection(TokioIo::new(tls_stream), state, peer_addr).await;
                        }
                        Err(e) => {
                            crate::log_error!("tls_handshake_failed", "peer" => peer_addr, "error" => e);
                        }
                    }
                } else {
                    serve_connection(TokioIo::new(stream), state, peer_addr).await;
                }
            });
        }
    });
}

async fn serve_connection<I>(io: I, state: ProxyState, peer_addr: SocketAddr)
where
    I: hyper::rt::Read + hyper::rt::Write + Unpin + 'static,
{
    let service = service_fn(move |req: Request<Incoming>| {
        let state = state.clone();
        async move { handle_request(state, peer_addr, req).await }
    });

    let builder = ServerBuilder::new(TokioExecutor::new());

    if let Err(e) = builder.serve_connection(io, service).await {
        crate::log_error!("connection_error", "peer" => peer_addr, "error" => e);
    }
}

/// Create a TcpListener with SO_REUSEPORT so multiple workers can bind the same address.
fn bind_reuseport(addr: SocketAddr) -> std::io::Result<TcpListener> {
    let domain = if addr.is_ipv6() {
        Domain::IPV6
    } else {
        Domain::IPV4
    };

    let socket = Socket::new(domain, Type::STREAM, Some(Protocol::TCP))?;
    socket.set_reuse_port(true)?;
    socket.set_reuse_address(true)?;
    socket.set_nonblocking(true)?;
    socket.bind(&addr.into())?;
    socket.listen(1024)?;

    let std_listener: StdTcpListener = socket.into();
    TcpListener::from_std(std_listener)
}
