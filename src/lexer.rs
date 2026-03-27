use crate::error::{LexError, Span};

#[derive(Debug, Clone, PartialEq)]
pub enum TokenKind {
    SingleLineComment(String),
    MultiLineComment(String),
    IntegerLiteral {
        value: u64,
        original: String,
        radix: u32, // 2, 8, 10, or 16
    },
    FloatLiteral {
        /// Original lexeme, including underscores (e.g. `1_e_-1`).
        original: String,
        /// Cleansed lexeme for numeric parsing (underscores removed).
        cleaned: String,
    },
    /// Decoded string value and the source lexeme (including surrounding `"`).
    StringLiteral {
        value: String,
        original: String,
    },
    Identifier(String),
    /// Keyword `let`
    Let,
    /// Keyword `const`
    Const,
    /// Keyword `true` / `false`
    True,
    False,
    /// Keywords `if` / `else` / `while` / `break` / `continue`
    If,
    Else,
    While,
    Break,
    Continue,
    /// Keyword `match`
    Match,
    /// Keyword `import`
    Import,
    /// Keyword `export`
    Export,
    /// Keyword `as` (import/export rename)
    As,
    /// Keyword `from`
    From,
    /// Keyword `params`
    Params,
    /// Keyword `type` (e.g. `[type T]` in extension receiver types)
    Type,
    /// Keyword `struct`
    Struct,
    /// Keyword `async`
    Async,
    /// Keyword `await`
    Await,
    /// Keyword `enum`
    Enum,
    /// Standalone `_` (wildcard pattern, not an identifier)
    Underscore,
    LParen,
    RParen,
    LBrace,
    RBrace,
    LBracket,
    RBracket,
    Colon,
    Semicolon,
    Comma,
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    Tilde,
    /// Single `=` (assignment / initializer)
    Eq,
    EqEq,
    /// Fat arrow `=>` (shorthand function body)
    FatArrow,
    Ne,
    Lt,
    Gt,
    Le,
    Ge,
    ShiftLeft,
    ShiftRight,
    ShiftLeftEq,
    ShiftRightEq,
    AndAnd,
    AndEq,
    /// Bitwise and (`&`), distinct from `&&`
    Amp,
    OrOr,
    OrEq,
    /// Bitwise or (`|`), distinct from `||`
    Pipe,
    Caret,
    CaretEq,
    PlusEq,
    MinusEq,
    StarEq,
    SlashEq,
    PercentEq,
    Bang,
    Dot,
    DotDot,
    /// `::`
    ColonColon,
    Eof,
}

/// A lexeme with its source [`Span`] (1-based line/column, length in characters).
#[derive(Debug, Clone, PartialEq)]
pub struct Token {
    pub kind: TokenKind,
    pub span: Span,
}

/// Used when the parser cursor moves past the last token (should not happen in a well-formed run).
pub const EOF_SENTINEL: Token = Token {
    kind: TokenKind::Eof,
    span: Span::new(1, 1, 0),
};

pub struct Lexer {
    input: Vec<char>,
    position: usize,
    file: Option<&'static str>,
}

impl Lexer {
    pub fn new(input: &str) -> Self {
        Lexer {
            input: input.chars().collect(),
            position: 0,
            file: None,
        }
    }

    pub fn new_with_file(input: &str, file: &'static str) -> Self {
        Lexer {
            input: input.chars().collect(),
            position: 0,
            file: Some(file),
        }
    }

    /// 1-based line and column for a character index `index` (0 ..= len).
    fn char_index_to_line_col(&self, index: usize) -> (usize, usize) {
        let mut line = 1usize;
        let mut col = 1usize;
        for i in 0..index.min(self.input.len()) {
            if self.input[i] == '\n' {
                line += 1;
                col = 1;
            } else {
                col += 1;
            }
        }
        (line, col)
    }

    fn span_at(&self, index: usize, len: usize) -> Span {
        let (line, col) = self.char_index_to_line_col(index);
        let span = Span::new(line, col, len);
        if let Some(file) = self.file {
            span.with_file(file)
        } else {
            span
        }
    }

    pub fn tokenize(&mut self) -> Result<Vec<Token>, LexError> {
        let mut tokens = Vec::new();

        while self.position < self.input.len() {
            self.skip_whitespace();

            if self.position >= self.input.len() {
                break;
            }

            if self.peek() == '/' && self.peek_next() == Some('/') {
                tokens.push(self.read_single_line_comment()?);
            } else if self.peek() == '/' && self.peek_next() == Some('*') {
                tokens.push(self.read_multi_line_comment()?);
            } else if self.peek() == '"' {
                tokens.push(self.read_string_literal()?);
            } else if is_ident_start(self.peek()) {
                tokens.push(self.read_identifier()?);
            } else if self.peek() == '.' {
                tokens.push(self.read_dot()?);
            } else if self.peek() == '=' {
                tokens.push(self.read_eq_or_eqeq()?);
            } else if self.peek() == '!' {
                tokens.push(self.read_bang_or_ne()?);
            } else if self.peek() == '<' {
                tokens.push(self.read_lt_or_le()?);
            } else if self.peek() == '>' {
                tokens.push(self.read_gt_or_ge()?);
            } else if self.peek() == '&' {
                tokens.push(self.read_ampersand()?);
            } else if self.peek() == '|' {
                tokens.push(self.read_pipe()?);
            } else if self.peek() == '^' {
                tokens.push(self.read_caret()?);
            } else if self.peek() == '+' {
                tokens.push(self.read_plus()?);
            } else if self.peek() == '-' {
                tokens.push(self.read_minus()?);
            } else if self.peek() == '*' {
                tokens.push(self.read_star()?);
            } else if self.peek() == '/' {
                tokens.push(self.read_slash()?);
            } else if self.peek() == '%' {
                tokens.push(self.read_percent()?);
            } else if self.peek() == ':' && self.peek_next() == Some(':') {
                let start = self.position;
                self.position += 2;
                tokens.push(Token {
                    kind: TokenKind::ColonColon,
                    span: self.span_at(start, 2),
                });
            } else if matches!(self.peek(), '(' | ')' | '{' | '}' | ':' | ';' | ',' | '~') {
                tokens.push(self.read_punct()?);
            } else if matches!(self.peek(), '[' | ']') {
                tokens.push(self.read_punct()?);
            } else if self.peek().is_ascii_digit() {
                tokens.push(self.read_number_literal()?);
            } else {
                let ch = self.peek();
                let span = self.span_at(self.position, 1);
                let msg = format!("unexpected character {}", describe_char(ch));
                return Err(LexError::new(msg, span));
            }
        }

        let eof_span = self.span_at(self.input.len(), 0);
        tokens.push(Token {
            kind: TokenKind::Eof,
            span: eof_span,
        });
        Ok(tokens)
    }

