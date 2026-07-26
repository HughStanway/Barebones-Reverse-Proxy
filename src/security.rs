use crate::config::SecurityConfig;
use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Window duration for tracking TLS handshake failures and request rate limits (60 seconds).
const ROLLING_WINDOW: Duration = Duration::from_secs(60);

/// Default maximum allowed TLS failures within the rolling window before blacklisting (> 5 failures).
pub const DEFAULT_MAX_TLS_FAILURES: usize = 5;

/// Default duration an IP remains blacklisted (1 hour = 3600 seconds).
pub const DEFAULT_BAN_DURATION: Duration = Duration::from_secs(3600);

/// Default maximum allowed requests per minute per IP (60 requests per minute).
pub const DEFAULT_RATE_LIMIT_RPM: usize = 60;

#[derive(Debug, Clone)]
pub struct SecurityManager {
    inner: Arc<Mutex<SecurityManagerInner>>,
}

impl Default for SecurityManager {
    fn default() -> Self {
        Self::new(
            DEFAULT_BAN_DURATION,
            DEFAULT_MAX_TLS_FAILURES,
            DEFAULT_RATE_LIMIT_RPM,
        )
    }
}

#[derive(Debug)]
struct SecurityManagerInner {
    /// Maps IP address to timestamps of TLS handshake failures within the rolling window.
    tls_failures: HashMap<IpAddr, Vec<Instant>>,
    /// Maps IP address to timestamps of HTTP requests within the rolling 60s window.
    request_tracker: HashMap<IpAddr, Vec<Instant>>,
    /// Maps blacklisted IP address to its ban expiration timestamp (`banned_until`).
    blacklist: HashMap<IpAddr, Instant>,
    /// Duration of a ban when an IP is blacklisted.
    ban_duration: Duration,
    /// Maximum allowed TLS failures within the 60-second window before blacklisting.
    max_tls_failures: usize,
    /// Maximum allowed requests per minute per IP.
    rate_limit_rpm: usize,
}

impl SecurityManager {
    pub fn new(ban_duration: Duration, max_tls_failures: usize, rate_limit_rpm: usize) -> Self {
        Self {
            inner: Arc::new(Mutex::new(SecurityManagerInner {
                tls_failures: HashMap::new(),
                request_tracker: HashMap::new(),
                blacklist: HashMap::new(),
                ban_duration,
                max_tls_failures,
                rate_limit_rpm,
            })),
        }
    }

    pub fn from_security_config(sec: Option<&SecurityConfig>) -> Self {
        if let Some(sec) = sec {
            Self::new(
                Duration::from_secs(sec.ban_duration_sec),
                sec.max_tls_failures,
                sec.rate_limit_rpm,
            )
        } else {
            Self::default()
        }
    }

    /// Record a TLS handshake failure for a client IP.
    ///
    /// Cleans up failure timestamps older than 60 seconds and evicts expired bans.
    /// If an IP accumulates more than `max_tls_failures` within 60 seconds,
    /// it is added to the blacklist for `ban_duration` and `true` is returned.
    pub fn record_tls_failure(&self, ip: IpAddr) -> bool {
        let mut guard = self.inner.lock().expect("SecurityManager lock poisoned");
        let now = Instant::now();

        // Garbage collect expired bans and empty failure entries to prevent memory leaks
        guard.cleanup(now);

        let failures = guard.tls_failures.entry(ip).or_default();
        failures.retain(|&t| now.duration_since(t) <= ROLLING_WINDOW);
        failures.push(now);

        let failure_count = failures.len();
        let max_failures = guard.max_tls_failures;

        if failure_count > max_failures {
            let banned_until = now + guard.ban_duration;
            let was_already_banned = guard.blacklist.insert(ip, banned_until).is_some();
            if !was_already_banned {
                crate::log_error!(
                    "tls_failure_blacklist_triggered",
                    "ip" => ip,
                    "failures" => failure_count,
                    "ban_duration_sec" => guard.ban_duration.as_secs()
                );
            }
            true
        } else {
            false
        }
    }

    /// Record an HTTP request for a client IP and check if the rolling 60-second request rate limit is exceeded.
    ///
    /// Returns `Ok(current_count)` if within rate limit, or `Err(current_count)` if rate limit is exceeded.
    pub fn check_and_record_request(&self, ip: IpAddr) -> Result<usize, usize> {
        let mut guard = self.inner.lock().expect("SecurityManager lock poisoned");
        let now = Instant::now();

        guard.cleanup(now);

        let limit = guard.rate_limit_rpm;
        if limit == 0 {
            return Ok(0);
        }

        let requests = guard.request_tracker.entry(ip).or_default();
        requests.retain(|&t| now.duration_since(t) <= ROLLING_WINDOW);
        requests.push(now);

        let count = requests.len();
        if count > limit { Err(count) } else { Ok(count) }
    }

