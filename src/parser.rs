use crate::config::{CacheConfig, Config, Route, SecurityConfig};
use crate::error::ParseError;
use std::collections::HashSet;

fn is_valid_url(url: &str) -> bool {
    // Basic URL validation: must contain :// or start with /
    url.contains("://") || url.starts_with('/') || url.contains(':')
}

fn parse_listen_line(line: &str) -> Result<u16, ParseError> {
    let line_without_semicolon = line.trim_end_matches(';').trim();
    let parts: Vec<&str> = line_without_semicolon.split_whitespace().collect();
    if parts.len() != 2 {
        return Err(ParseError::InvalidListenDirective);
    }

    parts[1]
        .parse::<u16>()
        .map_err(|_| ParseError::InvalidPort {
            value: parts[1].to_string(),
        })
}

fn validate_semicolon(line: &str) -> Result<(), ParseError> {
    if !line.trim().ends_with(';') {
        return Err(ParseError::MissingSemicolon {
            line: line.to_string(),
        });
    }
    Ok(())
}

fn validate_directive_case(directive: &str) -> Result<(), ParseError> {
    if directive != directive.to_lowercase() {
        return Err(ParseError::InvalidDirectiveCase {
            directive: directive.to_string(),
        });
    }
    Ok(())
}

fn get_directive(line: &str) -> Result<&str, ParseError> {
    let parts: Vec<&str> = line.split_whitespace().collect();
    if parts.is_empty() {
        return Err(ParseError::UnknownDirective {
            directive: "".to_string(),
        });
    }
    Ok(parts[0])
}

fn validate_known_top_level_directive(directive: &str) -> Result<(), ParseError> {
    match directive {
        "listen" | "route" | "workers" | "logfile" | "security" | "cache" | "intercept_errors" => {
            Ok(())
        }
        _ => Err(ParseError::UnknownDirective {
            directive: directive.to_string(),
        }),
    }
}

fn parse_single_value_directive<'a>(line: &'a str, directive: &str) -> Result<&'a str, ParseError> {
    let line_without_semicolon = line.trim_end_matches(';').trim();
    let parts: Vec<&str> = line_without_semicolon.split_whitespace().collect();
    if parts.len() != 2 {
        return Err(ParseError::UnknownDirective {
            directive: directive.to_string(),
        });
    }
    Ok(parts[1])
}

fn check_trailing_garbage(line: &str) -> Result<(), ParseError> {
    let trimmed = line.trim();
    if !trimmed.ends_with(';') {
        return Ok(());
    }

    if let Some(semicolon_pos) = trimmed.rfind(';') {
        let after_semicolon = trimmed[semicolon_pos + 1..].trim();
        if !after_semicolon.is_empty() {
            return Err(ParseError::InvalidListenDirective);
        }
    }

    Ok(())
}

fn parse_route_block_header(line: &str) -> Result<String, ParseError> {
    let parts: Vec<&str> = line.split_whitespace().collect();
    if parts.len() != 3 || parts[0] != "route" || parts[2] != "{" {
        return Err(ParseError::InvalidRouteDirective {
            value: line.to_string(),
        });
    }

    let endpoint = parts[1].to_string();
    if !is_valid_url(&endpoint) {
        return Err(ParseError::InvalidUrlFormat { value: endpoint });
    }

    Ok(endpoint)
}