    fn read_single_line_comment(&mut self) -> Result<Token, LexError> {
        let start = self.position;
        self.position += 2; // //

        let body_start = self.position;
        while self.position < self.input.len() && self.input[self.position] != '\n' {
            self.position += 1;
        }

        let comment_text = self.input[body_start..self.position].iter().collect::<String>();
        let span = self.span_at(start, self.position - start);
        Ok(Token {
            kind: TokenKind::SingleLineComment(comment_text),
            span,
        })
    }

    fn read_multi_line_comment(&mut self) -> Result<Token, LexError> {
        let open_pos = self.position;
        self.position += 2;

        let body_start = self.position;
        while self.position < self.input.len() {
            if self.peek() == '*' && self.peek_next() == Some('/') {
                let comment_text = self.input[body_start..self.position].iter().collect::<String>();
                self.position += 2; // Consume '*/'
                let span = self.span_at(open_pos, self.position - open_pos);
                return Ok(Token {
                    kind: TokenKind::MultiLineComment(comment_text),
                    span,
                });
            }
            self.position += 1;
        }

        Err(LexError::new(
            "unterminated multi-line comment",
            self.span_at(open_pos, 2),
        ))
    }

    fn read_string_literal(&mut self) -> Result<Token, LexError> {
        let start = self.position;
        self.position += 1; // opening "
        let mut value = String::new();

        while self.position < self.input.len() {
            let ch = self.peek();
            if ch == '"' {
                self.position += 1;
                let original = self.input[start..self.position].iter().collect::<String>();
                let span = self.span_at(start, self.position - start);
                return Ok(Token {
                    kind: TokenKind::StringLiteral { value, original },
                    span,
                });
            }
            if ch == '\\' {
                self.position += 1;
                if self.position >= self.input.len() {
                    return Err(LexError::new(
                        "unterminated string literal",
                        self.span_at(start, self.position - start),
                    ));
                }
                let esc = self.peek();
                self.position += 1;
                match esc {
                    'n' => value.push('\n'),
                    'r' => value.push('\r'),
                    '"' => value.push('"'),
                    '\\' => value.push('\\'),
                    _ => {
                        return Err(LexError::new(
                            format!("invalid escape sequence \\{}", esc),
                            self.span_at(self.position - 2, 2),
                        ));
                    }
                }
            } else {
                value.push(ch);
                self.position += 1;
            }
        }

        Err(LexError::new(
            "unterminated string literal",
            self.span_at(start, self.input.len() - start),
        ))
    }

    fn read_identifier(&mut self) -> Result<Token, LexError> {
        let start = self.position;
        self.position += 1;
        while self.position < self.input.len() && is_ident_continue(self.peek()) {
            self.position += 1;
        }
        let name = self.input[start..self.position].iter().collect::<String>();
        let span = self.span_at(start, self.position - start);
        let kind = match name.as_str() {
            "let" => TokenKind::Let,
            "const" => TokenKind::Const,
            "true" => TokenKind::True,
            "false" => TokenKind::False,
            "if" => TokenKind::If,
            "else" => TokenKind::Else,
            "while" => TokenKind::While,
            "break" => TokenKind::Break,
            "continue" => TokenKind::Continue,
            "struct" => TokenKind::Struct,
            "async" => TokenKind::Async,
            "await" => TokenKind::Await,
            "enum" => TokenKind::Enum,
            "match" => TokenKind::Match,
            "import" => TokenKind::Import,
            "export" => TokenKind::Export,
            "from" => TokenKind::From,
            "as" => TokenKind::As,
            "params" => TokenKind::Params,
            "type" => TokenKind::Type,
            "_" => TokenKind::Underscore,
            _ => TokenKind::Identifier(name),
        };
        Ok(Token { kind, span })
    }

    fn read_eq_or_eqeq(&mut self) -> Result<Token, LexError> {
        let start = self.position;
        self.position += 1;
        if self.peek() == '=' {
            self.position += 1;
            Ok(Token {
                kind: TokenKind::EqEq,
                span: self.span_at(start, 2),
            })
        } else if self.peek() == '>' {
            self.position += 1;
            Ok(Token {
                kind: TokenKind::FatArrow,
                span: self.span_at(start, 2),
            })
        } else {
            Ok(Token {
                kind: TokenKind::Eq,
                span: self.span_at(start, 1),
            })
        }
    }

    fn read_bang_or_ne(&mut self) -> Result<Token, LexError> {
        let start = self.position;
        self.position += 1;
        if self.peek() == '=' {
            self.position += 1;
            Ok(Token {
                kind: TokenKind::Ne,
                span: self.span_at(start, 2),
            })
        } else {
            Ok(Token {
                kind: TokenKind::Bang,
                span: self.span_at(start, 1),
            })
        }
    }

