#[derive(Debug, PartialEq)]
pub enum ParseError {
    NoListenDirective,
    InvalidListenDirective,
    TooManyListenDirectives,
    InvalidPort { value: String },
    NoRouteDirective,
    InvalidRouteDirective { value: String },
    DuplicateRequestEndpoint { value: String },
    MissingSemicolon { line: String },
    InvalidUrlFormat { value: String },
    InvalidDirectiveCase { directive: String },
    UnknownDirective { directive: String },
    IncompleteTlsConfig,
    InvalidWorkersValue { value: String },
    TooManyWorkersDirectives,
}

#[derive(Debug)]
pub enum ProxyError {
    IoError(std::io::Error),
    TlsError(String),
    NoMatchingRoute,
    UpstreamConnectionFailed(String),
}

impl std::fmt::Display for ProxyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProxyError::IoError(e) => write!(f, "IO error: {}", e),
            ProxyError::TlsError(e) => write!(f, "TLS error: {}", e),
            ProxyError::NoMatchingRoute => write!(f, "No matching route"),
            ProxyError::UpstreamConnectionFailed(addr) => {
                write!(f, "Failed to connect to upstream: {}", addr)
            }
        }
    }
}

impl std::error::Error for ProxyError {}

impl From<std::io::Error> for ProxyError {
    fn from(e: std::io::Error) -> Self {
        ProxyError::IoError(e)
    }
}