fn parse_route_block(
    lines: &[&str],
    index: &mut usize,
    request_endpoint: String,
) -> Result<Route, ParseError> {
    let mut forward_endpoint: Option<String> = None;
    let mut auth_required = false;
    let mut cert_path: Option<String> = None;
    let mut key_path: Option<String> = None;
    let mut cache: Option<bool> = None;
    let mut intercept_errors: Option<bool> = None;

    *index += 1;

    while *index < lines.len() {
        let line = lines[*index];

        if line == "}" || line == "};" {
            let forward_endpoint =
                forward_endpoint.ok_or_else(|| ParseError::InvalidRouteDirective {
                    value: request_endpoint.clone(),
                })?;

            if request_endpoint.starts_with("https://") {
                if cert_path.is_none() || key_path.is_none() {
                    return Err(ParseError::IncompleteRouteCertBlock {
                        endpoint: request_endpoint,
                    });
                }
            } else if cert_path.is_some() || key_path.is_some() {
                return Err(ParseError::CertNotAllowedForHttpRoute {
                    endpoint: request_endpoint,
                });
            }

            return Ok(Route {
                request_endpoint,
                forward_endpoint,
                auth_required,
                cert_path,
                key_path,
                cache,
                intercept_errors,
            });
        }

        if line.ends_with('{') {
            return Err(ParseError::InvalidRouteDirective {
                value: line.to_string(),
            });
        }

        validate_semicolon(line)?;
        let directive = get_directive(line)?;
        validate_directive_case(directive)?;
        check_trailing_garbage(line)?;

        match directive {
            "upstream" | "forward" => {
                let value = parse_single_value_directive(line, directive)?;
                if !is_valid_url(value) {
                    return Err(ParseError::InvalidUrlFormat {
                        value: value.to_string(),
                    });
                }
                forward_endpoint = Some(value.to_string());
            }
            "auth" => {
                let value = parse_single_value_directive(line, "auth")?;
                auth_required = match value {
                    "on" | "yes" | "true" => true,
                    "off" | "no" | "false" => false,
                    _ => {
                        return Err(ParseError::InvalidRouteDirective {
                            value: line.to_string(),
                        });
                    }
                };
            }
            "cert" => {
                let value = parse_single_value_directive(line, "cert")?;
                if cert_path.is_some() {
                    return Err(ParseError::InvalidRouteDirective {
                        value: line.to_string(),
                    });
                }
                cert_path = Some(value.to_string());
            }
            "key" => {
                let value = parse_single_value_directive(line, "key")?;
                if key_path.is_some() {
                    return Err(ParseError::InvalidRouteDirective {
                        value: line.to_string(),
                    });
                }
                key_path = Some(value.to_string());
            }
            "cache" => {
                let value = parse_single_value_directive(line, "cache")?;
                cache = match value {
                    "on" | "yes" | "true" => Some(true),
                    "off" | "no" | "false" => Some(false),
                    _ => {
                        return Err(ParseError::InvalidRouteDirective {
                            value: line.to_string(),
                        });
                    }
                };
            }
            "intercept_errors" => {
                let value = parse_single_value_directive(line, "intercept_errors")?;
                intercept_errors = match value {
                    "on" | "yes" | "true" => Some(true),
                    "off" | "no" | "false" => Some(false),
                    _ => {
                        return Err(ParseError::InvalidRouteDirective {
                            value: line.to_string(),
                        });
                    }
                };
            }
            _ => {
                return Err(ParseError::InvalidRouteDirective {
                    value: line.to_string(),
                });
            }
        }

        *index += 1;
    }

    Err(ParseError::InvalidRouteDirective {
        value: request_endpoint,
    })
}

fn parse_security_block(lines: &[&str], index: &mut usize) -> Result<SecurityConfig, ParseError> {
    let mut proxy_protocol: Option<bool> = None;
    let mut trusted_upstream: Option<std::net::IpAddr> = None;
    let mut timeout_ms: Option<u64> = None;
    let mut max_tls_failures: Option<usize> = None;
    let mut ban_duration_sec: Option<u64> = None;
    let mut rate_limit_rpm: Option<usize> = None;
    let mut forward_auth: Option<String> = None;
    let mut max_body_size: Option<usize> = None;
    let mut max_header_size: Option<usize> = None;

    *index += 1;

    while *index < lines.len() {
        let line = lines[*index];

        if line == "}" || line == "};" {
            let pp = proxy_protocol.unwrap_or(false);
            let tu = if pp {
                trusted_upstream.ok_or(ParseError::MissingSecurityDirective {
                    directive: "trusted_upstream".to_string(),
                })?
            } else {
                trusted_upstream.unwrap_or_else(|| "0.0.0.0".parse().unwrap())
            };
            let t_ms = timeout_ms.unwrap_or(200);

            return Ok(SecurityConfig {
                proxy_protocol: pp,
                trusted_upstream: tu,
                timeout_ms: t_ms,
                max_tls_failures: max_tls_failures.unwrap_or(5),
                ban_duration_sec: ban_duration_sec.unwrap_or(3600),
                rate_limit_rpm: rate_limit_rpm.unwrap_or(300),
                forward_auth,
                max_body_size: max_body_size.unwrap_or(10 * 1024 * 1024),
                max_header_size: max_header_size.unwrap_or(64 * 1024),
            });
        }

        if line.ends_with('{') {
            return Err(ParseError::InvalidSecurityBlock {
                value: line.to_string(),
            });
        }

        validate_semicolon(line)?;
        let directive = get_directive(line)?;
        validate_directive_case(directive)?;
        check_trailing_garbage(line)?;

        match directive {
            "proxy_protocol" => {
                let value = parse_single_value_directive(line, "proxy_protocol")?;
                if proxy_protocol.is_some() {
                    return Err(ParseError::DuplicateSecurityDirective {
                        directive: "proxy_protocol".to_string(),
                    });
                }
                let val = match value {
                    "on" | "true" => true,
                    "off" | "false" => false,
                    _ => {
                        return Err(ParseError::InvalidSecurityValue {
                            directive: "proxy_protocol".to_string(),
                            value: value.to_string(),
                        });
                    }
                };
                proxy_protocol = Some(val);
            }
            "trusted_upstream" => {
                let value = parse_single_value_directive(line, "trusted_upstream")?;
                if trusted_upstream.is_some() {
                    return Err(ParseError::DuplicateSecurityDirective {
                        directive: "trusted_upstream".to_string(),
                    });
                }
                let ip = value.parse::<std::net::IpAddr>().map_err(|_| {
                    ParseError::InvalidSecurityValue {
                        directive: "trusted_upstream".to_string(),
                        value: value.to_string(),
                    }
                })?;
                trusted_upstream = Some(ip);
            }
            "timeout" => {
                let value = parse_single_value_directive(line, "timeout")?;
                if timeout_ms.is_some() {
                    return Err(ParseError::DuplicateSecurityDirective {
                        directive: "timeout".to_string(),
                    });
                }
                let ms = value
                    .parse::<u64>()
                    .map_err(|_| ParseError::InvalidSecurityValue {
                        directive: "timeout".to_string(),
                        value: value.to_string(),
                    })?;
                timeout_ms = Some(ms);
            }
            "max_tls_failures" => {
                let value = parse_single_value_directive(line, "max_tls_failures")?;
                if max_tls_failures.is_some() {
                    return Err(ParseError::DuplicateSecurityDirective {
                        directive: "max_tls_failures".to_string(),
                    });
                }
                let count =
                    value
                        .parse::<usize>()
                        .map_err(|_| ParseError::InvalidSecurityValue {
                            directive: "max_tls_failures".to_string(),
                            value: value.to_string(),
                        })?;
                max_tls_failures = Some(count);
            }
            "ban_duration" => {
                let value = parse_single_value_directive(line, "ban_duration")?;
                if ban_duration_sec.is_some() {
                    return Err(ParseError::DuplicateSecurityDirective {
                        directive: "ban_duration".to_string(),
                    });
                }
                let sec = value
                    .parse::<u64>()
                    .map_err(|_| ParseError::InvalidSecurityValue {
                        directive: "ban_duration".to_string(),
                        value: value.to_string(),
                    })?;
                ban_duration_sec = Some(sec);
            }
            "rate_limit_rpm" | "rate_limit" => {
                let value = parse_single_value_directive(line, directive)?;
                if rate_limit_rpm.is_some() {
                    return Err(ParseError::DuplicateSecurityDirective {
                        directive: directive.to_string(),
                    });
                }
                let rpm = value
                    .parse::<usize>()
                    .map_err(|_| ParseError::InvalidSecurityValue {
                        directive: directive.to_string(),
                        value: value.to_string(),
                    })?;
                rate_limit_rpm = Some(rpm);
            }
            "forward_auth" => {
                let value = parse_single_value_directive(line, "forward_auth")?;
                if forward_auth.is_some() {
                    return Err(ParseError::DuplicateSecurityDirective {
                        directive: "forward_auth".to_string(),
                    });
                }
                forward_auth = Some(value.to_string());
            }
            "max_body_size" => {
                let value = parse_single_value_directive(line, "max_body_size")?;
                if max_body_size.is_some() {
                    return Err(ParseError::DuplicateSecurityDirective {
                        directive: "max_body_size".to_string(),
                    });
                }
                let size = parse_size_string(value, "max_body_size")?;
                max_body_size = Some(size);
            }
            "max_header_size" => {
                let value = parse_single_value_directive(line, "max_header_size")?;
                if max_header_size.is_some() {
                    return Err(ParseError::DuplicateSecurityDirective {
                        directive: "max_header_size".to_string(),
                    });
                }
                let size = parse_size_string(value, "max_header_size")?;
                max_header_size = Some(size);
            }
            _ => {
                return Err(ParseError::InvalidSecurityBlock {
                    value: line.to_string(),
                });
            }
        }

        *index += 1;
    }

    Err(ParseError::UnterminatedSecurityBlock)
}