    fn read_lt_or_le(&mut self) -> Result<Token, LexError> {
        let start = self.position;
        self.position += 1;
        // Shift operators
        if self.peek() == '<' {
            self.position += 1; // Consume second '<'
            if self.peek() == '=' {
                self.position += 1;
                Ok(Token {
                    kind: TokenKind::ShiftLeftEq,
                    span: self.span_at(start, 3),
                })
            } else {
                Ok(Token {
                    kind: TokenKind::ShiftLeft,
                    span: self.span_at(start, 2),
                })
            }
        } else if self.peek() == '=' {
            self.position += 1;
            Ok(Token {
                kind: TokenKind::Le,
                span: self.span_at(start, 2),
            })
        } else {
            Ok(Token {
                kind: TokenKind::Lt,
                span: self.span_at(start, 1),
            })
        }
    }

    fn read_gt_or_ge(&mut self) -> Result<Token, LexError> {
        let start = self.position;
        self.position += 1;
        // Shift operators
        if self.peek() == '>' {
            self.position += 1; // Consume second '>'
            if self.peek() == '=' {
                self.position += 1;
                Ok(Token {
                    kind: TokenKind::ShiftRightEq,
                    span: self.span_at(start, 3),
                })
            } else {
                Ok(Token {
                    kind: TokenKind::ShiftRight,
                    span: self.span_at(start, 2),
                })
            }
        } else if self.peek() == '=' {
            self.position += 1;
            Ok(Token {
                kind: TokenKind::Ge,
                span: self.span_at(start, 2),
            })
        } else {
            Ok(Token {
                kind: TokenKind::Gt,
                span: self.span_at(start, 1),
            })
        }
    }

    fn read_ampersand(&mut self) -> Result<Token, LexError> {
        let start = self.position;
        self.position += 1;
        if self.peek() == '&' {
            self.position += 1;
            Ok(Token {
                kind: TokenKind::AndAnd,
                span: self.span_at(start, 2),
            })
        } else if self.peek() == '=' {
            self.position += 1;
            Ok(Token {
                kind: TokenKind::AndEq,
                span: self.span_at(start, 2),
            })
        } else {
            Ok(Token {
                kind: TokenKind::Amp,
                span: self.span_at(start, 1),
            })
        }
    }

    fn read_pipe(&mut self) -> Result<Token, LexError> {
        let start = self.position;
        self.position += 1;
        if self.peek() == '|' {
            self.position += 1;
            Ok(Token {
                kind: TokenKind::OrOr,
                span: self.span_at(start, 2),
            })
        } else if self.peek() == '=' {
            self.position += 1;
            Ok(Token {
                kind: TokenKind::OrEq,
                span: self.span_at(start, 2),
            })
        } else {
            Ok(Token {
                kind: TokenKind::Pipe,
                span: self.span_at(start, 1),
            })
        }
    }

    fn read_caret(&mut self) -> Result<Token, LexError> {
        let start = self.position;
        self.position += 1;
        if self.peek() == '=' {
            self.position += 1;
            Ok(Token {
                kind: TokenKind::CaretEq,
                span: self.span_at(start, 2),
            })
        } else {
            Ok(Token {
                kind: TokenKind::Caret,
                span: self.span_at(start, 1),
            })
        }
    }

    fn read_plus(&mut self) -> Result<Token, LexError> {
        let start = self.position;
        self.position += 1;
        if self.peek() == '=' {
            self.position += 1;
            Ok(Token {
                kind: TokenKind::PlusEq,
                span: self.span_at(start, 2),
            })
        } else {
            Ok(Token {
                kind: TokenKind::Plus,
                span: self.span_at(start, 1),
            })
        }
    }

    fn read_minus(&mut self) -> Result<Token, LexError> {
        let start = self.position;
        self.position += 1;
        if self.peek() == '=' {
            self.position += 1;
            Ok(Token {
                kind: TokenKind::MinusEq,
                span: self.span_at(start, 2),
            })
        } else {
            Ok(Token {
                kind: TokenKind::Minus,
                span: self.span_at(start, 1),
            })
        }
    }

    fn read_star(&mut self) -> Result<Token, LexError> {
        let start = self.position;
        self.position += 1;
        if self.peek() == '=' {
            self.position += 1;
            Ok(Token {
                kind: TokenKind::StarEq,
                span: self.span_at(start, 2),
            })
        } else {
            Ok(Token {
                kind: TokenKind::Star,
                span: self.span_at(start, 1),
            })
        }
    }

    fn read_slash(&mut self) -> Result<Token, LexError> {
        let start = self.position;
        self.position += 1;
        if self.peek() == '=' {
            self.position += 1;
            Ok(Token {
                kind: TokenKind::SlashEq,
                span: self.span_at(start, 2),
            })
        } else {
            Ok(Token {
                kind: TokenKind::Slash,
                span: self.span_at(start, 1),
            })
        }
    }

    fn read_percent(&mut self) -> Result<Token, LexError> {
        let start = self.position;
        self.position += 1;
        if self.peek() == '=' {
            self.position += 1;
            Ok(Token {
                kind: TokenKind::PercentEq,
                span: self.span_at(start, 2),
            })
        } else {
            Ok(Token {
                kind: TokenKind::Percent,
                span: self.span_at(start, 1),
            })
        }
    }

