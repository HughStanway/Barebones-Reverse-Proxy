use crate::config::{Config, Route};
use crate::error::ParseError;
use std::collections::HashSet;

fn is_valid_url(url: &str) -> bool {
    // Basic URL validation: must contain :// or start with /
    url.contains("://") || url.starts_with('/') || url.contains(':')
}

fn parse_listen_line(line: &str) -> Result<u16, ParseError> {
    // Remove semicolon from the line before parsing
    let line_without_semicolon = line.trim_end_matches(';').trim();
    let parts: Vec<&str> = line_without_semicolon.split_whitespace().collect();
    if parts.len() != 2 {
        return Err(ParseError::InvalidListenDirective);
    }

    let port: u16 = parts[1]
        .parse::<u16>()
        .map_err(|_| ParseError::InvalidPort {
            value: parts[1].to_string(),
        })?;

    Ok(port)
}

fn parse_route(line: &str) -> Result<Route, ParseError> {
    // Remove semicolon from the line before parsing
    let line_without_semicolon = line.trim_end_matches(';').trim();
    let parts: Vec<&str> = line_without_semicolon.split_whitespace().collect();
    if parts.len() != 3 {
        return Err(ParseError::InvalidRouteDirective {
            value: line.to_string(),
        });
    }

    let request_endpoint: String = parts[1].to_string();
    let forward_endpoint: String = parts[2].to_string();

    // Validate URL formats
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

fn validate_known_directive(directive: &str) -> Result<(), ParseError> {
    match directive {
        "listen" | "route" => Ok(()),
        _ => Err(ParseError::UnknownDirective {
            directive: directive.to_string(),
        }),
    }
}

fn check_trailing_garbage(line: &str) -> Result<(), ParseError> {
    let trimmed = line.trim();
    if !trimmed.ends_with(';') {
        return Ok(());
    }

    // Check if there's anything after the semicolon
    if let Some(semicolon_pos) = trimmed.rfind(';') {
        let after_semicolon = trimmed[semicolon_pos + 1..].trim();
        if !after_semicolon.is_empty() {
            return Err(ParseError::InvalidListenDirective);
        }
    }

    Ok(())
}

pub fn parse_proxy_config(input: &str) -> Result<Config, ParseError> {
    let mut listen_port: u16 = 0;
    let mut listen_found: bool = false;
    let mut routes: Vec<Route> = Vec::new();
    let mut routes_found: bool = false;
    let mut request_endpoints: HashSet<String> = HashSet::new();

    for line in input.lines() {
        let line: &str = line.trim();

        if line.is_empty() {
            continue;
        }

        // Validate semicolon termination
        validate_semicolon(line)?;

        // Get directive and validate case
        let directive = get_directive(line)?;
        validate_directive_case(directive)?;
        validate_known_directive(directive)?;
        check_trailing_garbage(line)?;

        if directive == "listen" {
            if listen_found {
                return Err(ParseError::TooManyListenDirectives);
            }
            listen_port = parse_listen_line(line)?;
            listen_found = true;
        }

        if directive == "route" {
            let route = parse_route(line)?;

            // Check for duplicate request endpoints
            if request_endpoints.contains(&route.request_endpoint) {
                return Err(ParseError::DuplicateRequestEndpoint {
                    value: route.request_endpoint.clone(),
                });
            }

            request_endpoints.insert(route.request_endpoint.clone());
            routes.push(route);
            routes_found = true;
        }
    }

    if !listen_found {
        return Err(ParseError::NoListenDirective);
    }

    if !routes_found {
        return Err(ParseError::NoRouteDirective);
    }

    Ok(Config {
        listen_port,
        routes,
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
        // GIVEN
        let input: &str = r#"
            listen 8080;
            route https://dashboard.myserver.home/api http://localhost:3000;
            "#;

        // WHEN
        let config: Config = parse_proxy_config(input).unwrap();

        // THEN
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
        // GIVEN
        let input: &str = r#"
            listen 8080;
            route /api http://localhost:3000;
            route /auth http://localhost:4000;
        "#;

        // WHEN
        let result: Config = parse_proxy_config(input).unwrap();

        // THEN
        assert_eq!(result.routes.len(), 2);
    }

    #[test]
    fn test_parse_listen_port_line_too_many_arguments() {
        // GIVEN
        let input: &str = r#"
            listen 8080 443;
            route https://dashboard.myserver.local/api http://localhost:3000;
            "#;

        // WHEN
        let config: Result<Config, ParseError> = parse_proxy_config(input);

        // THEN
        assert!(config.is_err());
        assert!(matches!(config, Err(ParseError::InvalidListenDirective)));
    }

    #[test]
    fn test_parse_listen_port_not_valid_u16_type() {
        let cases = vec!["abc", "-1", "70000"];

        for port in cases {
            // GIVEN
            let input = format!("listen {};\nroute /api http://localhost:3000;", port);

            // WHEN
            let config: Result<Config, ParseError> = parse_proxy_config(&input);

            // THEN
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
        // GIVEN
        let input: &str = r#"
            route https://dashboard.myserver.local/api http://localhost:3000;
            "#;

        // WHEN
        let config: Result<Config, ParseError> = parse_proxy_config(input);

        // THEN
        assert!(config.is_err());
        assert!(matches!(config, Err(ParseError::NoListenDirective)));
    }

    #[test]
    fn test_parse_multiple_listen_directives_given() {
        // GIVEN
        let input: &str = r#"
            listen 8080;
            listen 443;
            route https://dashboard.myserver.local/api http://localhost:3000;
            "#;

        // WHEN
        let config: Result<Config, ParseError> = parse_proxy_config(input);

        // THEN
        assert!(config.is_err());
        assert!(matches!(config, Err(ParseError::TooManyListenDirectives)));
    }

    #[test]
    fn test_parse_route_line_too_many_arguments() {
        // GIVEN
        let input: &str = r#"
            listen 8080;
            route https://dashboard.myserver.local/api http://localhost:3000 http://localhost:3001;
            "#;

        // WHEN
        let config: Result<Config, ParseError> = parse_proxy_config(input);

        // THEN
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
        // GIVEN
        let input: &str = r#"
            listen 8080;
            route /api;
        "#;

        // WHEN
        let config: Result<Config, ParseError> = parse_proxy_config(input);

        // THEN
        assert_eq!(
            config.unwrap_err(),
            ParseError::InvalidRouteDirective {
                value: "route /api;".to_string()
            }
        );
    }

    #[test]
    fn test_parse_no_route_directives_given() {
        // GIVEN
        let input: &str = r#"
            listen 8080;
            "#;

        // WHEN
        let config: Result<Config, ParseError> = parse_proxy_config(input);

        // THEN
        assert!(config.is_err());
        assert!(matches!(config, Err(ParseError::NoRouteDirective)));
    }

    #[test]
    fn test_partial_invalid_config() {
        // GIVEN
        let input: &str = r#"
        listen 8080;
        route /api http://localhost:3000;
        route invalid;
    "#;

        // WHEN
        let config: Result<Config, ParseError> = parse_proxy_config(input);

        // THEN
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
        // GIVEN: Two routes with the same request endpoint
        let input: &str = r#"
            listen 8080;
            route /api http://localhost:3000;
            route /api http://localhost:4000;
        "#;

        // WHEN
        let config: Result<Config, ParseError> = parse_proxy_config(input);

        // THEN: Parser should reject duplicate request endpoints
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
        // GIVEN: A line without proper ';' termination
        let input: &str = r#"
            listen 443
            route /api http://localhost:3000;
        "#;

        // WHEN
        let config: Result<Config, ParseError> = parse_proxy_config(input);

        // THEN: All lines should have proper ';' termination
        assert!(config.is_err());
        assert!(matches!(config, Err(ParseError::MissingSemicolon { .. })));
    }

    #[test]
    fn test_parse_whitespace_padding_is_sanitised_and_ignored() {
        // GIVEN: Config with excessive whitespace
        let input: &str = r#"
            listen                    8080       ;
            route    /api    http://localhost:3000;
        "#;

        // WHEN
        let config: Config = parse_proxy_config(input).unwrap();

        // THEN: All whitespace is sanitised and ignored
        assert_eq!(config.listen_port, 8080);
        assert_eq!(config.routes.len(), 1);
        assert_eq!(config.routes[0].request_endpoint, "/api");
        assert_eq!(config.routes[0].forward_endpoint, "http://localhost:3000");
    }

    #[test]
    fn test_parse_empty_config_file() {
        // GIVEN: An empty config file
        let input: &str = "";

        // WHEN
        let config: Result<Config, ParseError> = parse_proxy_config(input);

        // THEN: Should fail - not a valid config (needs listen port and at least one route)
        assert!(config.is_err());
        assert!(matches!(config, Err(ParseError::NoListenDirective)));
    }

    #[test]
    fn test_parse_whitespace_only_config() {
        // GIVEN: Config with only whitespace
        let input: &str = r#"
            
            
            
        "#;

        // WHEN
        let config: Result<Config, ParseError> = parse_proxy_config(input);

        // THEN: Should fail - not a valid config (needs listen port and at least one route)
        assert!(config.is_err());
        assert!(matches!(config, Err(ParseError::NoListenDirective)));
    }

    #[test]
    fn test_parse_invalid_url_format_in_route() {
        // GIVEN: Route with invalid URL format (no protocol or path indicator)
        let input: &str = r#"
            listen 8080;
            route not-a-url http://localhost:3000;
        "#;

        // WHEN
        let config: Result<Config, ParseError> = parse_proxy_config(input);

        // THEN: Parser should check and validate URL formats
        assert!(config.is_err());
        assert!(matches!(config, Err(ParseError::InvalidUrlFormat { .. })));
    }

    #[test]
    fn test_parse_directive_case_sensitivity() {
        // GIVEN: Config with uppercase directive
        let input: &str = r#"
            LISTEN 8080;
            route /api http://localhost:3000;
        "#;

        // WHEN
        let config: Result<Config, ParseError> = parse_proxy_config(input);

        // THEN: All directives should be lowercase
        assert!(config.is_err());
        assert!(matches!(
            config,
            Err(ParseError::InvalidDirectiveCase { .. })
        ));
    }

    #[test]
    fn test_parse_unknown_directives_in_config() {
        // GIVEN: Config with unknown directive
        let input: &str = r#"
            listen 8080;
            foo bar baz;
            route /api http://localhost:3000;
        "#;

        // WHEN
        let config: Result<Config, ParseError> = parse_proxy_config(input);

        // THEN: Only valid directives are 'listen' and 'route'
        assert!(config.is_err());
        assert!(matches!(config, Err(ParseError::UnknownDirective { .. })));
    }

    #[test]
    fn test_trailing_garbage_in_directive_line() {
        // GIVEN
        let input: &str = r#"
            listen 8080; garbage
            route /api http://localhost:3000;
        "#;

        // WHEN
        let result: Result<Config, ParseError> = parse_proxy_config(input);

        // THEN
        assert!(result.is_err());
    }
}