pub fn parse_size_string(s: &str, directive_name: &str) -> Result<usize, ParseError> {
    let s = s.trim();
    if s.is_empty() {
        return Err(ParseError::InvalidSecurityValue {
            directive: directive_name.to_string(),
            value: s.to_string(),
        });
    }

    let (num_str, multiplier) = if s.ends_with(['k', 'K']) {
        (&s[..s.len() - 1], 1024usize)
    } else if s.ends_with(['m', 'M']) {
        (&s[..s.len() - 1], 1024usize * 1024)
    } else if s.ends_with(['g', 'G']) {
        (&s[..s.len() - 1], 1024usize * 1024 * 1024)
    } else {
        (s, 1usize)
    };

    let num: usize = num_str
        .parse()
        .map_err(|_| ParseError::InvalidSecurityValue {
            directive: directive_name.to_string(),
            value: s.to_string(),
        })?;

    num.checked_mul(multiplier)
        .ok_or_else(|| ParseError::InvalidSecurityValue {
            directive: directive_name.to_string(),
            value: s.to_string(),
        })
}

fn strip_comments(input: &str) -> String {
    let mut result = String::with_capacity(input.len());
    let chars: Vec<char> = input.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        if chars[i] == '/' && i + 1 < chars.len() {
            let next_char = chars[i + 1];
            if next_char == '/' || next_char == '*' {
                // Check if it satisfies the lexical boundary conditions:
                // 1. Start of line (only whitespace before it on current line)
                // 2. Preceded by whitespace (space, tab)
                // 3. Preceded by semicolon (';')
                // 4. Preceded by newline ('\n')
                let mut is_boundary = false;
                if i == 0 {
                    is_boundary = true;
                } else {
                    let prev_char = chars[i - 1];
                    if prev_char == ' '
                        || prev_char == '\t'
                        || prev_char == ';'
                        || prev_char == '\n'
                    {
                        is_boundary = true;
                    } else {
                        // Check if it's the start of the line (only whitespace before it)
                        let mut temp = i;
                        let mut only_whitespace = true;
                        while temp > 0 {
                            temp -= 1;
                            let c = chars[temp];
                            if c == '\n' {
                                break;
                            }
                            if c != ' ' && c != '\t' {
                                only_whitespace = false;
                                break;
                            }
                        }
                        if only_whitespace {
                            is_boundary = true;
                        }
                    }
                }

                if is_boundary {
                    if next_char == '/' {
                        // Single-line comment. Replace comment content with spaces.
                        result.push(' ');
                        result.push(' ');
                        i += 2;
                        while i < chars.len() && chars[i] != '\n' {
                            result.push(' ');
                            i += 1;
                        }
                        continue;
                    } else {
                        // Multi-line comment. Replace comment content with spaces, preserving newlines.
                        result.push(' ');
                        result.push(' ');
                        i += 2;
                        while i < chars.len() {
                            if chars[i] == '*' && i + 1 < chars.len() && chars[i + 1] == '/' {
                                result.push(' ');
                                result.push(' ');
                                i += 2;
                                break;
                            } else if chars[i] == '\n' {
                                result.push('\n');
                            } else {
                                result.push(' ');
                            }
                            i += 1;
                        }
                        continue;
                    }
                }
            }
        }
        result.push(chars[i]);
        i += 1;
    }
    result
}

