use crate::config::Config;
use crate::runtime_config::{
    ConfigReader, ConfigWriter, build_active_config, create_config_store,
    load_active_config_from_path,
};
use crate::worker::run_worker;
use std::net::SocketAddr;
use std::path::PathBuf;

pub struct Server {
    addr: SocketAddr,
    config_path: PathBuf,
    config_reader: ConfigReader,
    config_writer: ConfigWriter,
    workers: usize,
}

impl Server {
    pub fn new(config: Config, config_path: impl Into<PathBuf>) -> Result<Self, String> {
        let addr: SocketAddr = format!("0.0.0.0:{}", config.listen_port)
            .parse()
            .expect("Invalid listen address");
        let workers = config.workers;
        let active_config = build_active_config(config, 0)?;
        let (config_writer, config_reader) = create_config_store(active_config);

        Ok(Server {
            addr,
            config_path: config_path.into(),
            config_reader,
            config_writer,
            workers,
        })
    }

    pub fn start(self) {
        let Server {
            addr,
            config_path,
            config_reader,
            config_writer,
            workers,
        } = self;

        crate::log_info!("starting_workers", "count" => workers, "addr" => addr);
        spawn_reload_thread(config_path, addr.port(), workers, config_writer);

        let mut handles = Vec::with_capacity(workers);

        for id in 0..workers {
            let config_reader = config_reader.clone();

            let handle = std::thread::Builder::new()
                .name(format!("worker-thread-{}", id))
                .spawn(move || {
                    run_worker(id, addr, config_reader);
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

fn spawn_reload_thread(
    config_path: PathBuf,
    expected_listen_port: u16,
    expected_workers: usize,
    config_writer: ConfigWriter,
) {
    std::thread::Builder::new()
        .name("config-reload-thread".to_string())
        .spawn(move || {
            run_reload_thread(
                config_path,
                expected_listen_port,
                expected_workers,
                config_writer,
            );
        })
        .expect("Failed to spawn config reload thread");
}

#[cfg(unix)]
fn run_reload_thread(
    config_path: PathBuf,
    expected_listen_port: u16,
    expected_workers: usize,
    config_writer: ConfigWriter,
) {
    use tokio::signal::unix::{SignalKind, signal};

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("Failed to build Tokio runtime for reload thread");

    rt.block_on(async move {
        let mut sighup = signal(SignalKind::hangup()).expect("Failed to register SIGHUP handler");
        let mut generation = 0_u64;

        crate::log_info!(
            "config_reload_ready",
            "path" => config_path.display(),
            "signal" => "SIGHUP"
        );

        while sighup.recv().await.is_some() {
            crate::log_info!("config_reload_requested", "path" => config_path.display());

            match load_active_config_from_path(
                &config_path,
                expected_listen_port,
                expected_workers,
                generation + 1,
            ) {
                Ok(active_config) => {
                    generation = active_config.generation;
                    config_writer.store(active_config);
                    crate::log_info!(
                        "config_reloaded",
                        "path" => config_path.display(),
                        "generation" => generation
                    );
                }
                Err(error) => {
                    crate::log_error!(
                        "config_reload_failed",
                        "path" => config_path.display(),
                        "error" => error
                    );
                }
            }
        }
    });
}

#[cfg(not(unix))]
fn run_reload_thread(
    config_path: PathBuf,
    _expected_listen_port: u16,
    _expected_workers: usize,
    _config_writer: ConfigWriter,
) {
    crate::log_error!(
        "config_reload_unsupported",
        "path" => config_path.display(),
        "error" => "SIGHUP reload is only supported on Unix"
    );
}
