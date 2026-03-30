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
}
