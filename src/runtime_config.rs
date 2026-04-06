use crate::config::Config;
use crate::parser::parse_proxy_config;
use crate::router::Router;
use crate::tls::build_tls_acceptor;
use std::fs;
use std::path::Path;
use std::sync::{Arc, RwLock};
use tokio_rustls::TlsAcceptor;

pub struct ActiveConfig {
    pub generation: u64,
    pub router: Arc<Router>,
    pub tls_acceptor: Option<TlsAcceptor>,
}

struct SharedConfig {
    current: RwLock<Arc<ActiveConfig>>,
}

pub struct ConfigWriter {
    shared: Arc<SharedConfig>,
}

#[derive(Clone)]
pub struct ConfigReader {
    shared: Arc<SharedConfig>,
}

impl ConfigWriter {
    pub fn store(&self, config: ActiveConfig) {
        let mut guard = self
            .shared
            .current
            .write()
            .expect("Live config lock poisoned");
        *guard = Arc::new(config);
    }
}

impl ConfigReader {
    pub fn load(&self) -> Arc<ActiveConfig> {
        let guard = self
            .shared
            .current
            .read()
            .expect("Live config lock poisoned");
        Arc::clone(&guard)
    }
}

pub fn create_config_store(initial: ActiveConfig) -> (ConfigWriter, ConfigReader) {
    let shared = Arc::new(SharedConfig {
        current: RwLock::new(Arc::new(initial)),
    });

    (
        ConfigWriter {
            shared: Arc::clone(&shared),
        },
        ConfigReader { shared },
    )
}

pub fn build_active_config(config: Config, generation: u64) -> Result<ActiveConfig, String> {
    let router = Arc::new(Router::new(config.routes));
    let tls_acceptor = build_tls_acceptor(&config.certs)
        .map_err(|e| format!("Failed to initialise TLS: {}", e))?;

    Ok(ActiveConfig {
        generation,
        router,
        tls_acceptor,
    })
}

pub fn load_config_from_path(path: &Path) -> Result<Config, String> {
    let contents = fs::read_to_string(path)
        .map_err(|e| format!("Failed to read proxy config '{}': {}", path.display(), e))?;

    parse_proxy_config(&contents)
        .map_err(|e| format!("Failed to parse proxy config '{}': {:?}", path.display(), e))
}

pub fn load_active_config_from_path(
    path: &Path,
    expected_listen_port: u16,
    expected_workers: usize,
    generation: u64,
) -> Result<ActiveConfig, String> {
    let config = load_config_from_path(path)?;

    if config.listen_port != expected_listen_port {
        return Err(format!(
            "Reload rejected: listen port changed from {} to {}",
            expected_listen_port, config.listen_port
        ));
    }

    if config.workers != expected_workers {
        return Err(format!(
            "Reload rejected: workers changed from {} to {}",
            expected_workers, config.workers
        ));
    }

    build_active_config(config, generation)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_config_path(test_name: &str) -> PathBuf {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "barebones-reverse-proxy-{}-{}-{}.conf",
            test_name,
            std::process::id(),
            suffix
        ))
    }

    fn write_temp_config(test_name: &str, contents: &str) -> PathBuf {
        let path = temp_config_path(test_name);
        fs::write(&path, contents).unwrap();
        path
    }

    #[test]
    fn reload_accepts_route_changes_when_startup_shape_matches() {
        let path = write_temp_config(
            "route-reload",
            r#"
            listen 8080;
            workers 2;
            route https://example.com/new http://localhost:4000;
            "#,
        );

        let active = load_active_config_from_path(&path, 8080, 2, 1).unwrap();
        let matched = active.router.match_route("example.com", "/new");

        assert!(matched.is_some());
        assert_eq!(active.generation, 1);

        let _ = fs::remove_file(path);
    }

    #[test]
    fn reload_rejects_listen_port_changes() {
        let path = write_temp_config(
            "listen-change",
            r#"
            listen 9090;
            workers 2;
            route https://example.com/api http://localhost:3000;
            "#,
        );

        let error = load_active_config_from_path(&path, 8080, 2, 1)
            .err()
            .expect("reload should reject listen port changes");

        assert!(error.contains("listen port changed"));

        let _ = fs::remove_file(path);
    }

    #[test]
    fn config_store_updates_are_visible_to_readers() {
        let initial = build_active_config(
            load_config_from_path(&write_temp_config(
                "initial-store",
                r#"
                listen 8080;
                workers 2;
                route https://example.com/api http://localhost:3000;
                "#,
            ))
            .unwrap(),
            0,
        )
        .unwrap();

        let (writer, reader) = create_config_store(initial);
        let updated = build_active_config(
            load_config_from_path(&write_temp_config(
                "updated-store",
                r#"
                listen 8080;
                workers 2;
                route https://example.com/admin http://localhost:4000;
                "#,
            ))
            .unwrap(),
            1,
        )
        .unwrap();

        writer.store(updated);

        let current = reader.load();
        let matched = current.router.match_route("example.com", "/admin");

        assert!(matched.is_some());
        assert_eq!(current.generation, 1);
    }
}