pub fn parse_proxy_config(input: &str) -> Result<Config, ParseError> {
    let stripped = strip_comments(input);
    let lines: Vec<&str> = stripped
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .collect();

    let mut listen_port: u16 = 0;
    let mut listen_found = false;
    let mut routes = Vec::new();
    let mut routes_found = false;
    let mut request_endpoints = HashSet::new();
    let mut workers: Option<usize> = None;
    let mut logfile: Option<String> = None;
    let mut security: Option<SecurityConfig> = None;
    let mut cache_config: Option<CacheConfig> = None;
    let mut intercept_errors: Option<bool> = None;

    let mut index = 0;
    while index < lines.len() {
        let line = lines[index];

        if line == "}" || line == "};" {
            return Err(ParseError::UnexpectedBlockTerminator);
        }

        let directive = get_directive(line)?;
        validate_directive_case(directive)?;
        validate_known_top_level_directive(directive)?;

        match directive {
            "listen" => {
                validate_semicolon(line)?;
                check_trailing_garbage(line)?;

                if listen_found {
                    return Err(ParseError::TooManyListenDirectives);
                }
                listen_port = parse_listen_line(line)?;
                listen_found = true;
            }
            "logfile" => {
                validate_semicolon(line)?;
                check_trailing_garbage(line)?;

                if logfile.is_some() {
                    return Err(ParseError::TooManyLogfileDirectives);
                }
                let value = parse_single_value_directive(line, "logfile")?;
                logfile = Some(value.to_string());
            }
            "intercept_errors" => {
                validate_semicolon(line)?;
                check_trailing_garbage(line)?;

                if intercept_errors.is_some() {
                    return Err(ParseError::TooManySecurityDirectives);
                }
                let value = parse_single_value_directive(line, "intercept_errors")?;
                let enabled = match value {
                    "on" | "yes" | "true" => true,
                    "off" | "no" | "false" => false,
                    _ => {
                        return Err(ParseError::UnknownDirective {
                            directive: line.to_string(),
                        });
                    }
                };
                intercept_errors = Some(enabled);
            }
            "route" if line.ends_with('{') => {
                let endpoint = parse_route_block_header(line)?;
                if request_endpoints.contains(&endpoint) {
                    return Err(ParseError::DuplicateRequestEndpoint {
                        value: endpoint.clone(),
                    });
                }
                let route = parse_route_block(&lines, &mut index, endpoint)?;
                request_endpoints.insert(route.request_endpoint.clone());
                routes.push(route);
                routes_found = true;
            }
            "route" => {
                return Err(ParseError::InvalidRouteDirective {
                    value: line.to_string(),
                });
            }
            "security" if line.ends_with('{') => {
                if security.is_some() {
                    return Err(ParseError::TooManySecurityDirectives);
                }
                let sec = parse_security_block(&lines, &mut index)?;
                security = Some(sec);
            }
            "security" => {
                return Err(ParseError::InvalidSecurityBlock {
                    value: line.to_string(),
                });
            }
            "workers" => {
                validate_semicolon(line)?;
                check_trailing_garbage(line)?;

                if workers.is_some() {
                    return Err(ParseError::TooManyWorkersDirectives);
                }
                let value = parse_single_value_directive(line, "workers")?;
                let n = value
                    .parse::<usize>()
                    .map_err(|_| ParseError::InvalidWorkersValue {
                        value: value.to_string(),
                    })?;
                if n == 0 {
                    return Err(ParseError::InvalidWorkersValue {
                        value: value.to_string(),
                    });
                }
                workers = Some(n);
            }
            "cache" if line.ends_with('{') => {
                if cache_config.is_some() {
                    return Err(ParseError::TooManySecurityDirectives);
                }
                let c = parse_cache_block(&lines, &mut index)?;
                cache_config = Some(c);
            }
            "cache" => {
                return Err(ParseError::InvalidSecurityBlock {
                    value: line.to_string(),
                });
            }
            _ => unreachable!(),
        }

        index += 1;
    }

    if !listen_found {
        return Err(ParseError::NoListenDirective);
    }

    if !routes_found {
        return Err(ParseError::NoRouteDirective);
    }

    if routes.iter().any(|r| r.auth_required) {
        let has_forward_auth = security
            .as_ref()
            .and_then(|s| s.forward_auth.as_ref())
            .is_some();

        if !has_forward_auth {
            return Err(ParseError::MissingSecurityDirective {
                directive: "forward_auth".to_string(),
            });
        }
    }

    let workers = workers.unwrap_or_else(|| {
        std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1)
    });

    Ok(Config {
        listen_port,
        routes,
        workers,
        logfile,
        security,
        cache: cache_config,
        intercept_errors,
    })
}

