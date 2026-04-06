use crate::config::{CertConfig, Config, Route};
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

fn parse_route(line: &str) -> Result<Route, ParseError> {
    let line_without_semicolon = line.trim_end_matches(';').trim();
    let parts: Vec<&str> = line_without_semicolon.split_whitespace().collect();
    if parts.len() != 3 {
        return Err(ParseError::InvalidRouteDirective {
            value: line.to_string(),
        });
    }

    let request_endpoint = parts[1].to_string();
    let forward_endpoint = parts[2].to_string();

    if !is_valid_url(&request_endpoint) {
        return Err(ParseError::InvalidUrlFormat {
            value: request_endpoint,
        });
    }
    if !is_valid_url(&forward_endpoint) {
        return Err(ParseError::InvalidUrlFormat {
            value: forward_endpoint,
        });
    }

    Ok(Route {
        request_endpoint,
        forward_endpoint,
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
        "listen" | "route" | "workers" | "cert" => Ok(()),
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

fn parse_cert_block_header(line: &str) -> Result<String, ParseError> {
    let parts: Vec<&str> = line.split_whitespace().collect();
    if parts.len() != 3 || parts[0] != "cert" || parts[2] != "{" {
        return Err(ParseError::InvalidCertBlock {
            value: line.to_string(),
        });
    }

    Ok(parts[1].to_string())
}

fn parse_cert_block(
    lines: &[&str],
    index: &mut usize,
    hostname: String,
) -> Result<CertConfig, ParseError> {
    let mut cert_path: Option<String> = None;
    let mut key_path: Option<String> = None;

    *index += 1;

    while *index < lines.len() {
        let line = lines[*index];

        if line == "}" || line == "};" {
            return match (cert_path, key_path) {
                (Some(cert_path), Some(key_path)) => Ok(CertConfig {
                    hostname,
                    cert_path,
                    key_path,
                }),
                _ => Err(ParseError::IncompleteCertBlock { hostname }),
            };
        }

        if line.ends_with('{') {
            return Err(ParseError::InvalidCertBlock {
                value: line.to_string(),
            });
        }

        validate_semicolon(line)?;
        let directive = get_directive(line)?;
        validate_directive_case(directive)?;
        check_trailing_garbage(line)?;

        match directive {
            "cert" => {
                let value = parse_single_value_directive(line, "cert")?;
                if cert_path.is_some() {
                    return Err(ParseError::InvalidCertBlock {
                        value: line.to_string(),
                    });
                }
                cert_path = Some(value.to_string());
            }
            "key" => {
                let value = parse_single_value_directive(line, "key")?;
                if key_path.is_some() {
                    return Err(ParseError::InvalidCertBlock {
                        value: line.to_string(),
                    });
                }
                key_path = Some(value.to_string());
            }
            _ => {
                return Err(ParseError::InvalidCertBlock {
                    value: line.to_string(),
                });
            }
        }

        *index += 1;
    }

    Err(ParseError::UnterminatedCertBlock { hostname })
}

pub fn parse_proxy_config(input: &str) -> Result<Config, ParseError> {
    let lines: Vec<&str> = input
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .collect();

    let mut listen_port: u16 = 0;
    let mut listen_found = false;
    let mut routes = Vec::new();
    let mut routes_found = false;
    let mut request_endpoints = HashSet::new();
    let mut certs = Vec::new();
    let mut cert_hostnames = HashSet::new();
    let mut workers: Option<usize> = None;

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
            "cert" if line.ends_with('{') => {
                let hostname = parse_cert_block_header(line)?;

                if cert_hostnames.contains(&hostname) {
                    return Err(ParseError::DuplicateCertHostname { value: hostname });
                }

                let cert = parse_cert_block(&lines, &mut index, hostname.clone())?;
                cert_hostnames.insert(hostname);
                certs.push(cert);
            }
            "cert" => {
                return Err(ParseError::InvalidCertBlock {
                    value: line.to_string(),
                });
            }
            "listen" => {
                validate_semicolon(line)?;
                check_trailing_garbage(line)?;

                if listen_found {
                    return Err(ParseError::TooManyListenDirectives);
                }
                listen_port = parse_listen_line(line)?;
                listen_found = true;
            }
            "route" => {
                validate_semicolon(line)?;
                check_trailing_garbage(line)?;

                let route = parse_route(line)?;
                if request_endpoints.contains(&route.request_endpoint) {
                    return Err(ParseError::DuplicateRequestEndpoint {
                        value: route.request_endpoint.clone(),
                    });
                }

                request_endpoints.insert(route.request_endpoint.clone());
                routes.push(route);
                routes_found = true;
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

    let workers = workers.unwrap_or_else(|| {
        std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1)
    });

    Ok(Config {
        listen_port,
        routes,
        certs,
        workers,
    })
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
            route https://dashboard.myserver.home/api http://localhost:3000;
            "#;

        let config: Config = parse_proxy_config(input).unwrap();

        assert_eq!(config.listen_port, 8080);
        assert_eq!(config.routes.len(), 1);
        assert_eq!(
            config.routes[0].request_endpoint,
            "https://dashboard.myserver.home/api"
        );
        assert_eq!(config.routes[0].forward_endpoint, "http://localhost:3000");
    }

    #[test]
    fn test_parse_config_with_multiple_routes() {
        let input: &str = r#"
            listen 8080;
            route /api http://localhost:3000;
            route /auth http://localhost:4000;
        "#;

        let result: Config = parse_proxy_config(input).unwrap();

        assert_eq!(result.routes.len(), 2);
    }

    #[test]
    fn test_parse_listen_port_line_too_many_arguments() {
        let input: &str = r#"
            listen 8080 443;
            route https://dashboard.myserver.local/api http://localhost:3000;
            "#;

        let config: Result<Config, ParseError> = parse_proxy_config(input);

        assert!(config.is_err());
        assert!(matches!(config, Err(ParseError::InvalidListenDirective)));
    }

    #[test]
    fn test_parse_listen_port_not_valid_u16_type() {
        let cases = vec!["abc", "-1", "70000"];

        for port in cases {
            let input = format!("listen {};\nroute /api http://localhost:3000;", port);
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
            route https://dashboard.myserver.local/api http://localhost:3000;
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
            route https://dashboard.myserver.local/api http://localhost:3000;
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
        route /api http://localhost:3000;
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
            route /api http://localhost:3000;
            route /api http://localhost:4000;
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
            route /api http://localhost:3000;
        "#;

        let config: Result<Config, ParseError> = parse_proxy_config(input);

        assert!(config.is_err());
        assert!(matches!(config, Err(ParseError::MissingSemicolon { .. })));
    }

    #[test]
    fn test_parse_whitespace_padding_is_sanitised_and_ignored() {
        let input: &str = r#"
            listen                    8080       ;
            route    /api    http://localhost:3000;
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
            route not-a-url http://localhost:3000;
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
    fn test_parse_valid_cert_blocks() {
        let input: &str = r#"
            listen 443;
            workers 2;

            cert dashboard.asahi.tailbce682.ts.net {
                cert /var/lib/tailscale/certs/dashboard.crt;
                key /var/lib/tailscale/certs/dashboard.key;
            }

            cert grafana.asahi.tailbce682.ts.net {
                cert /var/lib/tailscale/certs/grafana.crt;
                key /var/lib/tailscale/certs/grafana.key;
            }

            route https://dashboard.asahi.tailbce682.ts.net/ http://localhost:3000/;
            route https://grafana.asahi.tailbce682.ts.net/ http://localhost:3001/;
        "#;

        let config = parse_proxy_config(input).unwrap();

        assert_eq!(config.listen_port, 443);
        assert_eq!(config.workers, 2);
        assert_eq!(config.certs.len(), 2);
        assert_eq!(
            config.certs[0].hostname,
            "dashboard.asahi.tailbce682.ts.net"
        );
        assert_eq!(config.certs[1].hostname, "grafana.asahi.tailbce682.ts.net");
    }

    #[test]
    fn test_parse_duplicate_cert_hostnames() {
        let input: &str = r#"
            listen 443;
            cert dashboard.asahi.tailbce682.ts.net {
                cert /var/lib/tailscale/certs/dashboard.crt;
                key /var/lib/tailscale/certs/dashboard.key;
            }
            cert dashboard.asahi.tailbce682.ts.net {
                cert /var/lib/tailscale/certs/dashboard-2.crt;
                key /var/lib/tailscale/certs/dashboard-2.key;
            }
            route https://dashboard.asahi.tailbce682.ts.net/ http://localhost:3000/;
        "#;

        let config = parse_proxy_config(input);

        assert_eq!(
            config.unwrap_err(),
            ParseError::DuplicateCertHostname {
                value: "dashboard.asahi.tailbce682.ts.net".to_string()
            }
        );
    }

    #[test]
    fn test_parse_incomplete_cert_block() {
        let input: &str = r#"
            listen 443;
            cert dashboard.asahi.tailbce682.ts.net {
                cert /var/lib/tailscale/certs/dashboard.crt;
            }
            route https://dashboard.asahi.tailbce682.ts.net/ http://localhost:3000/;
        "#;

        let config = parse_proxy_config(input);

        assert_eq!(
            config.unwrap_err(),
            ParseError::IncompleteCertBlock {
                hostname: "dashboard.asahi.tailbce682.ts.net".to_string()
            }
        );
    }

    #[test]
    fn test_parse_unterminated_cert_block() {
        let input: &str = r#"
            listen 443;
            cert dashboard.asahi.tailbce682.ts.net {
                cert /var/lib/tailscale/certs/dashboard.crt;
                key /var/lib/tailscale/certs/dashboard.key;
        "#;

        let config = parse_proxy_config(input);

        assert_eq!(
            config.unwrap_err(),
            ParseError::UnterminatedCertBlock {
                hostname: "dashboard.asahi.tailbce682.ts.net".to_string()
            }
        );
    }

    #[test]
    fn test_parse_invalid_directive_inside_cert_block() {
        let input: &str = r#"
            listen 443;
            cert dashboard.asahi.tailbce682.ts.net {
                cert /var/lib/tailscale/certs/dashboard.crt;
                key /var/lib/tailscale/certs/dashboard.key;
                route https://dashboard.asahi.tailbce682.ts.net/ http://localhost:3000/;
            }
            route https://dashboard.asahi.tailbce682.ts.net/ http://localhost:3000/;
        "#;

        let config = parse_proxy_config(input);

        assert_eq!(
            config.unwrap_err(),
            ParseError::InvalidCertBlock {
                value: "route https://dashboard.asahi.tailbce682.ts.net/ http://localhost:3000/;"
                    .to_string()
            }
        );
    }
}
