use crate::config::Config;
use crate::router::Router;
use crate::tls::build_tls_acceptor;
use crate::worker::run_worker;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio_rustls::TlsAcceptor;

pub struct Server {
    addr: SocketAddr,
    router: Arc<Router>,
    tls_acceptor: Option<TlsAcceptor>,
    workers: usize,
}

impl Server {
    pub fn new(config: Config) -> Self {
        let addr: SocketAddr = format!("0.0.0.0:{}", config.listen_port)
            .parse()
            .expect("Invalid listen address");

        let router = Arc::new(Router::new(config.routes));

        let tls_acceptor = config
            .tls
            .as_ref()
            .map(|tls_config| build_tls_acceptor(tls_config).expect("Failed to initialise TLS"));

        Server {
            addr,
            router,
            tls_acceptor,
            workers: config.workers,
        }
    }

    pub fn start(self) {
        crate::log_info!("starting_workers", "count" => self.workers, "addr" => self.addr);

        let mut handles = Vec::with_capacity(self.workers);

        for id in 0..self.workers {
            let addr = self.addr;
            let router = Arc::clone(&self.router);
            let tls_acceptor = self.tls_acceptor.clone();

            let handle = std::thread::Builder::new()
                .name(format!("worker-thread-{}", id))
                .spawn(move || {
                    run_worker(id, addr, router, tls_acceptor);
                })
                .expect("Failed to spawn worker thread");

            handles.push(handle);
        }

        // Block until all workers exit (they run indefinitely)
        for handle in handles {
            handle.join().expect("Worker thread panicked");
        }
    }
}
