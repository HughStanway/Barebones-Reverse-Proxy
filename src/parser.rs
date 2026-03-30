use crate::config::{Config, Route};

#[derive(Debug)]
pub enum ParseError {
    NoListenDirective,
    InvalidListenDirective,
    InvalidPort,
    TooManyListenDirectives,
    NoRouteDirective,
    InvalidRouteDirective,
}

impl From<ParseError> for String {
    fn from(error: ParseError) -> Self {
        match error {
            ParseError::NoListenDirective => "No listen directive".into(),
            ParseError::InvalidListenDirective => "Invalid listen directive".into(),
            ParseError::InvalidPort => "Invalid port".into(),
            ParseError::TooManyListenDirectives => "Too many listen directive".into(),
            ParseError::NoRouteDirective => "No route directive".into(),
            ParseError::InvalidRouteDirective => "Invalid route directive".into(),
        }
    }
}

fn parse_listen_line(line: &str) -> Result<u16, ParseError> {
    let parts: Vec<&str> = line.split_whitespace().collect();
    if parts.len() != 2 {
        return Err(ParseError::InvalidListenDirective);
    }

    let port: u16 = parts[1].trim_end_matches(';')
        .parse::<u16>()
        .map_err(|_| ParseError::InvalidPort)?;

    Ok(port)
}

fn parse_route(line: &str) -> Result<Route, ParseError> {
    let parts: Vec<&str> = line.split_whitespace().collect();
    if parts.len() != 3 {
        return Err(ParseError::InvalidRouteDirective);
    }

    let request_endpoint: String = parts[1].to_string();
    let forward_endpoint: String = parts[2].trim_end_matches(";").to_string();

    Ok(Route{
        request_endpoint: request_endpoint,
        forward_endpoint: forward_endpoint,
    })
}

pub fn parse_proxy_config(input: &str) -> Result<Config, ParseError> {
    let mut listen_port: u16 = 0;
    let mut listen_found: bool = false;
    let mut routes: Vec<Route> = Vec::new();
    let mut routes_found: bool = false;

    for line in input.lines() {
        let line: &str = line.trim();

        if line.is_empty() {
            continue;
        }

        if line.starts_with("listen") {
            if listen_found {
                return Err(ParseError::TooManyListenDirectives);
            }
            listen_port = parse_listen_line(line)?;
            listen_found = true;
        }

        if line.starts_with("route") {
            routes.push(parse_route(line)?);
            routes_found = true;
        }
    }

    if !listen_found {
        return Err(ParseError::NoListenDirective);
    }

    if !routes_found {
        return Err(ParseError::NoRouteDirective);
    }

    Ok(Config{
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
            route https://dashboard.myserver.home/api http://localhost:3000
            "#;
        
        // WHEN
        let config: Config = parse_proxy_config(input).unwrap();

        // THEN
        assert_eq!(config.listen_port, 8080);
        assert_eq!(config.routes.len(), 1);
        assert_eq!(config.routes[0].request_endpoint, "https://dashboard.myserver.home/api");
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
        assert_eq!(config.is_err(), true);
        assert!(matches!(config, Err(ParseError::InvalidListenDirective)));
    }

    #[test]
    fn test_parse_listen_port_not_valid_u16_type() {
        let cases = vec!["abc", "-1", "70000"];

        for port in cases {
            // GIVEN
            let input = format!(
                "listen {};\nroute /api http://localhost:3000;",
                port
            );
            
            // WHEN
            let config:Result<Config, ParseError> = parse_proxy_config(&input);

            // THEN
            assert_eq!(config.is_err(), true);
            assert!(matches!(config, Err(ParseError::InvalidPort)));
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
        assert_eq!(config.is_err(), true);
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
        assert_eq!(config.is_err(), true);
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
        let config:Result<Config, ParseError> = parse_proxy_config(&input);

        // THEN
        assert_eq!(config.is_err(), true);
        assert!(matches!(config, Err(ParseError::InvalidRouteDirective)));
    }

    #[test]
    fn test_parse_route_missing_arguments() {
        // GIVEN
        let input: &str = r#"
            listen 8080;
            route /api;
        "#;

        // WHEN
        let result: Result<Config, ParseError> = parse_proxy_config(input);
        
        // THEN
        assert!(matches!(result, Err(ParseError::InvalidRouteDirective)));
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
        assert_eq!(config.is_err(), true);
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
    let result: Result<Config, ParseError> = parse_proxy_config(input);

    // THEN
    assert!(result.is_err());
    assert!(matches!(result, Err(ParseError::InvalidRouteDirective)));
}

    fn test_parse_duplicate_request_endpoint_routes() {
        // TODO
    }

    fn test_parse_missing_EOL_semi_colon() {
        // TODO e.g. 'listen 443'
    }

    fn test_parse_whitespace_padding_is_sanitised_and_ignored() {
        // TODO e.g. 'listen                    443       ;'
    }

    fn test_parse_empty_config_file() {
        // TODO
    }

    fn test_parse_whitespace_only_config() {
        // TODO
    }

    fn test_parse_invalid_url_format_in_route() {
        // TODO e.g. not-a-url
    }

    fn test_parse_directive_case_sensitivity() {
        // TODO e.g. LISTEN or LisTEn
    }

    fn test_parse_unknown_directives_in_config() {
        // TODO e.g. foo bar baz;
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