fn parse_cache_block(lines: &[&str], index: &mut usize) -> Result<CacheConfig, ParseError> {
    let mut enabled = true;
    let mut max_capacity_bytes = 64 * 1024 * 1024;
    let mut max_file_size_bytes = 2 * 1024 * 1024;
    let mut default_ttl_sec = 300;

    *index += 1;

    while *index < lines.len() {
        let line = lines[*index];

        if line == "}" || line == "};" {
            return Ok(CacheConfig {
                enabled,
                max_capacity_bytes,
                max_file_size_bytes,
                default_ttl_sec,
            });
        }

        if line.ends_with('{') {
            return Err(ParseError::InvalidSecurityBlock {
                value: line.to_string(),
            });
        }

        validate_semicolon(line)?;
        let directive = get_directive(line)?;
        validate_directive_case(directive)?;
        check_trailing_garbage(line)?;

        match directive {
            "enabled" | "cache" => {
                let value = parse_single_value_directive(line, directive)?;
                enabled = match value {
                    "on" | "true" | "yes" => true,
                    "off" | "false" | "no" => false,
                    _ => {
                        return Err(ParseError::InvalidSecurityValue {
                            directive: directive.to_string(),
                            value: value.to_string(),
                        });
                    }
                };
            }
            "max_capacity" | "max_capacity_mb" => {
                let value = parse_single_value_directive(line, directive)?;
                let val_str =
                    if directive.ends_with("_mb") && value.chars().all(|c| c.is_ascii_digit()) {
                        format!("{}M", value)
                    } else {
                        value.to_string()
                    };
                max_capacity_bytes = parse_size_string(&val_str, directive)?;
            }
            "max_file_size" | "max_file_size_mb" => {
                let value = parse_single_value_directive(line, directive)?;
                let val_str =
                    if directive.ends_with("_mb") && value.chars().all(|c| c.is_ascii_digit()) {
                        format!("{}M", value)
                    } else {
                        value.to_string()
                    };
                max_file_size_bytes = parse_size_string(&val_str, directive)?;
            }
            "default_ttl" | "default_ttl_sec" | "ttl" => {
                let value = parse_single_value_directive(line, directive)?;
                default_ttl_sec =
                    value
                        .parse::<u64>()
                        .map_err(|_| ParseError::InvalidSecurityValue {
                            directive: directive.to_string(),
                            value: value.to_string(),
                        })?;
            }
            _ => {
                return Err(ParseError::InvalidSecurityBlock {
                    value: line.to_string(),
                });
            }
        }

        *index += 1;
    }

    Err(ParseError::UnterminatedSecurityBlock)
}