    fn read_dot(&mut self) -> Result<Token, LexError> {
        let start = self.position;
        self.position += 1;
        if self.position < self.input.len() && self.peek() == '.' {
            self.position += 1;
            Ok(Token {
                kind: TokenKind::DotDot,
                span: self.span_at(start, 2),
            })
        } else {
            Ok(Token {
                kind: TokenKind::Dot,
                span: self.span_at(start, 1),
            })
        }
    }

    fn read_punct(&mut self) -> Result<Token, LexError> {
        let start = self.position;
        let ch = self.peek();
        let kind = match ch {
            '(' => TokenKind::LParen,
            ')' => TokenKind::RParen,
            '{' => TokenKind::LBrace,
            '}' => TokenKind::RBrace,
            '[' => TokenKind::LBracket,
            ']' => TokenKind::RBracket,
            ':' => TokenKind::Colon,
            ';' => TokenKind::Semicolon,
            ',' => TokenKind::Comma,
            '~' => TokenKind::Tilde,
            _ => {
                return Err(LexError::new(
                    "internal lexer error: read_punct",
                    self.span_at(start, 1),
                ));
            }
        };
        self.position += 1;
        Ok(Token {
            kind,
            span: self.span_at(start, 1),
        })
    }

    fn read_integer(&mut self) -> Result<Token, LexError> {
        let token_start = self.position;
        let prefix_start = self.position;

        if self.peek() == '0' {
            let next = self.peek_next();
            match next {
                Some('b') | Some('B') => {
                    self.position += 2;
                    self.read_prefixed_integer("binary", 2, token_start)?;
                    let original = self.input[prefix_start..self.position]
                        .iter()
                        .collect::<String>();
                    let value = u64::from_str_radix(&original[2..].replace('_', ""), 2).unwrap_or(0);
                    let span = self.span_at(token_start, self.position - token_start);
                    return Ok(Token {
                        kind: TokenKind::IntegerLiteral {
                            value,
                            original,
                            radix: 2,
                        },
                        span,
                    });
                }
                Some('o') | Some('O') => {
                    self.position += 2;
                    self.read_prefixed_integer("octal", 8, token_start)?;
                    let original = self.input[prefix_start..self.position]
                        .iter()
                        .collect::<String>();
                    let value = u64::from_str_radix(&original[2..].replace('_', ""), 8).unwrap_or(0);
                    let span = self.span_at(token_start, self.position - token_start);
                    return Ok(Token {
                        kind: TokenKind::IntegerLiteral {
                            value,
                            original,
                            radix: 8,
                        },
                        span,
                    });
                }
                Some('x') | Some('X') => {
                    self.position += 2;
                    self.read_prefixed_integer("hexadecimal", 16, token_start)?;
                    let original = self.input[prefix_start..self.position]
                        .iter()
                        .collect::<String>();
                    let value = u64::from_str_radix(&original[2..].replace('_', ""), 16).unwrap_or(0);
                    let span = self.span_at(token_start, self.position - token_start);
                    return Ok(Token {
                        kind: TokenKind::IntegerLiteral {
                            value,
                            original,
                            radix: 16,
                        },
                        span,
                    });
                }
                _ => {}
            }
        }

        while self.position < self.input.len() {
            let ch = self.peek();
            if ch.is_ascii_digit() || ch == '_' {
                self.position += 1;
            } else {
                break;
            }
        }

        let original = self.input[prefix_start..self.position]
            .iter()
            .collect::<String>();
        let value = u64::from_str_radix(&original.replace('_', ""), 10).unwrap_or(0);

        let span = self.span_at(token_start, self.position - token_start);
        Ok(Token {
            kind: TokenKind::IntegerLiteral {
                value,
                original,
                radix: 10,
            },
            span,
        })
    }

    fn read_number_literal(&mut self) -> Result<Token, LexError> {
        // Handle prefixed integers first (`0b...`, `0o...`, `0x...`).
        if self.peek() == '0' {
            if let Some(next) = self.peek_next() {
                match next {
                    'b' | 'B' | 'o' | 'O' | 'x' | 'X' => {
                        return self.read_integer();
                    }
                    _ => {}
                }
            }
        }

        let token_start = self.position;

        // Mantissa: digits + underscores.
        while self.position < self.input.len() {
            let ch = self.peek();
            if ch.is_ascii_digit() || ch == '_' {
                self.position += 1;
            } else {
                break;
            }
        }

        let mut saw_dot = false;
        let mut saw_exp = false;

        // Optional fractional part: `.<digits...>`
        if self.peek() == '.'
            && self.peek_next() != Some('.')
            && self.peek_next().is_some()
        {
            let _dot_start = self.position;
            // Consume '.'.
            self.position += 1;
            saw_dot = true;

            let mut has_frac_digit = false;
            while self.position < self.input.len() {
                let ch = self.peek();
                if ch.is_ascii_digit() {
                    has_frac_digit = true;
                    self.position += 1;
                } else if ch == '_' {
                    self.position += 1;
                } else {
                    break;
                }
            }

            // If we didn't get any digits after '.', fall back to integer + '.' tokenization.
            if !has_frac_digit {
                // Rewind so the '.' can be lexed by the normal '.' logic.
                self.position = _dot_start;
                saw_dot = false;
            }
        }

        // Optional exponent part: `e[+/-]<digits...>`
        if matches!(self.peek(), 'e' | 'E') {
            saw_exp = true;
            self.position += 1; // consume e/E

            while self.position < self.input.len() && self.peek() == '_' {
                self.position += 1;
            }

            if matches!(self.peek(), '+' | '-') {
                self.position += 1;
            }

            while self.position < self.input.len() && self.peek() == '_' {
                self.position += 1;
            }

            let mut has_exp_digit = false;
            while self.position < self.input.len() {
                let ch = self.peek();
                if ch.is_ascii_digit() {
                    has_exp_digit = true;
                    self.position += 1;
                } else if ch == '_' {
                    self.position += 1;
                } else {
                    break;
                }
            }

            if !has_exp_digit {
                // Not a valid float exponent; treat as integer and let parser see `e` as identifier.
                saw_exp = false;
            }
        }

        let span = self.span_at(token_start, self.position - token_start);
        let original = self.input[token_start..self.position]
            .iter()
            .collect::<String>();
        let cleaned = original.replace('_', "");

        if saw_dot || saw_exp {
            Ok(Token {
                kind: TokenKind::FloatLiteral { original, cleaned },
                span,
            })
        } else {
            let value = u64::from_str_radix(&cleaned, 10).unwrap_or(0);
            Ok(Token {
                kind: TokenKind::IntegerLiteral {
                    value,
                    original,
                    radix: 10,
                },
                span,
            })
        }
    }

