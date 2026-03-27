use std::fmt;

/// Source location in **characters** (matches lexer indexing). Line and column are 1-based.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub line: usize,
    pub column: usize,
    pub len: usize,
    /// Originating source file path (when available).
    ///
    /// This is primarily used for printing caret/snippets for errors that originate in imported modules.
    pub file: Option<&'static str>,
}

impl Span {
    pub const fn new(line: usize, column: usize, len: usize) -> Self {
        Self {
            line,
            column,
            len,
            file: None,
        }
    }

    pub fn with_file(self, file: &'static str) -> Self {
        Self {
            file: Some(file),
            ..self
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LexError {
    pub message: String,
    pub span: Span,
}

impl LexError {
    pub fn new(message: impl Into<String>, span: Span) -> Self {
        Self {
            message: message.into(),
            span,
        }
    }

    pub fn format_with_file(&self, path: &str) -> String {
        format!(
            "{}:{}:{}: lexer error: {}",
            path, self.span.line, self.span.column, self.message
        )
    }
}

impl fmt::Display for LexError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}:{}: lexer error: {}",
            self.span.line, self.span.column, self.message
        )
    }
}

/// Parse failures. Some variants are reserved for future grammar checks.
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseError {
    UnexpectedToken {
        message: String,
        span: Option<Span>,
    },
    UnexpectedEof {
        expected: &'static str,
    },
    Message(String),
}

impl ParseError {
    pub fn format_with_file(&self, path: &str) -> String {
        match self {
            ParseError::UnexpectedToken { message, span: Some(s) } => format!(
                "{}:{}:{}: parse error: {}",
                path, s.line, s.column, message
            ),
            ParseError::UnexpectedToken { message, span: None } => {
                format!("{}: parse error: {}", path, message)
            }
            ParseError::UnexpectedEof { expected } => format!(
                "{}: parse error: unexpected end of file, expected {}",
                path, expected
            ),
            ParseError::Message(msg) => format!("{}: parse error: {}", path, msg),
        }
    }
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ParseError::UnexpectedToken { message, span: Some(s) } => {
                write!(f, "{}:{}: parse error: {}", s.line, s.column, message)
            }
            ParseError::UnexpectedToken { message, span: None } => {
                write!(f, "parse error: {}", message)
            }
            ParseError::UnexpectedEof { expected } => write!(
                f,
                "parse error: unexpected end of file, expected {}",
                expected
            ),
            ParseError::Message(msg) => write!(f, "parse error: {}", msg),
        }
    }
}

/// Semantic / type-check diagnostic (always has a source [`Span`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemanticError {
    pub message: String,
    pub span: Span,
}

impl SemanticError {
    pub fn new(message: impl Into<String>, span: Span) -> Self {
        Self {
            message: message.into(),
            span,
        }
    }

    pub fn format_with_file(&self, path: &str) -> String {
        format!(
            "{}:{}:{}: semantic error: {}",
            path, self.span.line, self.span.column, self.message
        )
    }
}

impl fmt::Display for SemanticError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}:{}: semantic error: {}",
            self.span.line, self.span.column, self.message
        )
    }
}

/// Semantic warning diagnostic (non-fatal, includes a source [`Span`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemanticWarning {
    pub message: String,
    pub span: Span,
}

impl SemanticWarning {
    pub fn new(message: impl Into<String>, span: Span) -> Self {
        Self {
            message: message.into(),
            span,
        }
    }

    pub fn format_with_file(&self, path: &str) -> String {
        format!(
            "{}:{}:{}: semantic warning: {}",
            path, self.span.line, self.span.column, self.message
        )
    }
}

impl fmt::Display for SemanticWarning {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}:{}: semantic warning: {}",
            self.span.line, self.span.column, self.message
        )
    }
}