/*
* UNIT TESTS
*/

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_basic_valid_config() {
        let input: &str = r#"
            listen 8080;
            route http://dashboard.myserver.home/api {
                upstream http://localhost:3000;
            }
            "#;

        let config: Config = parse_proxy_config(input).unwrap();

        assert_eq!(config.listen_port, 8080);
        assert_eq!(config.routes.len(), 1);
        assert_eq!(
            config.routes[0].request_endpoint,
            "http://dashboard.myserver.home/api"
        );
        assert_eq!(config.routes[0].forward_endpoint, "http://localhost:3000");
    }

    #[test]
    fn test_parse_config_with_multiple_routes() {
        let input: &str = r#"
            listen 8080;
            route /api {
                upstream http://localhost:3000;
            }
            route /auth {
                upstream http://localhost:4000;
            }
        "#;

        let result: Config = parse_proxy_config(input).unwrap();

        assert_eq!(result.routes.len(), 2);
    }

    #[test]
    fn test_parse_listen_port_line_too_many_arguments() {
        let input: &str = r#"
            listen 8080 443;
            route http://dashboard.myserver.local/api {
                upstream http://localhost:3000;
            }
            "#;

        let config: Result<Config, ParseError> = parse_proxy_config(input);

        assert!(config.is_err());
        assert!(matches!(config, Err(ParseError::InvalidListenDirective)));
    }

    #[test]
    fn test_parse_listen_port_not_valid_u16_type() {
        let cases = vec!["abc", "-1", "70000"];

        for port in cases {
            let input = format!(
                "listen {};\nroute /api {{\n    upstream http://localhost:3000;\n}}",
                port
            );
            let config: Result<Config, ParseError> = parse_proxy_config(&input);

            assert!(config.is_err());
            assert_eq!(
                config.unwrap_err(),
                ParseError::InvalidPort {
                    value: port.to_string()
                }
            );
        }
    }

    #[test]
    fn test_parse_no_listen_directive_given() {
        let input: &str = r#"
            route http://dashboard.myserver.local/api {
                upstream http://localhost:3000;
            }
            "#;

        let config: Result<Config, ParseError> = parse_proxy_config(input);

        assert!(config.is_err());
        assert!(matches!(config, Err(ParseError::NoListenDirective)));
    }

    #[test]
    fn test_parse_multiple_listen_directives_given() {
        let input: &str = r#"
            listen 8080;
            listen 443;
            route http://dashboard.myserver.local/api {
                upstream http://localhost:3000;
            }
            "#;

        let config: Result<Config, ParseError> = parse_proxy_config(input);

        assert!(config.is_err());
        assert!(matches!(config, Err(ParseError::TooManyListenDirectives)));
    }

    #[test]
    fn test_parse_route_line_too_many_arguments() {
        let input: &str = r#"
            listen 8080;
            route https://dashboard.myserver.local/api http://localhost:3000 http://localhost:3001;
            "#;

        let config: Result<Config, ParseError> = parse_proxy_config(input);

        assert!(config.is_err());
        assert_eq!(
            config.unwrap_err(),
            ParseError::InvalidRouteDirective {
                value: "route https://dashboard.myserver.local/api http://localhost:3000 http://localhost:3001;".to_string()
            }
        );
    }

    #[test]
    fn test_parse_route_missing_arguments() {
        let input: &str = r#"
            listen 8080;
            route /api;
        "#;

        let config: Result<Config, ParseError> = parse_proxy_config(input);

        assert_eq!(
            config.unwrap_err(),
            ParseError::InvalidRouteDirective {
                value: "route /api;".to_string()
            }
        );
    }

    #[test]
    fn test_parse_no_route_directives_given() {
        let input: &str = r#"
            listen 8080;
            "#;

        let config: Result<Config, ParseError> = parse_proxy_config(input);

        assert!(config.is_err());
        assert!(matches!(config, Err(ParseError::NoRouteDirective)));
    }

    #[test]
    fn test_partial_invalid_config() {
        let input: &str = r#"
        listen 8080;
        route /api {
            upstream http://localhost:3000;
        }
        route invalid;
    "#;

        let config: Result<Config, ParseError> = parse_proxy_config(input);

        assert!(config.is_err());
        assert_eq!(
            config.unwrap_err(),
            ParseError::InvalidRouteDirective {
                value: "route invalid;".to_string()
            }
        );
    }

    #[test]
    fn test_parse_duplicate_request_endpoint_routes() {
        let input: &str = r#"
            listen 8080;
            route /api {
                upstream http://localhost:3000;
            }
            route /api {
                upstream http://localhost:4000;
            }
        "#;

        let config: Result<Config, ParseError> = parse_proxy_config(input);

        assert!(config.is_err());
        assert_eq!(
            config.unwrap_err(),
            ParseError::DuplicateRequestEndpoint {
                value: "/api".to_string()
            }
        );
    }

    #[test]
    fn test_parse_missing_eol_semi_colon() {
        let input: &str = r#"
            listen 443
            route /api {
                upstream http://localhost:3000;
            }
        "#;

        let config: Result<Config, ParseError> = parse_proxy_config(input);

        assert!(config.is_err());
        assert!(matches!(config, Err(ParseError::MissingSemicolon { .. })));
    }

    #[test]
    fn test_parse_whitespace_padding_is_sanitised_and_ignored() {
        let input: &str = r#"
            listen                    8080       ;
            route    /api    {
                upstream    http://localhost:3000;
            }
        "#;

        let config: Config = parse_proxy_config(input).unwrap();

        assert_eq!(config.listen_port, 8080);
        assert_eq!(config.routes.len(), 1);
        assert_eq!(config.routes[0].request_endpoint, "/api");
        assert_eq!(config.routes[0].forward_endpoint, "http://localhost:3000");
    }

    #[test]
    fn test_parse_empty_config_file() {
        let input: &str = "";
        let config: Result<Config, ParseError> = parse_proxy_config(input);

        assert!(config.is_err());
        assert!(matches!(config, Err(ParseError::NoListenDirective)));
    }

    #[test]
    fn test_parse_whitespace_only_config() {
        let input: &str = r#"



        "#;

        let config: Result<Config, ParseError> = parse_proxy_config(input);

        assert!(config.is_err());
        assert!(matches!(config, Err(ParseError::NoListenDirective)));
    }

    #[test]
    fn test_parse_invalid_url_format_in_route() {
        let input: &str = r#"
            listen 8080;
            route not-a-url {
                upstream http://localhost:3000;
            }
        "#;

        let config: Result<Config, ParseError> = parse_proxy_config(input);

        assert!(config.is_err());
        assert!(matches!(config, Err(ParseError::InvalidUrlFormat { .. })));
    }

    #[test]
    fn test_parse_directive_case_sensitivity() {
        let input: &str = r#"
            LISTEN 8080;
            route /api http://localhost:3000;
        "#;

        let config: Result<Config, ParseError> = parse_proxy_config(input);

        assert!(config.is_err());
        assert!(matches!(
            config,
            Err(ParseError::InvalidDirectiveCase { .. })
        ));
    }

    #[test]
    fn test_parse_unknown_directives_in_config() {
        let input: &str = r#"
            listen 8080;
            foo bar baz;
            route /api http://localhost:3000;
        "#;

        let config: Result<Config, ParseError> = parse_proxy_config(input);

        assert!(config.is_err());
        assert!(matches!(config, Err(ParseError::UnknownDirective { .. })));
    }

    #[test]
    fn test_trailing_garbage_in_directive_line() {
        let input: &str = r#"
            listen 8080; garbage
            route /api http://localhost:3000;
        "#;

        let result: Result<Config, ParseError> = parse_proxy_config(input);

        assert!(result.is_err());
    }

    #[test]
    fn test_parse_valid_route_with_cert() {
        let input: &str = r#"
            listen 443;
            workers 2;

            route https://dashboard.asahi.tailbce682.ts.net/ {
                upstream http://localhost:3000/;
                cert /var/lib/tailscale/certs/dashboard.crt;
                key /var/lib/tailscale/certs/dashboard.key;
            }
        "#;

        let config = parse_proxy_config(input).unwrap();

        assert_eq!(config.listen_port, 443);
        assert_eq!(config.workers, 2);
        assert_eq!(config.routes.len(), 1);
        assert_eq!(
            config.routes[0].request_endpoint,
            "https://dashboard.asahi.tailbce682.ts.net/"
        );
        assert_eq!(
            config.routes[0].cert_path.as_deref(),
            Some("/var/lib/tailscale/certs/dashboard.crt")
        );
        assert_eq!(
            config.routes[0].key_path.as_deref(),
            Some("/var/lib/tailscale/certs/dashboard.key")
        );
    }

    #[test]
    fn test_parse_incomplete_route_cert_block() {
        let input: &str = r#"
            listen 443;
            route https://dashboard.asahi.tailbce682.ts.net/ {
                upstream http://localhost:3000/;
                cert /var/lib/tailscale/certs/dashboard.crt;
            }
        "#;

        let config = parse_proxy_config(input);

        assert_eq!(
            config.unwrap_err(),
            ParseError::IncompleteRouteCertBlock {
                endpoint: "https://dashboard.asahi.tailbce682.ts.net/".to_string()
            }
        );
    }

    #[test]
    fn test_parse_cert_not_allowed_for_http_route() {
        let input: &str = r#"
            listen 8080;
            route http://dashboard.local/ {
                upstream http://localhost:3000/;
                cert /var/lib/tailscale/certs/dashboard.crt;
                key /var/lib/tailscale/certs/dashboard.key;
            }
        "#;

        let config = parse_proxy_config(input);

        assert_eq!(
            config.unwrap_err(),
            ParseError::CertNotAllowedForHttpRoute {
                endpoint: "http://dashboard.local/".to_string()
            }
        );
    }

    #[test]
    fn test_parse_config_with_single_line_comments() {
        let input: &str = r#"
            // Set the listening port
            listen 8080; // This is the standard port
            
            // Define routes
            route /api {
                upstream http://localhost:3000;
            }
        "#;

        let config = parse_proxy_config(input).unwrap();
        assert_eq!(config.listen_port, 8080);
        assert_eq!(config.routes.len(), 1);
    }

    #[test]
    fn test_parse_config_with_multi_line_comments() {
        let input: &str = r#"
            /*
             * Multi-line configuration comment.
             * It should be completely stripped by the preprocessor.
             */
            listen 8080;
            
            route /api {
                upstream http://localhost:3000;
            }
        "#;

        let config = parse_proxy_config(input).unwrap();
        assert_eq!(config.listen_port, 8080);
        assert_eq!(config.routes.len(), 1);
    }

    #[test]
    fn test_parse_comments_with_edge_cases() {
        let input: &str = r#"
            listen 8080;//no-space-comment
            
            // Verify path and url slash edge cases:
            // 1. :// is not a comment:
            route http://example.com/ {
                upstream http://localhost:3000;
            }
            
            // 2. double slashes in paths is not a comment:
            logfile /var/log//proxy.log;
        "#;

        let config = parse_proxy_config(input).unwrap();
        assert_eq!(config.listen_port, 8080);
        assert_eq!(config.routes.len(), 1);
        assert_eq!(config.routes[0].request_endpoint, "http://example.com/");
        assert_eq!(config.logfile, Some("/var/log//proxy.log".to_string()));
    }

    #[test]
    fn test_parse_security_block_valid() {
        let input = r#"
            listen 8080;
            route /api {
                upstream http://localhost:3000;
            }
            security {
                proxy_protocol on;
                trusted_upstream 10.0.0.1;
                timeout 200;
            }
        "#;
        let config = parse_proxy_config(input).unwrap();
        let sec = config.security.unwrap();
        assert!(sec.proxy_protocol);
        assert_eq!(
            sec.trusted_upstream,
            "10.0.0.1".parse::<std::net::IpAddr>().unwrap()
        );
        assert_eq!(sec.timeout_ms, 200);
        assert_eq!(sec.max_tls_failures, 5);
        assert_eq!(sec.ban_duration_sec, 3600);
    }

    #[test]
    fn test_parse_security_block_custom_throttling_and_ban() {
        let input = r#"
            listen 8080;
            route /api {
                upstream http://localhost:3000;
            }
            security {
                max_tls_failures 3;
                ban_duration 1800;
            }
        "#;
        let config = parse_proxy_config(input).unwrap();
        let sec = config.security.unwrap();
        assert_eq!(sec.max_tls_failures, 3);
        assert_eq!(sec.ban_duration_sec, 1800);
    }

    #[test]
    fn test_parse_security_block_default_timeout() {
        let input = r#"
            listen 8080;
            route /api {
                upstream http://localhost:3000;
            }
            security {
                proxy_protocol off;
            }
        "#;
        let config = parse_proxy_config(input).unwrap();
        let sec = config.security.unwrap();
        assert!(!sec.proxy_protocol);
        assert_eq!(sec.timeout_ms, 200); // defaults to 200
    }

    #[test]
    fn test_parse_security_block_missing_trusted_upstream_when_on() {
        let input = r#"
            listen 8080;
            route /api {
                upstream http://localhost:3000;
            }
            security {
                proxy_protocol on;
            }
        "#;
        let config = parse_proxy_config(input);
        assert_eq!(
            config.unwrap_err(),
            ParseError::MissingSecurityDirective {
                directive: "trusted_upstream".to_string()
            }
        );
    }

    #[test]
    fn test_parse_example_config_file() {
        let contents = std::fs::read_to_string("proxy.conf.example").unwrap();
        let config = parse_proxy_config(&contents).unwrap();
        assert_eq!(config.listen_port, 443);
        assert_eq!(config.workers, 2);
        let sec = config.security.unwrap();
        assert!(!sec.proxy_protocol);
        assert_eq!(sec.max_tls_failures, 5);
        assert_eq!(sec.ban_duration_sec, 3600);
        assert_eq!(sec.rate_limit_rpm, 300);
    }

    #[test]
    fn test_parse_route_block_and_forward_auth() {
        let input = r#"
            listen 8080;
            security {
                proxy_protocol off;
                forward_auth http://localhost:9091/api/verify;
            }
            route https://grafana.example.com/ {
                upstream http://localhost:3002/;
                auth on;
                cert /etc/ssl/grafana/cert.pem;
                key /etc/ssl/grafana/key.pem;
            }
            route http://public.example.com/ {
                upstream http://localhost:4000/;
                auth off;
            }
        "#;
        let config = parse_proxy_config(input).unwrap();
        let sec = config.security.unwrap();
        assert_eq!(
            sec.forward_auth.as_deref(),
            Some("http://localhost:9091/api/verify")
        );

        assert_eq!(config.routes.len(), 2);
        assert_eq!(
            config.routes[0].request_endpoint,
            "https://grafana.example.com/"
        );
        assert_eq!(config.routes[0].forward_endpoint, "http://localhost:3002/");
        assert!(config.routes[0].auth_required);

        assert_eq!(
            config.routes[1].request_endpoint,
            "http://public.example.com/"
        );
        assert_eq!(config.routes[1].forward_endpoint, "http://localhost:4000/");
        assert!(!config.routes[1].auth_required);
    }

    #[test]
    fn test_parse_security_block_max_body_and_header_size() {
        let input = r#"
            listen 8080;
            route http://api.example.com/ {
                upstream http://localhost:3000/;
            }
            security {
                max_body_size 10M;
                max_header_size 64K;
            }
        "#;
        let config = parse_proxy_config(input).unwrap();
        let sec = config.security.unwrap();
        assert_eq!(sec.max_body_size, 10 * 1024 * 1024);
        assert_eq!(sec.max_header_size, 64 * 1024);
    }

    #[test]
    fn test_parse_intercept_errors_directive() {
        let input = r#"
            listen 8080;
            intercept_errors on;
            route http://api.example.com/ {
                upstream http://localhost:3000/;
                intercept_errors off;
            }
            route http://app.example.com/ {
                upstream http://localhost:4000/;
            }
        "#;
        let config = parse_proxy_config(input).unwrap();
        assert_eq!(config.intercept_errors, Some(true));
        assert_eq!(config.routes[0].intercept_errors, Some(false));
        assert!(!config.routes[0].should_intercept_errors(true));
        assert_eq!(config.routes[1].intercept_errors, None);
        assert!(config.routes[1].should_intercept_errors(true));
    }

    #[test]
    fn test_parse_size_string_units() {
        assert_eq!(parse_size_string("1024", "test").unwrap(), 1024);
        assert_eq!(parse_size_string("10k", "test").unwrap(), 10240);
        assert_eq!(parse_size_string("5M", "test").unwrap(), 5 * 1024 * 1024);
        assert_eq!(parse_size_string("1G", "test").unwrap(), 1024 * 1024 * 1024);
    }
}