    fn read_prefixed_integer(
        &mut self,
        literal_type: &str,
        radix: u32,
        token_start: usize,
    ) -> Result<(), LexError> {
        let mut has_digit = false;
        while self.position < self.input.len() {
            let ch = self.peek();
            if ch == '_' {
                self.position += 1;
            } else if radix == 16 && ch.is_ascii_hexdigit() {
                has_digit = true;
                self.position += 1;
            } else if radix == 8 && ch.is_ascii_digit() && ch < '8' {
                has_digit = true;
                self.position += 1;
            } else if radix == 2 && (ch == '0' || ch == '1') {
                has_digit = true;
                self.position += 1;
            } else if ch.is_ascii_digit() {
                has_digit = true;
                self.position += 1;
            } else {
                break;
            }
        }

        if !has_digit {
            let len = (self.position - token_start).max(1);
            return Err(LexError::new(
                format!("empty {} literal", literal_type),
                self.span_at(token_start, len),
            ));
        }

        Ok(())
    }

    fn peek(&self) -> char {
        if self.position < self.input.len() {
            self.input[self.position]
        } else {
            '\0'
        }
    }

    fn peek_next(&self) -> Option<char> {
        if self.position + 1 < self.input.len() {
            Some(self.input[self.position + 1])
        } else {
            None
        }
    }

    fn skip_whitespace(&mut self) {
        while self.position < self.input.len() && self.input[self.position].is_whitespace() {
            self.position += 1;
        }
    }
}

fn is_ident_start(ch: char) -> bool {
    ch.is_ascii_alphabetic() || ch == '_'
}

fn is_ident_continue(ch: char) -> bool {
    is_ident_start(ch) || ch.is_ascii_digit()
}