    /// Check if an IP address is currently blacklisted.
    pub fn is_blacklisted(&self, ip: &IpAddr) -> bool {
        let mut guard = self.inner.lock().expect("SecurityManager lock poisoned");
        let now = Instant::now();

        if let Some(&banned_until) = guard.blacklist.get(ip) {
            if now < banned_until {
                true
            } else {
                guard.blacklist.remove(ip);
                false
            }
        } else {
            false
        }
    }

    /// Retrieve the current TLS failure count for an IP in the 60-second window.
    pub fn get_tls_failure_count(&self, ip: &IpAddr) -> usize {
        let mut guard = self.inner.lock().expect("SecurityManager lock poisoned");
        let now = Instant::now();
        if let Some(failures) = guard.tls_failures.get_mut(ip) {
            failures.retain(|&t| now.duration_since(t) <= ROLLING_WINDOW);
            let count = failures.len();
            if count == 0 {
                guard.tls_failures.remove(ip);
            }
            count
        } else {
            0
        }
    }

    /// Manually add an IP address to the blacklist with the configured ban duration.
    pub fn add_to_blacklist(&self, ip: IpAddr) {
        let mut guard = self.inner.lock().expect("SecurityManager lock poisoned");
        let banned_until = Instant::now() + guard.ban_duration;
        guard.blacklist.insert(ip, banned_until);
    }
}

impl SecurityManagerInner {
    fn cleanup(&mut self, now: Instant) {
        // Evict expired bans
        self.blacklist.retain(|_, banned_until| now < *banned_until);

        // Evict IP entries whose failure timestamps have all expired
        self.tls_failures.retain(|_, failures| {
            failures.retain(|&t| now.duration_since(t) <= ROLLING_WINDOW);
            !failures.is_empty()
        });

        // Evict IP entries whose request timestamps have all expired
        self.request_tracker.retain(|_, requests| {
            requests.retain(|&t| now.duration_since(t) <= ROLLING_WINDOW);
            !requests.is_empty()
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;
    use std::thread;

    #[test]
    fn test_tls_failure_tracking_and_blacklisting() {
        let manager = SecurityManager::default();
        let ip = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 100));

        assert!(!manager.is_blacklisted(&ip));
        assert_eq!(manager.get_tls_failure_count(&ip), 0);

        // Record 5 failures - should not be blacklisted yet (threshold is > 5)
        for i in 1..=5 {
            let blacklisted = manager.record_tls_failure(ip);
            assert!(!blacklisted);
            assert!(!manager.is_blacklisted(&ip));
            assert_eq!(manager.get_tls_failure_count(&ip), i);
        }

        // 6th failure -> should trigger blacklist (> 5 failures within 60s)
        let blacklisted = manager.record_tls_failure(ip);
        assert!(blacklisted);
        assert!(manager.is_blacklisted(&ip));
        assert_eq!(manager.get_tls_failure_count(&ip), 6);
    }

    #[test]
    fn test_manual_blacklisting() {
        let manager = SecurityManager::default();
        let ip = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 5));

        assert!(!manager.is_blacklisted(&ip));
        manager.add_to_blacklist(ip);
        assert!(manager.is_blacklisted(&ip));
    }

    #[test]
    fn test_ban_expiration() {
        // Use a short ban duration of 50ms for testing
        let manager = SecurityManager::new(Duration::from_millis(50), 5, 60);
        let ip = IpAddr::V4(Ipv4Addr::new(172, 16, 0, 1));

        manager.add_to_blacklist(ip);
        assert!(manager.is_blacklisted(&ip));

        // Sleep long enough for the ban to expire
        thread::sleep(Duration::from_millis(60));

        assert!(!manager.is_blacklisted(&ip));
    }

    #[test]
    fn test_request_rate_limiting() {
        let manager = SecurityManager::new(Duration::from_secs(3600), 5, 3);
        let ip = IpAddr::V4(Ipv4Addr::new(192, 168, 2, 1));

        assert_eq!(manager.check_and_record_request(ip), Ok(1));
        assert_eq!(manager.check_and_record_request(ip), Ok(2));
        assert_eq!(manager.check_and_record_request(ip), Ok(3));

        // 4th request exceeds threshold (limit 3 per minute)
        assert_eq!(manager.check_and_record_request(ip), Err(4));
    }
}