fn describe_char(ch: char) -> String {
    match ch {
        '\n' => "'\\n' (U+000A)".to_string(),
        '\r' => "'\\r' (U+000D)".to_string(),
        '\t' => "'\\t' (U+0009)".to_string(),
        c if c.is_control() => format!("U+{:04X}", c as u32),
        c => format!("'{}' (U+{:04X})", c, c as u32),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_single_line_comment() {
        let mut lexer = Lexer::new("// This is a comment");
        let tokens = lexer.tokenize().unwrap();
        assert_eq!(
            tokens[0].kind,
            TokenKind::SingleLineComment(" This is a comment".to_string())
        );
        assert_eq!(tokens[0].span, Span::new(1, 1, 20));
    }

    #[test]
    fn test_multi_line_comment() {
        let mut lexer = Lexer::new("/* This is a multi-line comment */");
        let tokens = lexer.tokenize().unwrap();
        assert_eq!(
            tokens[0].kind,
            TokenKind::MultiLineComment(" This is a multi-line comment ".to_string())
        );
        assert_eq!(tokens[0].span, Span::new(1, 1, 34));
    }

    #[test]
    fn test_multiple_comments() {
        let source = "// First comment\n/* Second comment */";
        let mut lexer = Lexer::new(source);
        let tokens = lexer.tokenize().unwrap();
        assert_eq!(tokens.len(), 3);
        assert!(matches!(
            tokens[0].kind,
            TokenKind::SingleLineComment(_)
        ));
        assert!(matches!(
            tokens[1].kind,
            TokenKind::MultiLineComment(_)
        ));
        assert_eq!(tokens[1].span.line, 2);
    }

    #[test]
    fn test_unterminated_multi_line_comment() {
        let mut lexer = Lexer::new("/* Unterminated comment");
        let result = lexer.tokenize();
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.span.line, 1);
        assert_eq!(err.span.column, 1);
        assert!(err.message.contains("unterminated"));
    }

    #[test]
    fn test_unknown_character() {
        let mut lexer = Lexer::new("42 @");
        let result = lexer.tokenize();
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.message.contains("unexpected character"));
        assert_eq!(err.span.line, 1);
        assert_eq!(err.span.column, 4);
    }

    #[test]
    fn test_decimal_literals() {
        let mut lexer = Lexer::new("0 1 42 12345");
        let tokens = lexer.tokenize().unwrap();
        assert_eq!(tokens.len(), 5);

        assert_eq!(
            tokens[0].kind,
            TokenKind::IntegerLiteral {
                value: 0,
                original: "0".to_string(),
                radix: 10
            }
        );
        assert_eq!(
            tokens[1].kind,
            TokenKind::IntegerLiteral {
                value: 1,
                original: "1".to_string(),
                radix: 10
            }
        );
        assert_eq!(
            tokens[2].kind,
            TokenKind::IntegerLiteral {
                value: 42,
                original: "42".to_string(),
                radix: 10
            }
        );
    }

    #[test]
    fn test_decimal_with_underscores() {
        let mut lexer = Lexer::new("1_000 12_345_678");
        let tokens = lexer.tokenize().unwrap();
        assert_eq!(tokens.len(), 3);

        assert_eq!(
            tokens[0].kind,
            TokenKind::IntegerLiteral {
                value: 1000,
                original: "1_000".to_string(),
                radix: 10
            }
        );
        assert_eq!(
            tokens[1].kind,
            TokenKind::IntegerLiteral {
                value: 12345678,
                original: "12_345_678".to_string(),
                radix: 10
            }
        );
    }

    #[test]
    fn test_float_decimal_literals() {
        let mut lexer = Lexer::new("1.0 12.34");
        let tokens = lexer.tokenize().unwrap();
        // 2 literals + Eof
        assert_eq!(tokens.len(), 3);

        assert_eq!(
            tokens[0].kind,
            TokenKind::FloatLiteral {
                original: "1.0".to_string(),
                cleaned: "1.0".to_string()
            }
        );
        assert_eq!(
            tokens[1].kind,
            TokenKind::FloatLiteral {
                original: "12.34".to_string(),
                cleaned: "12.34".to_string()
            }
        );
    }

    #[test]
    fn test_float_exponent_literals() {
        let mut lexer = Lexer::new("1e0 1e-1 1E10 1.5e-1");
        let tokens = lexer.tokenize().unwrap();
        // 4 literals + Eof
        assert_eq!(tokens.len(), 5);

        assert_eq!(
            tokens[0].kind,
            TokenKind::FloatLiteral {
                original: "1e0".to_string(),
                cleaned: "1e0".to_string()
            }
        );
        assert_eq!(
            tokens[1].kind,
            TokenKind::FloatLiteral {
                original: "1e-1".to_string(),
                cleaned: "1e-1".to_string()
            }
        );
        assert_eq!(
            tokens[2].kind,
            TokenKind::FloatLiteral {
                original: "1E10".to_string(),
                cleaned: "1E10".to_string()
            }
        );
        assert_eq!(
            tokens[3].kind,
            TokenKind::FloatLiteral {
                original: "1.5e-1".to_string(),
                cleaned: "1.5e-1".to_string()
            }
        );
    }

    #[test]
    fn test_float_underscores() {
        let mut lexer = Lexer::new("12_34_5.78_90 1_e_-1");
        let tokens = lexer.tokenize().unwrap();
        // 2 literals + Eof
        assert_eq!(tokens.len(), 3);

        assert_eq!(
            tokens[0].kind,
            TokenKind::FloatLiteral {
                original: "12_34_5.78_90".to_string(),
                cleaned: "12345.7890".to_string()
            }
        );
        assert_eq!(
            tokens[1].kind,
            TokenKind::FloatLiteral {
                original: "1_e_-1".to_string(),
                cleaned: "1e-1".to_string()
            }
        );
    }

    #[test]
    fn test_binary_literals() {
        let mut lexer = Lexer::new("0b0 0b1 0b1010 0b1000_0000");
        let tokens = lexer.tokenize().unwrap();
        assert_eq!(tokens.len(), 5);

        assert_eq!(
            tokens[0].kind,
            TokenKind::IntegerLiteral {
                value: 0,
                original: "0b0".to_string(),
                radix: 2
            }
        );
        assert_eq!(
            tokens[1].kind,
            TokenKind::IntegerLiteral {
                value: 1,
                original: "0b1".to_string(),
                radix: 2
            }
        );
        assert_eq!(
            tokens[3].kind,
            TokenKind::IntegerLiteral {
                value: 128,
                original: "0b1000_0000".to_string(),
                radix: 2
            }
        );
    }

    #[test]
    fn test_octal_literals() {
        let mut lexer = Lexer::new("0o0 0o7 0o1234_567 0o777");
        let tokens = lexer.tokenize().unwrap();
        assert_eq!(tokens.len(), 5);

        assert_eq!(
            tokens[0].kind,
            TokenKind::IntegerLiteral {
                value: 0,
                original: "0o0".to_string(),
                radix: 8
            }
        );
        assert_eq!(
            tokens[1].kind,
            TokenKind::IntegerLiteral {
                value: 7,
                original: "0o7".to_string(),
                radix: 8
            }
        );
        assert_eq!(
            tokens[3].kind,
            TokenKind::IntegerLiteral {
                value: 511,
                original: "0o777".to_string(),
                radix: 8
            }
        );
    }

    #[test]
    fn test_hex_literals() {
        let mut lexer = Lexer::new("0x0 0xaa 0xdeadbeef 0xDEAD_bEaf");
        let tokens = lexer.tokenize().unwrap();
        assert_eq!(tokens.len(), 5);

        assert_eq!(
            tokens[0].kind,
            TokenKind::IntegerLiteral {
                value: 0,
                original: "0x0".to_string(),
                radix: 16
            }
        );
        assert_eq!(
            tokens[1].kind,
            TokenKind::IntegerLiteral {
                value: 0xaa,
                original: "0xaa".to_string(),
                radix: 16
            }
        );
        assert_eq!(
            tokens[2].kind,
            TokenKind::IntegerLiteral {
                value: 0xdeadbeef,
                original: "0xdeadbeef".to_string(),
                radix: 16
            }
        );
        assert_eq!(
            tokens[3].kind,
            TokenKind::IntegerLiteral {
                value: 0xDEADbEaf,
                original: "0xDEAD_bEaf".to_string(),
                radix: 16
            }
        );
    }

    #[test]
    fn test_string_empty() {
        let mut lexer = Lexer::new(r#""""#);
        let tokens = lexer.tokenize().unwrap();
        assert_eq!(tokens.len(), 2);
        assert_eq!(
            tokens[0].kind,
            TokenKind::StringLiteral {
                value: String::new(),
                original: r#""""#.to_string(),
            }
        );
    }

    #[test]
    fn test_string_simple() {
        let mut lexer = Lexer::new(r#""Hello""#);
        let tokens = lexer.tokenize().unwrap();
        assert_eq!(
            tokens[0].kind,
            TokenKind::StringLiteral {
                value: "Hello".to_string(),
                original: r#""Hello""#.to_string(),
            }
        );
    }

    #[test]
    fn test_string_escapes() {
        let mut lexer = Lexer::new(r#""a\nb\r\"d\\e""#);
        let tokens = lexer.tokenize().unwrap();
        assert_eq!(
            tokens[0].kind,
            TokenKind::StringLiteral {
                value: "a\nb\r\"d\\e".to_string(),
                original: r#""a\nb\r\"d\\e""#.to_string(),
            }
        );
    }

    #[test]
    fn test_string_line_from_example() {
        let s = r#""Also it must support other escape-sequences like \" quotes \", \r - resetting""#;
        let mut lexer = Lexer::new(s);
        let tokens = lexer.tokenize().unwrap();
        let TokenKind::StringLiteral { value, .. } = &tokens[0].kind else {
            panic!("expected string");
        };
        assert!(value.contains("quotes"));
        assert!(value.contains('\r'));
        assert!(value.contains('"'));
    }

    #[test]
    fn test_unterminated_string() {
        let mut lexer = Lexer::new("\"no close");
        let err = lexer.tokenize().unwrap_err();
        assert!(err.message.contains("unterminated"));
    }

    #[test]
    fn test_invalid_escape() {
        let mut lexer = Lexer::new(r#""\z""#);
        let err = lexer.tokenize().unwrap_err();
        assert!(err.message.contains("invalid escape"));
    }

    #[test]
    fn test_tokenize_example0_2() {
        let src = include_str!("../examples/basics/example0_2.vc");
        let mut lexer = Lexer::new(src);
        let tokens = lexer.tokenize().expect("example0_2.vc should tokenize");
        let strings = tokens
            .iter()
            .filter(|t| matches!(t.kind, TokenKind::StringLiteral { .. }))
            .count();
        assert!(
            strings >= 10,
            "expected many string literals in example0_2.vc, got {}",
            strings
        );
        assert!(matches!(
            tokens.last().unwrap().kind,
            TokenKind::Eof
        ));
    }

    #[test]
    fn test_identifier_and_punct() {
        let mut lexer = Lexer::new("func main ( ) { }");
        let tokens = lexer.tokenize().unwrap();
        assert_eq!(tokens[0].kind, TokenKind::Identifier("func".to_string()));
        assert_eq!(tokens[1].kind, TokenKind::Identifier("main".to_string()));
        assert_eq!(tokens[2].kind, TokenKind::LParen);
        assert_eq!(tokens[3].kind, TokenKind::RParen);
        assert_eq!(tokens[4].kind, TokenKind::LBrace);
        assert_eq!(tokens[5].kind, TokenKind::RBrace);
        assert_eq!(tokens[6].kind, TokenKind::Eof);
    }

    #[test]
    fn test_colon_semicolon_comma() {
        let mut lexer = Lexer::new("a: b; c, d");
        let tokens = lexer.tokenize().unwrap();
        assert_eq!(tokens[0].kind, TokenKind::Identifier("a".to_string()));
        assert_eq!(tokens[1].kind, TokenKind::Colon);
        assert_eq!(tokens[2].kind, TokenKind::Identifier("b".to_string()));
        assert_eq!(tokens[3].kind, TokenKind::Semicolon);
        assert_eq!(tokens[4].kind, TokenKind::Identifier("c".to_string()));
        assert_eq!(tokens[5].kind, TokenKind::Comma);
        assert_eq!(tokens[6].kind, TokenKind::Identifier("d".to_string()));
    }

    #[test]
    fn test_plus_operator() {
        let mut lexer = Lexer::new("a + b");
        let tokens = lexer.tokenize().unwrap();
        assert_eq!(tokens[0].kind, TokenKind::Identifier("a".to_string()));
        assert_eq!(tokens[1].kind, TokenKind::Plus);
        assert_eq!(tokens[2].kind, TokenKind::Identifier("b".to_string()));
    }

    #[test]
    fn test_let_underscore_eq_tokens() {
        let mut lexer = Lexer::new("let x: Int = 1; let _ = 2;");
        let tokens = lexer.tokenize().unwrap();
        assert_eq!(tokens[0].kind, TokenKind::Let);
        assert_eq!(tokens[1].kind, TokenKind::Identifier("x".to_string()));
        assert_eq!(tokens[2].kind, TokenKind::Colon);
        assert_eq!(tokens[3].kind, TokenKind::Identifier("Int".to_string()));
        assert_eq!(tokens[4].kind, TokenKind::Eq);
        assert!(matches!(
            tokens[5].kind,
            TokenKind::IntegerLiteral { value: 1, .. }
        ));
        assert_eq!(tokens[6].kind, TokenKind::Semicolon);
        assert_eq!(tokens[7].kind, TokenKind::Let);
        assert_eq!(tokens[8].kind, TokenKind::Underscore);
        assert_eq!(tokens[9].kind, TokenKind::Eq);
        assert!(matches!(
            tokens[10].kind,
            TokenKind::IntegerLiteral { value: 2, .. }
        ));
        assert_eq!(tokens[11].kind, TokenKind::Semicolon);
        assert!(matches!(tokens[12].kind, TokenKind::Eof));
    }

    #[test]
    fn test_bool_keywords_comparison_logic_ops() {
        let mut lexer = Lexer::new("true false if else == != < > <= >= && || !");
        let tokens = lexer.tokenize().unwrap();
        assert_eq!(tokens[0].kind, TokenKind::True);
        assert_eq!(tokens[1].kind, TokenKind::False);
        assert_eq!(tokens[2].kind, TokenKind::If);
        assert_eq!(tokens[3].kind, TokenKind::Else);
        assert_eq!(tokens[4].kind, TokenKind::EqEq);
        assert_eq!(tokens[5].kind, TokenKind::Ne);
        assert_eq!(tokens[6].kind, TokenKind::Lt);
        assert_eq!(tokens[7].kind, TokenKind::Gt);
        assert_eq!(tokens[8].kind, TokenKind::Le);
        assert_eq!(tokens[9].kind, TokenKind::Ge);
        assert_eq!(tokens[10].kind, TokenKind::AndAnd);
        assert_eq!(tokens[11].kind, TokenKind::OrOr);
        assert_eq!(tokens[12].kind, TokenKind::Bang);
        assert!(matches!(tokens[13].kind, TokenKind::Eof));
    }

    #[test]
    fn test_bitwise_amp_pipe_caret_and_compounds() {
        let mut lexer = Lexer::new("a & b | c ^ d += -= *= /= %= &= |= ^=");
        let tokens = lexer.tokenize().unwrap();
        assert!(matches!(tokens[1].kind, TokenKind::Amp));
        assert!(matches!(tokens[3].kind, TokenKind::Pipe));
        assert!(matches!(tokens[5].kind, TokenKind::Caret));
        assert!(matches!(tokens[7].kind, TokenKind::PlusEq));
        assert!(matches!(tokens[8].kind, TokenKind::MinusEq));
        assert!(matches!(tokens[9].kind, TokenKind::StarEq));
        assert!(matches!(tokens[10].kind, TokenKind::SlashEq));
        assert!(matches!(tokens[11].kind, TokenKind::PercentEq));
        assert!(matches!(tokens[12].kind, TokenKind::AndEq));
        assert!(matches!(tokens[13].kind, TokenKind::OrEq));
        assert!(matches!(tokens[14].kind, TokenKind::CaretEq));
    }

    #[test]
    fn test_while_break_continue_keywords() {
        let mut lexer = Lexer::new("while break continue");
        let tokens = lexer.tokenize().unwrap();
        assert_eq!(tokens[0].kind, TokenKind::While);
        assert_eq!(tokens[1].kind, TokenKind::Break);
        assert_eq!(tokens[2].kind, TokenKind::Continue);
    }

    #[test]
    fn test_tokenize_example3_1() {
        let src = include_str!("../examples/basics/example3_1.vc");
        let mut lexer = Lexer::new(src);
        let tokens = lexer.tokenize().expect("example3_1.vc");
        let idents: Vec<_> = tokens
            .iter()
            .filter_map(|t| match &t.kind {
                TokenKind::Identifier(s) => Some(s.as_str()),
                _ => None,
            })
            .collect();
        assert!(idents.iter().any(|&s| s == "itos"));
        assert!(idents.iter().any(|&s| s == "concat"));
        assert!(idents.iter().any(|&s| s == "String"));
        assert!(matches!(tokens.last().unwrap().kind, TokenKind::Eof));
    }

    #[test]
    fn test_tokenize_example2() {
        let src = include_str!("../examples/basics/example2_0.vc");
        let mut lexer = Lexer::new(src);
        let tokens = lexer.tokenize().expect("example2_0.vc");
        assert!(tokens.iter().any(|t| {
            matches!(&t.kind, TokenKind::Identifier(s) if s == "internal")
        }));
        assert!(tokens.iter().any(|t| {
            matches!(&t.kind, TokenKind::Identifier(s) if s == "String")
        }));
        assert!(matches!(tokens.last().unwrap().kind, TokenKind::Eof));
    }

    #[test]
    fn test_tokenize_example1() {
        let src = include_str!("../examples/basics/example1.vc");
        let mut lexer = Lexer::new(src);
        let tokens = lexer.tokenize().expect("example1.vc should tokenize");
        let has_func = tokens.iter().any(|t| {
            matches!(&t.kind, TokenKind::Identifier(s) if s == "func")
        });
        let has_main = tokens.iter().any(|t| {
            matches!(&t.kind, TokenKind::Identifier(s) if s == "main")
        });
        assert!(has_func && has_main);
        assert!(matches!(tokens.last().unwrap().kind, TokenKind::Eof));
    }

    #[test]
    fn test_tokenize_example4() {
        let src = include_str!("../examples/operators/example4.vc");
        let mut lexer = Lexer::new(src);
        let tokens = lexer.tokenize().expect("example4.vc should tokenize");
        assert!(matches!(tokens.last().unwrap().kind, TokenKind::Eof));

        let idents: Vec<_> = tokens
            .iter()
            .filter_map(|t| match &t.kind {
                TokenKind::Identifier(s) => Some(s.as_str()),
                _ => None,
            })
            .collect();
        assert!(idents.iter().any(|&s| s == "inverse_bits"));
        assert!(idents.iter().any(|&s| s == "priority_chains"));
        assert!(idents.iter().any(|&s| s == "itos"));

        let kinds: Vec<_> = tokens.iter().map(|t| &t.kind).collect();
        assert!(kinds.iter().any(|k| matches!(k, TokenKind::Plus)));
        assert!(kinds.iter().any(|k| matches!(k, TokenKind::Minus)));
        assert!(kinds.iter().any(|k| matches!(k, TokenKind::Star)));
        assert!(kinds.iter().any(|k| matches!(k, TokenKind::Slash)));
        assert!(kinds.iter().any(|k| matches!(k, TokenKind::Percent)));
        assert!(kinds.iter().any(|k| matches!(k, TokenKind::Tilde)));
    }
}
