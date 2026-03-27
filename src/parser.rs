pub use crate::ast::{
    AstNode, BinaryOp, CallArg, CompoundOp, ExtensionReceiver, FunctionTypeParam, GenericParam,
    ImportBinding, LambdaBody, LambdaParam, Param, Pattern, PatternElem, TypeExpr, UnaryOp,
};
use crate::error::{ParseError, Span};
use crate::lexer::{Lexer, Token, TokenKind, EOF_SENTINEL};

pub struct Parser {
    tokens: Vec<Token>,
    position: usize,
}

impl Parser {
    fn type_expr_receiver_key(ty: &TypeExpr) -> Option<String> {
        match ty {
            TypeExpr::Named(n) => Some(n.clone()),
            TypeExpr::Unit => Some("()".to_string()),
            TypeExpr::Array(elem) => {
                let inner = Self::type_expr_receiver_key(elem.as_ref())?;
                Some(format!("[{inner}]"))
            }
            TypeExpr::Tuple(parts) => {
                let mut out = String::from("(");
                for (i, p) in parts.iter().enumerate() {
                    if i > 0 {
                        out.push_str(", ");
                    }
                    out.push_str(&Self::type_expr_receiver_key(p)?);
                }
                if parts.len() == 1 {
                    out.push(',');
                }
                out.push(')');
                Some(out)
            }
            TypeExpr::TypeParam(n) => Some(n.clone()),
            TypeExpr::EnumApp { name, args } => {
                let mut parts = Vec::new();
                for a in args {
                    parts.push(Self::type_expr_receiver_key(a)?);
                }
                Some(format!("{}<{}>", name, parts.join(", ")))
            }
            TypeExpr::Function { params, ret } => {
                let mut p = Vec::new();
                for part in params {
                    let rendered = Self::type_expr_receiver_key(&part.ty)?;
                    if let Some(n) = &part.name {
                        p.push(format!("{n}: {rendered}"));
                    } else {
                        p.push(rendered);
                    }
                }
                Some(format!(
                    "({}) => {}",
                    p.join(", "),
                    Self::type_expr_receiver_key(ret)?
                ))
            }
            TypeExpr::Infer => None,
        }
    }
    pub fn new(tokens: Vec<Token>) -> Self {
        Parser {
            tokens,
            position: 0,
        }
    }

    pub fn parse(&mut self) -> Result<AstNode, ParseError> {
        let mut items = Vec::new();
        while !self.is_at_end() {
            items.push(self.parse_top_level_item()?);
        }
        Ok(AstNode::Program(items))
    }

    fn parse_top_level_item(&mut self) -> Result<AstNode, ParseError> {
        let token = self.peek().clone();
        match token.kind {
            TokenKind::Import => self.parse_import_declaration(),
            TokenKind::Export => {
                let export_span = self.peek().span;
                self.advance(); // `export`
                // `export name as name;`
                if matches!(self.peek().kind, TokenKind::Identifier(_))
                    && matches!(self.peek_n(1).kind, TokenKind::As)
                {
                    let (from, _) = self.take_identifier()?;
                    self.advance(); // `as`
                    let (to, _) = self.take_identifier()?;
                    self.expect_semicolon()?;
                    return Ok(AstNode::ExportAlias {
                        from,
                        to,
                        span: export_span,
                    });
                }
                if matches!(self.peek().kind, TokenKind::Identifier(_))
                    && matches!(self.peek_n(1).kind, TokenKind::Semicolon)
                {
                    let (name, name_span) = self.take_identifier()?;
                    self.expect_semicolon()?;
                    return Ok(AstNode::ExportName {
                        name,
                        name_span,
                        span: export_span,
                    });
                }
                let item = match self.peek().kind {
                    TokenKind::Struct => self.parse_struct_definition(false)?,
                    TokenKind::Enum => self.parse_enum_definition(false)?,
                    TokenKind::Type => self.parse_type_alias_definition()?,
                    TokenKind::Const => self.parse_const_statement(true)?,
                    TokenKind::Async => {
                        self.advance(); // `async`
                        self.expect_func_after_async()?;
                        self.parse_function_definition(true)?
                    }
                    TokenKind::Identifier(ref s) if s == "internal" => {
                        self.parse_internal_declaration()?
                    }
                    TokenKind::Identifier(ref s) if s == "func" => self.parse_function_definition(false)?,
                    _ => {
                        return Err(ParseError::UnexpectedToken {
                            message: "expected exportable declaration after `export`".to_string(),
                            span: Some(export_span),
                        });
                    }
                };
                self.mark_exported(item)
            }
            TokenKind::Let => self.parse_let_statement(true, true),
            TokenKind::Const => self.parse_const_statement(false),
            TokenKind::Struct => self.parse_struct_definition(false),
            TokenKind::Enum => self.parse_enum_definition(false),
            TokenKind::Type => self.parse_type_alias_definition(),
            TokenKind::Async => {
                self.advance(); // `async`
                self.expect_func_after_async()?;
                self.parse_function_definition(true)
            }
            TokenKind::Identifier(ref s) if s == "internal" => self.parse_internal_declaration(),
            TokenKind::Identifier(ref s) if s == "func" => self.parse_function_definition(false),
            TokenKind::Identifier(_) => self.parse_call_statement(),
            TokenKind::SingleLineComment(text) => {
                self.advance();
                Ok(AstNode::SingleLineComment(text))
            }
            TokenKind::MultiLineComment(text) => {
                self.advance();
                Ok(AstNode::MultiLineComment(text))
            }
            TokenKind::IntegerLiteral {
                value,
                original,
                radix,
            } => {
                let span = token.span;
                self.advance();
                Ok(AstNode::IntegerLiteral {
                    value,
                    original,
                    radix,
                    span,
                })
            }
            TokenKind::FloatLiteral { original, cleaned } => {
                let span = token.span;
                self.advance();
                Ok(AstNode::FloatLiteral {
                    original,
                    cleaned,
                    span,
                })
            }
            TokenKind::StringLiteral { value, original } => {
                let span = token.span;
                self.advance();
                Ok(AstNode::StringLiteral {
                    value,
                    original,
                    span,
                })
            }
            TokenKind::LParen
            | TokenKind::RParen
            | TokenKind::LBrace
            | TokenKind::RBrace
            | TokenKind::LBracket
            | TokenKind::RBracket
            | TokenKind::Colon
            | TokenKind::Semicolon
            | TokenKind::Comma
            | TokenKind::ColonColon
            | TokenKind::Plus
            | TokenKind::Minus
            | TokenKind::Star
            | TokenKind::Slash
            | TokenKind::Percent
            | TokenKind::Tilde
            | TokenKind::Eq
            | TokenKind::EqEq
            | TokenKind::Ne
            | TokenKind::Lt
            | TokenKind::Gt
            | TokenKind::Le
            | TokenKind::Ge
            | TokenKind::ShiftLeft
            | TokenKind::ShiftRight
            | TokenKind::AndAnd
            | TokenKind::AndEq
            | TokenKind::Amp
            | TokenKind::OrOr
            | TokenKind::OrEq
            | TokenKind::Pipe
            | TokenKind::Caret
            | TokenKind::CaretEq
            | TokenKind::PlusEq
            | TokenKind::MinusEq
            | TokenKind::StarEq
            | TokenKind::SlashEq
            | TokenKind::PercentEq
            | TokenKind::ShiftLeftEq
            | TokenKind::ShiftRightEq
            | TokenKind::Bang
            | TokenKind::Dot
            | TokenKind::DotDot
            | TokenKind::FatArrow
            | TokenKind::From
            | TokenKind::As => Err(ParseError::UnexpectedToken {
                message: "unexpected punctuation".to_string(),
                span: Some(self.peek().span),
            }),
            TokenKind::Underscore
            | TokenKind::True
            | TokenKind::False
            | TokenKind::If
            | TokenKind::Else
            | TokenKind::While
            | TokenKind::Match
            | TokenKind::Break
            | TokenKind::Continue
            | TokenKind::Params
            | TokenKind::Await => Err(ParseError::UnexpectedToken {
                message: "unexpected token at top level".to_string(),
                span: Some(self.peek().span),
            }),
            TokenKind::Eof => unreachable!("parse_top_level_item called at EOF"),
        }
    }

    /// Heuristic disambiguation for `<` after an identifier:
    /// treat as generic args only when it can plausibly be a type-arg list,
    /// not a relational comparison like `x < f(...)`.
    fn lt_starts_generic_args(&self) -> bool {
        // Treat `<...>` as generic args when a balanced `>` is followed by
        // tokens that can legally continue generic expressions/calls.
        let mut depth = 1i32; // at `<`
        let mut i = 1usize;
        loop {
            let tk = self.peek_n(i).kind.clone();
            match tk {
                TokenKind::Lt => depth += 1,
                TokenKind::Gt => {
                    depth -= 1;
                    if depth == 0 {
                        return matches!(
                            self.peek_n(i + 1).kind,
                            TokenKind::LParen
                                | TokenKind::ColonColon
                                | TokenKind::LBrace
                                | TokenKind::Semicolon
                                | TokenKind::Comma
                                | TokenKind::RParen
                                | TokenKind::RBracket
                                | TokenKind::RBrace
                                | TokenKind::Dot
                                | TokenKind::EqEq
                                | TokenKind::Ne
                                | TokenKind::Lt
                                | TokenKind::Gt
                                | TokenKind::Le
                                | TokenKind::Ge
                                | TokenKind::Plus
                                | TokenKind::Minus
                                | TokenKind::Star
                                | TokenKind::Slash
                                | TokenKind::Percent
                                | TokenKind::AndAnd
                                | TokenKind::OrOr
                                | TokenKind::Eq
                        );
                    }
                }
                TokenKind::Eof
                | TokenKind::Semicolon
                | TokenKind::LBrace
                | TokenKind::RBrace
                | TokenKind::Eq
                | TokenKind::FatArrow => return false,
                _ => {}
            }
            i += 1;
            if i > 128 {
                return false;
            }
        }
    }

    fn mark_exported(&self, item: AstNode) -> Result<AstNode, ParseError> {
        Ok(match item {
            AstNode::InternalFunction {
                name,
                type_params,
                params,
                return_type,
                name_span,
                is_async,
                ..
            } => AstNode::InternalFunction {
                name,
                type_params,
                params,
                return_type,
                name_span,
                is_exported: true,
                is_async,
            },
            AstNode::Function {
                name,
                extension_receiver,
                type_params,
                params,
                return_type,
                body,
                name_span,
                closing_span,
                is_async,
                ..
            } => AstNode::Function {
                name,
                extension_receiver,
                type_params,
                params,
                return_type,
                body,
                name_span,
                closing_span,
                is_exported: true,
                is_async,
            },
            AstNode::StructDef {
                name,
                type_params,
                fields,
                is_unit,
                is_internal,
                name_span,
                span,
                ..
            } => AstNode::StructDef {
                name,
                type_params,
                fields,
                is_unit,
                is_internal,
                name_span,
                span,
                is_exported: true,
            },
            AstNode::EnumDef {
                name,
                type_params,
                variants,
                is_internal,
                name_span,
                span,
                ..
            } => AstNode::EnumDef {
                name,
                type_params,
                variants,
                is_internal,
                name_span,
                span,
                is_exported: true,
            },
            AstNode::TypeAlias {
                name,
                type_params,
                target,
                name_span,
                span,
                ..
            } => AstNode::TypeAlias {
                name,
                type_params,
                target,
                name_span,
                span,
                is_exported: true,
            },
            AstNode::Let {
                pattern,
                type_annotation,
                initializer,
                is_const: true,
                span,
                ..
            } => AstNode::Let {
                pattern,
                type_annotation,
                initializer,
                is_const: true,
                is_exported: true,
                span,
            },
            _ => {
                return Err(ParseError::UnexpectedToken {
                    message: "item cannot be exported".to_string(),
                    span: None,
                })
            }
        })
    }

    fn parse_import_declaration(&mut self) -> Result<AstNode, ParseError> {
        let span = self.peek().span;
        self.advance(); // import
        self.expect_lbrace()?;
        let mut bindings = Vec::new();
        if !matches!(self.peek().kind, TokenKind::RBrace) {
            loop {
                let (export_name, export_span) = self.take_identifier()?;
                let (local_name, local_span) = if matches!(self.peek().kind, TokenKind::As) {
                    self.advance();
                    self.take_identifier()?
                } else {
                    (export_name.clone(), export_span)
                };
                bindings.push(ImportBinding {
                    export_name,
                    local_name,
                    local_span,
                });
                if matches!(self.peek().kind, TokenKind::Comma) {
                    self.advance();
                    continue;
                }
                break;
            }
        }
        self.expect_rbrace()?;
        if !matches!(self.peek().kind, TokenKind::From) {
            return Err(ParseError::UnexpectedToken {
                message: "expected `from` in import declaration".to_string(),
                span: Some(self.peek().span),
            });
        }
        self.advance();
        let module_path = match self.peek().kind.clone() {
            TokenKind::StringLiteral { value, .. } => {
                self.advance();
                value
            }
            _ => {
                return Err(ParseError::UnexpectedToken {
                    message: "expected module path string literal after `from`".to_string(),
                    span: Some(self.peek().span),
                })
            }
        };
        self.expect_semicolon()?;
        Ok(AstNode::Import {
            bindings,
            module_path,
            span,
        })
    }

    /// `let` at top level (`top_level`) or in a block. `require_initializer` for globals.
    fn parse_let_statement(
        &mut self,
        require_initializer: bool,
        top_level: bool,
    ) -> Result<AstNode, ParseError> {
        let span = self.peek().span;
        self.advance(); // `let`
        let pattern = {
            let p = self.parse_pattern()?;
            if top_level {
                self.validate_top_level_let_pattern(&p)?;
            }
            p
        };
        let type_annotation = if matches!(self.peek().kind, TokenKind::Colon) {
            self.advance();
            Some(self.parse_type_expr()?)
        } else {
            None
        };
        let initializer = if matches!(self.peek().kind, TokenKind::Eq) {
            self.advance();
            Some(Box::new(self.parse_expression()?))
        } else {
            None
        };
        if require_initializer && initializer.is_none() {
            return Err(ParseError::UnexpectedToken {
                message: "top-level `let` must have an initializer".to_string(),
                span: Some(span),
            });
        }
        self.expect_semicolon()?;
        Ok(AstNode::Let {
            pattern,
            type_annotation,
            initializer,
            is_const: false,
            is_exported: false,
            span,
        })
    }

    fn parse_const_statement(&mut self, already_consumed_export: bool) -> Result<AstNode, ParseError> {
        let span = self.peek().span;
        self.advance(); // `const`
        let (name, name_span) = self.take_identifier()?;
        let pattern = Pattern::Binding { name, name_span };
        let type_annotation = if matches!(self.peek().kind, TokenKind::Colon) {
            self.advance();
            Some(self.parse_type_expr()?)
        } else {
            None
        };
        self.expect_eq()?;
        let initializer = Some(Box::new(self.parse_expression()?));
        self.expect_semicolon()?;
        Ok(AstNode::Let {
            pattern,
            type_annotation,
            initializer,
            is_const: true,
            is_exported: already_consumed_export,
            span,
        })
    }

    fn validate_top_level_let_pattern(&self, p: &Pattern) -> Result<(), ParseError> {
        match p {
            Pattern::Wildcard { .. } | Pattern::Binding { .. } => Ok(()),
            Pattern::IntLiteral { span, .. } => Err(ParseError::UnexpectedToken {
                message: "literal patterns are not allowed at the top level".to_string(),
                span: Some(*span),
            }),
            Pattern::StringLiteral { span, .. } => Err(ParseError::UnexpectedToken {
                message: "literal patterns are not allowed at the top level".to_string(),
                span: Some(*span),
            }),
            Pattern::BoolLiteral { span, .. } => Err(ParseError::UnexpectedToken {
                message: "literal patterns are not allowed at the top level".to_string(),
                span: Some(*span),
            }),
            Pattern::Tuple { elements, span } => {
                if elements.is_empty() {
                    return Ok(());
                }
                Err(ParseError::UnexpectedToken {
                    message: "tuple patterns are not allowed at the top level".to_string(),
                    span: Some(*span),
                })
            }
            Pattern::Array { elements, span } => {
                if elements.is_empty() {
                    return Ok(());
                }
                Err(ParseError::UnexpectedToken {
                    message: "array patterns are not allowed at the top level".to_string(),
                    span: Some(*span),
                })
            }
            Pattern::Struct { rest, span, .. } => {
                // Keep the same restriction policy as tuples/arrays for now.
                // Struct destructuring is supported inside functions.
                if rest.is_some() {
                    return Err(ParseError::UnexpectedToken {
                        message: "struct patterns are not allowed at the top level".to_string(),
                        span: Some(*span),
                    });
                }
                Err(ParseError::UnexpectedToken {
                    message: "struct patterns are not allowed at the top level".to_string(),
                    span: Some(*span),
                })
            }
            Pattern::EnumVariant { span, .. } => {
                Err(ParseError::UnexpectedToken {
                    message: "enum patterns are not allowed at the top level".to_string(),
                    span: Some(*span),
                })
            }
        }
    }

    fn parse_type_expr(&mut self) -> Result<TypeExpr, ParseError> {
        let t = self.peek().clone();
        match t.kind {
            TokenKind::LParen => {
                self.advance();
                if matches!(self.peek().kind, TokenKind::RParen) {
                    self.advance();
                    if matches!(self.peek().kind, TokenKind::FatArrow) {
                        self.advance();
                        let ret = self.parse_type_expr()?;
                        return Ok(TypeExpr::Function {
                            params: Vec::new(),
                            ret: Box::new(ret),
                        });
                    }
                    return Ok(TypeExpr::Unit);
                }

                let mut fn_params: Vec<FunctionTypeParam> = Vec::new();
                let mut saw_named = false;
                loop {
                    let mut param_name: Option<String> = None;
                    let mut has_default = false;
                    let ty = if let TokenKind::Identifier(name) = self.peek().kind.clone() {
                        if matches!(self.peek_n(1).kind, TokenKind::Colon) {
                            saw_named = true;
                            param_name = Some(name);
                            self.advance(); // name
                            self.advance(); // :
                            self.parse_type_expr()?
                        } else {
                            self.parse_type_expr()?
                        }
                    } else {
                        self.parse_type_expr()?
                    };

                    if matches!(self.peek().kind, TokenKind::Eq) {
                        has_default = true;
                        self.advance(); // =
                        let _ = self.parse_expression()?;
                    }

                    fn_params.push(FunctionTypeParam {
                        name: param_name,
                        ty,
                        has_default,
                    });

                    match self.peek().kind {
                        TokenKind::RParen => {
                            self.advance();
                            break;
                        }
                        TokenKind::Comma => {
                            self.advance();
                            if matches!(self.peek().kind, TokenKind::RParen) {
                                self.advance();
                                break;
                            }
                        }
                        _ => {
                            return Err(ParseError::UnexpectedToken {
                                message: "expected `,` or `)` in type".to_string(),
                                span: Some(self.peek().span),
                            });
                        }
                    }
                }

                if matches!(self.peek().kind, TokenKind::FatArrow) {
                    self.advance();
                    let ret = self.parse_type_expr()?;
                    return Ok(TypeExpr::Function {
                        params: fn_params,
                        ret: Box::new(ret),
                    });
                }

                if saw_named || fn_params.iter().any(|p| p.has_default) {
                    // Allow omitting lambda return type in type position.
                    return Ok(TypeExpr::Function {
                        params: fn_params,
                        ret: Box::new(TypeExpr::Infer),
                    });
                }

                if fn_params.len() == 1 {
                    Ok(fn_params.remove(0).ty)
                } else {
                    Ok(TypeExpr::Tuple(fn_params.into_iter().map(|p| p.ty).collect()))
                }
            }
            TokenKind::LBracket => {
                self.advance();
                let elem_ty = if matches!(self.peek().kind, TokenKind::Type) {
                    self.advance(); // `type`
                    let (n, _) = self.take_identifier()?;
                    TypeExpr::TypeParam(n)
                } else {
                    self.parse_type_expr()?
                };
                self.expect_rbracket()?;
                Ok(TypeExpr::Array(Box::new(elem_ty)))
            }
            TokenKind::Underscore => {
                self.advance();
                Ok(TypeExpr::Infer)
            }
            TokenKind::Type => {
                self.advance(); // `type`
                let (n, _) = self.take_identifier()?;
                Ok(TypeExpr::TypeParam(n))
            }
            TokenKind::Identifier(_) => {
                let (n, _) = self.take_identifier()?;
                if matches!(self.peek().kind, TokenKind::Lt) {
                    self.advance(); // '<'
                    let mut args = Vec::new();
                    if !matches!(self.peek().kind, TokenKind::Gt) {
                        args.push(self.parse_type_expr()?);
                        while matches!(self.peek().kind, TokenKind::Comma) {
                            self.advance(); // ','
                            args.push(self.parse_type_expr()?);
                        }
                    }
                    if matches!(self.peek().kind, TokenKind::Gt) {
                        self.advance();
                    } else {
                        return Err(ParseError::UnexpectedToken {
                            message: "expected `>` in type argument list".to_string(),
                            span: Some(self.peek().span),
                        });
                    }
                    Ok(TypeExpr::EnumApp { name: n, args })
                } else {
                    Ok(TypeExpr::Named(n))
                }
            }
            _ => Err(ParseError::UnexpectedToken {
                message: "expected type".to_string(),
                span: Some(self.peek().span),
            }),
        }
    }

    fn parse_pattern(&mut self) -> Result<Pattern, ParseError> {
        let t = self.peek().clone();
        match t.kind {
            TokenKind::IntegerLiteral {
                value,
                original,
                radix,
            } => {
                let span = t.span;
                self.advance();
                Ok(Pattern::IntLiteral {
                    value,
                    original,
                    radix,
                    span,
                })
            }
            TokenKind::StringLiteral { value, .. } => {
                let span = t.span;
                self.advance();
                Ok(Pattern::StringLiteral { value, span })
            }
            TokenKind::True => {
                let span = t.span;
                self.advance();
                Ok(Pattern::BoolLiteral { value: true, span })
            }
            TokenKind::False => {
                let span = t.span;
                self.advance();
                Ok(Pattern::BoolLiteral { value: false, span })
            }
            TokenKind::Underscore => {
                let s = t.span;
                self.advance();
                Ok(Pattern::Wildcard { span: s })
            }
            TokenKind::LParen => self.parse_tuple_pattern(),
            TokenKind::LBracket => self.parse_array_pattern(),
            TokenKind::Identifier(_) => {
                let (name, name_span) = self.take_identifier()?;
                if matches!(self.peek().kind, TokenKind::ColonColon) {
                    // Enum variant destructuring: `EnumName::Variant(...)`.
                    let type_args = Vec::new();
                    self.advance(); // `::`
                    let (variant, _variant_span) = self.take_identifier()?;
                    let mut payloads = Vec::new();
                    if matches!(self.peek().kind, TokenKind::LParen) {
                        self.expect_lparen()?;
                        if !matches!(self.peek().kind, TokenKind::RParen) {
                            payloads.push(self.parse_pattern()?);
                            while matches!(self.peek().kind, TokenKind::Comma) {
                                self.advance();
                                payloads.push(self.parse_pattern()?);
                            }
                        }
                        self.expect_rparen()?;
                    }
                    Ok(Pattern::EnumVariant {
                        enum_name: name,
                        enum_name_span: name_span,
                        type_args,
                        variant,
                        payloads,
                        span: name_span,
                    })
                } else if matches!(self.peek().kind, TokenKind::Lt) {
                    // Enum variant destructuring with explicit type args: `EnumName<T>::Variant(...)`.
                    // Also supports generic struct/unit-struct patterns: `Name<T> { ... }` and `Name<T>`.
                    let mut type_args = Vec::new();
                    self.advance(); // `<`
                    if !matches!(self.peek().kind, TokenKind::Gt) {
                        type_args.push(self.parse_type_expr()?);
                        while matches!(self.peek().kind, TokenKind::Comma) {
                            self.advance();
                            type_args.push(self.parse_type_expr()?);
                        }
                    }
                    match self.peek().kind {
                        TokenKind::Gt => {
                            self.advance();
                        }
                        _ => {
                            return Err(ParseError::UnexpectedToken {
                                message: "expected `>` in enum pattern type argument list".to_string(),
                                span: Some(self.peek().span),
                            });
                        }
                    }
                    if matches!(self.peek().kind, TokenKind::ColonColon) {
                        self.advance();
                        let (variant, _variant_span) = self.take_identifier()?;
                        let mut payloads = Vec::new();
                        if matches!(self.peek().kind, TokenKind::LParen) {
                            self.expect_lparen()?;
                            if !matches!(self.peek().kind, TokenKind::RParen) {
                                payloads.push(self.parse_pattern()?);
                                while matches!(self.peek().kind, TokenKind::Comma) {
                                    self.advance();
                                    payloads.push(self.parse_pattern()?);
                                }
                            }
                            self.expect_rparen()?;
                        }
                        Ok(Pattern::EnumVariant {
                            enum_name: name,
                            enum_name_span: name_span,
                            type_args,
                            variant,
                            payloads,
                            span: name_span,
                        })
                    } else if matches!(self.peek().kind, TokenKind::LBrace) {
                        self.parse_struct_pattern_after_name(name, name_span, type_args)
                    } else {
                        // Typed unit-struct pattern: `Name<T>` (no fields).
                        Ok(Pattern::Struct {
                            name,
                            name_span,
                            type_args,
                            fields: Vec::new(),
                            rest: None,
                            span: name_span,
                        })
                    }
                } else if matches!(self.peek().kind, TokenKind::LBrace) {
                    self.parse_struct_pattern_after_name(name, name_span, Vec::new())
                } else {
                    Ok(Pattern::Binding { name, name_span })
                }
            }
            _ => Err(ParseError::UnexpectedToken {
                message: "expected pattern".to_string(),
                span: Some(self.peek().span),
            }),
        }
    }

    fn parse_tuple_pattern(&mut self) -> Result<Pattern, ParseError> {
        let span = self.peek().span;
        self.expect_lparen()?;
        let mut elements = Vec::new();
        if matches!(self.peek().kind, TokenKind::RParen) {
            self.advance();
            return Ok(Pattern::Tuple { elements, span });
        }
        loop {
            if matches!(self.peek().kind, TokenKind::DotDot) {
                let s = self.peek().span;
                self.advance();
                elements.push(PatternElem::Rest(s));
            } else if matches!(self.peek().kind, TokenKind::LParen) {
                elements.push(PatternElem::Pattern(self.parse_tuple_pattern()?));
            } else if matches!(self.peek().kind, TokenKind::Underscore) {
                let s = self.peek().span;
                self.advance();
                elements.push(PatternElem::Pattern(Pattern::Wildcard { span: s }));
            } else if matches!(
                self.peek().kind,
                TokenKind::IntegerLiteral { .. }
                    | TokenKind::StringLiteral { .. }
                    | TokenKind::True
                    | TokenKind::False
            ) {
                elements.push(PatternElem::Pattern(self.parse_pattern()?));
            } else {
                let (name, name_span) = self.take_identifier()?;
                elements.push(PatternElem::Pattern(Pattern::Binding { name, name_span }));
            }
            match self.peek().kind {
                TokenKind::RParen => {
                    self.advance();
                    break;
                }
                TokenKind::Comma => {
                    self.advance();
                    if matches!(self.peek().kind, TokenKind::RParen) {
                        self.advance();
                        break;
                    }
                }
                _ => {
                    return Err(ParseError::UnexpectedToken {
                        message: "expected `,` or `)` in tuple pattern".to_string(),
                        span: Some(self.peek().span),
                    });
                }
            }
        }
        let rest_count = elements
            .iter()
            .filter(|e| matches!(e, PatternElem::Rest(_)))
            .count();
        if rest_count > 1 {
            return Err(ParseError::UnexpectedToken {
                message: "at most one `..` is allowed in a tuple pattern".to_string(),
                span: Some(span),
            });
        }
        Ok(Pattern::Tuple { elements, span })
    }

    fn parse_array_pattern(&mut self) -> Result<Pattern, ParseError> {
        let span = self.peek().span;
        self.expect_lbracket()?;
        let mut elements = Vec::new();
        if matches!(self.peek().kind, TokenKind::RBracket) {
            self.advance();
            return Ok(Pattern::Array { elements, span });
        }
        loop {
            if matches!(self.peek().kind, TokenKind::DotDot) {
                let s = self.peek().span;
                self.advance();
                elements.push(PatternElem::Rest(s));
            } else if matches!(self.peek().kind, TokenKind::LBracket) {
                elements.push(PatternElem::Pattern(self.parse_array_pattern()?));
            } else if matches!(self.peek().kind, TokenKind::LParen) {
                elements.push(PatternElem::Pattern(self.parse_tuple_pattern()?));
            } else if matches!(self.peek().kind, TokenKind::Underscore) {
                let s = self.peek().span;
                self.advance();
                elements.push(PatternElem::Pattern(Pattern::Wildcard { span: s }));
            } else if matches!(
                self.peek().kind,
                TokenKind::IntegerLiteral { .. }
                    | TokenKind::StringLiteral { .. }
                    | TokenKind::True
                    | TokenKind::False
            ) {
                elements.push(PatternElem::Pattern(self.parse_pattern()?));
            } else if matches!(self.peek().kind, TokenKind::Identifier(_)) {
                let (name, name_span) = self.take_identifier()?;
                elements.push(PatternElem::Pattern(Pattern::Binding {
                    name,
                    name_span,
                }));
            } else {
                return Err(ParseError::UnexpectedToken {
                    message: "expected pattern element in array pattern".to_string(),
                    span: Some(self.peek().span),
                });
            }

            match self.peek().kind {
                TokenKind::Comma => {
                    self.advance();
                    if matches!(self.peek().kind, TokenKind::RBracket) {
                        self.advance();
                        break;
                    }
                }
                TokenKind::RBracket => {
                    self.advance();
                    break;
                }
                _ => {
                    return Err(ParseError::UnexpectedToken {
                        message: "expected `,` or `]` in array pattern".to_string(),
                        span: Some(self.peek().span),
                    })
                }
            }
        }
        Ok(Pattern::Array { elements, span })
    }

    fn expect_func_after_async(&mut self) -> Result<(), ParseError> {
        match self.peek().kind.clone() {
            TokenKind::Identifier(ref s) if s == "func" => {
                self.advance();
                Ok(())
            }
            _ => Err(ParseError::UnexpectedToken {
                message: "expected `func` after `async`".to_string(),
                span: Some(self.peek().span),
            }),
        }
    }

    /// `internal func ...` or `internal async func ...`
    fn parse_internal_declaration(&mut self) -> Result<AstNode, ParseError> {
        self.advance(); // `internal`
        match self.peek().kind.clone() {
            TokenKind::Export => {
                // Language items (`struct`/`enum`/`type`) must not use `internal`.
                // `internal export ...` is therefore rejected.
                Err(ParseError::UnexpectedToken {
                    message:
                        "`internal` supports only `func`/`async func` declarations; structs/enums/types must be `export`/non-internal"
                            .to_string(),
                    span: Some(self.peek().span),
                })
            }
            TokenKind::Async => {
                self.advance(); // `async`
                self.expect_func_after_async()?; // consumes `func`
                self.parse_internal_function_declaration(true, true)
            }
            TokenKind::Identifier(ref s) if s == "func" => {
                self.advance(); // `func`
                self.parse_internal_function_declaration(true, false)
            }
            _ => Err(ParseError::UnexpectedToken {
                message: "expected `async func` or `func` after `internal`".to_string(),
                span: Some(self.peek().span),
            }),
        }
    }

    /// `struct Name<T, ...> { field: Type, ... }` or `struct Name<T, ...>;`
    /// When `is_internal`, only unit structs (`;`) are allowed.
    fn parse_struct_definition(&mut self, is_internal: bool) -> Result<AstNode, ParseError> {
        let span = self.peek().span;
        self.advance(); // `struct`
        let (name, name_span) = self.take_identifier()?;
        let type_params = self.parse_optional_type_param_list()?;
        if matches!(self.peek().kind, TokenKind::Semicolon) {
            self.advance();
            return Ok(AstNode::StructDef {
                name,
                type_params,
                fields: Vec::new(),
                is_unit: true,
                is_internal,
                name_span,
                span,
                is_exported: false,
            });
        }
        if is_internal {
            return Err(ParseError::UnexpectedToken {
                message: "`internal struct` must be a unit struct ending with `;`".to_string(),
                span: Some(self.peek().span),
            });
        }
        self.expect_lbrace()?;

        let mut fields = Vec::new();
        while !matches!(self.peek().kind, TokenKind::RBrace) {
            if matches!(self.peek().kind, TokenKind::Eof) {
                return Err(ParseError::UnexpectedEof { expected: "}" });
            }
            if matches!(self.peek().kind, TokenKind::SingleLineComment(_)) {
                self.advance();
                continue;
            }
            if matches!(self.peek().kind, TokenKind::MultiLineComment(_)) {
                self.advance();
                continue;
            }
            let (field_name, field_name_span) = self.take_identifier()?;
            self.expect_colon()?;
            let ty_span = self.peek().span;
            let ty = self.parse_type_expr()?;
            fields.push(crate::ast::StructFieldDecl {
                name: field_name,
                name_span: field_name_span,
                ty,
                ty_span,
            });
            match self.peek().kind {
                TokenKind::Comma => {
                    self.advance();
                    while matches!(
                        self.peek().kind,
                        TokenKind::SingleLineComment(_) | TokenKind::MultiLineComment(_)
                    ) {
                        self.advance();
                    }
                }
                TokenKind::RBrace => {}
                _ => {
                    return Err(ParseError::UnexpectedToken {
                        message: "expected `,` or `}` in struct definition".to_string(),
                        span: Some(self.peek().span),
                    })
                }
            }
        }
        self.expect_rbrace()?;

        Ok(AstNode::StructDef {
            name,
            type_params,
            fields,
            is_unit: false,
            is_internal: false,
            name_span,
            span,
            is_exported: false,
        })
    }

    /// `enum Name<T, U> { Variant, Other(T), ... }`
    fn parse_enum_definition(&mut self, is_internal: bool) -> Result<AstNode, ParseError> {
        let span = self.peek().span;
        self.advance(); // `enum`
        let (name, name_span) = self.take_identifier()?;
        let type_params = self.parse_optional_type_param_list()?;
        self.expect_lbrace()?;

        let mut variants = Vec::new();
        while !matches!(self.peek().kind, TokenKind::RBrace) {
            if matches!(self.peek().kind, TokenKind::Eof) {
                return Err(ParseError::UnexpectedEof { expected: "}" });
            }
            // Allow comments inside enum blocks.
            if matches!(self.peek().kind, TokenKind::SingleLineComment(_)) {
                self.advance();
                continue;
            }
            if matches!(self.peek().kind, TokenKind::MultiLineComment(_)) {
                self.advance();
                continue;
            }

            let (variant_name, variant_name_span) = self.take_identifier()?;
            let mut payload_types = Vec::new();

            if matches!(self.peek().kind, TokenKind::LParen) {
                self.advance(); // `(`
                if !matches!(self.peek().kind, TokenKind::RParen) {
                    payload_types.push(self.parse_type_expr()?);
                    while matches!(self.peek().kind, TokenKind::Comma) {
                        self.advance(); // `,`
                        payload_types.push(self.parse_type_expr()?);
                    }
                }
                self.expect_rparen()?;
            }

            variants.push(crate::ast::EnumVariantDecl {
                name: variant_name,
                name_span: variant_name_span,
                payload_types,
            });

            // Allow trailing comments after a variant declaration.
            while matches!(
                self.peek().kind,
                TokenKind::SingleLineComment(_)
                    | TokenKind::MultiLineComment(_)
            ) {
                self.advance();
            }

            match self.peek().kind {
                TokenKind::Comma => {
                    self.advance();
                    if matches!(self.peek().kind, TokenKind::RBrace) {
                        break;
                    }
                }
                TokenKind::RBrace => break,
                _ => {
                    return Err(ParseError::UnexpectedToken {
                        message: "expected `,` or `}` in enum variant list".to_string(),
                        span: Some(self.peek().span),
                    });
                }
            }
        }

        self.expect_rbrace()?;
        Ok(AstNode::EnumDef {
            name,
            type_params,
            variants,
            is_internal,
            name_span,
            span,
            is_exported: false,
        })
    }

    /// `type Name<T, ...> = TypeExpr;`
    fn parse_type_alias_definition(&mut self) -> Result<AstNode, ParseError> {
        let span = self.peek().span;
        self.advance(); // `type`
        let (name, name_span) = self.take_identifier()?;
        let type_params = self.parse_optional_type_param_list()?;
        self.expect_eq()?;
        let target = self.parse_type_expr()?;
        self.expect_semicolon()?;
        Ok(AstNode::TypeAlias {
            name,
            type_params,
            target,
            name_span,
            span,
            is_exported: false,
        })
    }

    fn parse_struct_pattern_after_name(
        &mut self,
        name: String,
        name_span: Span,
        type_args: Vec<TypeExpr>,
    ) -> Result<Pattern, ParseError> {
        let span = self.peek().span; // `{` span
        self.expect_lbrace()?;

        let mut fields: Vec<crate::ast::StructPatternField> = Vec::new();
        let mut rest: Option<Span> = None;

        if matches!(self.peek().kind, TokenKind::RBrace) {
            self.advance();
            return Ok(Pattern::Struct {
                name,
                name_span,
                type_args,
                fields,
                rest,
                span,
            });
        }

        loop {
            if matches!(self.peek().kind, TokenKind::DotDot) {
                if rest.is_some() {
                    return Err(ParseError::UnexpectedToken {
                        message: "multiple `..` in struct pattern".to_string(),
                        span: Some(self.peek().span),
                    });
                }
                let s = self.peek().span;
                self.advance();
                rest = Some(s);
            } else if matches!(self.peek().kind, TokenKind::Identifier(_)) {
                let (field_name, field_name_span) = self.take_identifier()?;
                let pat = if matches!(self.peek().kind, TokenKind::Colon) {
                    self.advance(); // `:`
                    if matches!(self.peek().kind, TokenKind::Underscore) {
                        let s = self.peek().span;
                        self.advance();
                        Pattern::Wildcard { span: s }
                    } else {
                        self.parse_pattern()?
                    }
                } else {
                    // Shorthand: `x` means `x: x`
                    Pattern::Binding {
                        name: field_name.clone(),
                        name_span: field_name_span,
                    }
                };

                fields.push(crate::ast::StructPatternField {
                    name: field_name,
                    name_span: field_name_span,
                    pattern: pat,
                });
            } else {
                return Err(ParseError::UnexpectedToken {
                    message: "expected struct pattern field or `..`".to_string(),
                    span: Some(self.peek().span),
                });
            }

            match self.peek().kind {
                TokenKind::Comma => {
                    self.advance();
                    if matches!(self.peek().kind, TokenKind::RBrace) {
                        self.advance();
                        break;
                    }
                }
                TokenKind::RBrace => {
                    self.advance();
                    break;
                }
                _ => {
                    return Err(ParseError::UnexpectedToken {
                        message: "expected `,` or `}` in struct pattern".to_string(),
                        span: Some(self.peek().span),
                    });
                }
            }
        }

        Ok(Pattern::Struct {
            name,
            name_span,
            type_args,
            fields,
            rest,
            span,
        })
    }

    fn parse_struct_literal_after_name(
        &mut self,
        name: String,
        type_args: Vec<TypeExpr>,
        span: Span,
    ) -> Result<AstNode, ParseError> {
        self.expect_lbrace()?;
        let mut fields: Vec<(String, AstNode)> = Vec::new();
        let mut update: Option<Box<AstNode>> = None;

        if matches!(self.peek().kind, TokenKind::RBrace) {
            self.advance();
            return Ok(AstNode::StructLiteral {
                name,
                type_args,
                fields,
                update,
                span,
            });
        }

        loop {
            // Allow comments inside struct literals.
            while matches!(
                self.peek().kind,
                TokenKind::SingleLineComment(_)
                    | TokenKind::MultiLineComment(_)
            ) {
                self.advance();
            }
            if matches!(self.peek().kind, TokenKind::DotDot) {
                if update.is_some() {
                    return Err(ParseError::UnexpectedToken {
                        message: "multiple struct update bases (`..`)".to_string(),
                        span: Some(self.peek().span),
                    });
                }
                self.advance(); // `..`
                let base = Box::new(self.parse_expression()?);
                update = Some(base);

                // Rust-like rule: struct update must be last (modulo trailing comma).
                match self.peek().kind {
                    TokenKind::Comma => {
                        self.advance();
                    }
                    TokenKind::RBrace => {}
                    _ => {
                        return Err(ParseError::UnexpectedToken {
                            message: "struct update `..base` must be last".to_string(),
                            span: Some(self.peek().span),
                        });
                    }
                }
                self.expect_rbrace()?;
                break;
            }

            let (field_name, _) = self.take_identifier()?;
            self.expect_colon()?;
            let value = self.parse_expression()?;
            fields.push((field_name, value));

            match self.peek().kind {
                TokenKind::Comma => {
                    self.advance();
                    // Allow trailing comments after a comma.
                    while matches!(
                        self.peek().kind,
                        TokenKind::SingleLineComment(_)
                            | TokenKind::MultiLineComment(_)
                    ) {
                        self.advance();
                    }
                    if matches!(self.peek().kind, TokenKind::RBrace) {
                        self.advance();
                        break;
                    }
                }
                TokenKind::RBrace => {
                    self.advance();
                    break;
                }
                _ => {
                    return Err(ParseError::UnexpectedToken {
                        message: "expected `,` or `}` in struct literal".to_string(),
                        span: Some(self.peek().span),
                    });
                }
            }
        }

        Ok(AstNode::StructLiteral {
            name,
            type_args,
            fields,
            update,
            span,
        })
    }

    /// Pre: already consumed `func` when `leading_func_already_consumed`; otherwise current token is `func`.
    /// `internal async func` path: consumes `func` via caller (`expect_func_after_async`).
    fn parse_internal_function_declaration(
        &mut self,
        leading_func_already_consumed: bool,
        is_async: bool,
    ) -> Result<AstNode, ParseError> {
        if !leading_func_already_consumed {
            match self.peek().kind.clone() {
                TokenKind::Identifier(ref s) if s == "func" => {
                    self.advance();
                }
                _ => {
                    return Err(ParseError::UnexpectedToken {
                        message: "expected `func`".to_string(),
                        span: Some(self.peek().span),
                    });
                }
            }
        }
        // Support extension-receiver internal functions:
        //   internal func <ReceiverType>::<method><T...>(self, ...): Ret;
        let checkpoint = self.position;
        if let Ok(receiver_ty) = self.parse_type_expr() {
            if matches!(self.peek().kind, TokenKind::ColonColon) {
                self.advance(); // `::`
                let (method_name, method_name_span) = self.take_identifier()?;
                let type_params = self.parse_optional_type_param_list()?;
                self.expect_lparen()?;
                let params = self.parse_parameter_list(Some(&receiver_ty))?;
                self.expect_rparen()?;
                if params.iter().any(|p| p.default_value.is_some()) {
                    return Err(ParseError::UnexpectedToken {
                        message: "internal functions cannot have parameters with default values".to_string(),
                        span: Some(method_name_span),
                    });
                }
                let return_type = self.parse_optional_return_type_expr()?;
                self.expect_semicolon()?;

                let Some(receiver_key) = Self::type_expr_receiver_key(&receiver_ty) else {
                    return Err(ParseError::UnexpectedToken {
                        message: "extension receiver type must be concrete".to_string(),
                        span: Some(method_name_span),
                    });
                };

                let name = format!("{}::{}", receiver_key, method_name);
                return Ok(AstNode::InternalFunction {
                    name,
                    type_params,
                    params,
                    return_type,
                    name_span: method_name_span,
                    is_exported: false,
                    is_async,
                });
            }
        }

        // Fallback: regular internal function syntax.
        self.position = checkpoint;

        let (name, name_span) = self.take_identifier()?;
        let type_params = self.parse_optional_type_param_list()?;
        self.expect_lparen()?;
        let params = self.parse_parameter_list(None)?;
        self.expect_rparen()?;
        if params.iter().any(|p| p.default_value.is_some()) {
            return Err(ParseError::UnexpectedToken {
                message: "internal functions cannot have parameters with default values".to_string(),
                span: Some(name_span),
            });
        }
        let return_type = self.parse_optional_return_type_expr()?;
        self.expect_semicolon()?;
        Ok(AstNode::InternalFunction {
            name,
            type_params,
            params,
            return_type,
            name_span,
            is_exported: false,
            is_async,
        })
    }

    /// `func name(params) { body }` or `func name(params): Type { body }`.
    /// When `is_async` is true, `async` and `func` were already consumed by the caller.
    fn parse_function_definition(&mut self, is_async: bool) -> Result<AstNode, ParseError> {
        if !is_async {
            self.advance(); // `func`
        }
        let mut extension_receiver: Option<ExtensionReceiver> = None;
        let (mut name, name_span) = {
            let checkpoint = self.position;
            if matches!(
                self.peek().kind,
                TokenKind::Identifier(_) | TokenKind::LParen | TokenKind::LBracket
            ) {
                if let Ok(receiver_ty) = self.parse_type_expr() {
                    if matches!(self.peek().kind, TokenKind::ColonColon) {
                        self.advance(); // ::
                        let (method_name, method_span) = self.take_identifier()?;
                        extension_receiver = Some(ExtensionReceiver {
                            ty: receiver_ty,
                            method_name: method_name.clone(),
                        });
                        (method_name, method_span)
                    } else {
                        self.position = checkpoint;
                        self.take_identifier()?
                    }
                } else {
                    self.position = checkpoint;
                    self.take_identifier()?
                }
            } else {
                self.take_identifier()?
            }
        };
        if let Some(ext) = extension_receiver.as_ref() {
            let Some(rcv) = Self::type_expr_receiver_key(&ext.ty) else {
                return Err(ParseError::UnexpectedToken {
                    message: "extension receiver type must be a concrete type usable at compile time"
                        .to_string(),
                    span: Some(name_span),
                });
            };
            name = format!("{rcv}::{name}");
        }
        let type_params = self.parse_optional_type_param_list()?;
        self.expect_lparen()?;
        let params = self.parse_parameter_list(extension_receiver.as_ref().map(|r| &r.ty))?;
        self.expect_rparen()?;
        let return_type = self.parse_optional_return_type_expr()?;
        // Short-hand: `func f(args) = expr;`
        if matches!(self.peek().kind, TokenKind::Eq) {
            let arrow_span = self.peek().span;
            self.advance(); // `=`
            let expr = self.parse_expression()?;
            let semi_tok = self.peek().clone();
            self.expect_semicolon()?;

            let body = vec![AstNode::Return {
                value: Some(Box::new(expr)),
                span: arrow_span,
            }];

            let closing_span = semi_tok.span;

            Ok(AstNode::Function {
                name,
                extension_receiver: extension_receiver.clone(),
                type_params,
                params,
                return_type,
                body,
                name_span,
                closing_span,
                is_exported: false,
                is_async,
            })
        } else if matches!(self.peek().kind, TokenKind::FatArrow) {
            Err(ParseError::UnexpectedToken {
                message: "function shorthand now uses `=`; use `func name(args) = expr;`"
                    .to_string(),
                span: Some(self.peek().span),
            })
        } else {
            self.expect_lbrace()?;
            let body = self.parse_block_items_until_rbrace()?;
            let closing_tok = self.peek().clone();
            self.expect_rbrace()?;
            let closing_span = closing_tok.span;

            Ok(AstNode::Function {
                name,
                extension_receiver,
                type_params,
                params,
                return_type,
                body,
                name_span,
                closing_span,
                is_exported: false,
                is_async,
            })
        }
    }

    fn parse_optional_type_param_list(&mut self) -> Result<Vec<GenericParam>, ParseError> {
        if !matches!(self.peek().kind, TokenKind::Lt) {
            return Ok(Vec::new());
        }
        self.advance(); // '<'
        let mut out = Vec::new();
        let mut saw_default = false;
        loop {
            let (n, n_span) = self.take_identifier()?;
            let default = if matches!(self.peek().kind, TokenKind::Eq) {
                saw_default = true;
                self.advance();
                Some(self.parse_type_expr()?)
            } else {
                if saw_default {
                    return Err(ParseError::UnexpectedToken {
                        message: "generic parameters without defaults must come before parameters with defaults"
                            .to_string(),
                        span: Some(self.peek().span),
                    });
                }
                None
            };
            out.push(GenericParam {
                name: n,
                name_span: n_span,
                default,
            });
            if matches!(self.peek().kind, TokenKind::Comma) {
                self.advance();
                continue;
            }
            if matches!(self.peek().kind, TokenKind::Gt) {
                self.advance();
                break;
            }
            return Err(ParseError::UnexpectedToken {
                message: "expected `,` or `>` in generic parameter list".to_string(),
                span: Some(self.peek().span),
            });
        }
        Ok(out)
    }

    fn parse_optional_return_type_expr(&mut self) -> Result<Option<TypeExpr>, ParseError> {
        if matches!(self.peek().kind, TokenKind::Colon) {
            self.advance();
            Ok(Some(self.parse_type_expr()?))
        } else {
            Ok(None)
        }
    }

    fn parse_parameter_list(
        &mut self,
        ext_receiver_ty: Option<&TypeExpr>,
    ) -> Result<Vec<Param>, ParseError> {
        let mut params = Vec::new();
        let mut seen_default = false;
        let mut seen_params = false;
        if matches!(self.peek().kind, TokenKind::RParen) {
            return Ok(params);
        }
        loop {
            let is_params = if matches!(self.peek().kind, TokenKind::Params) {
                self.advance();
                seen_params = true;
                true
            } else {
                false
            };
            let (name, name_span, is_wildcard, implicit_self) = match self.peek().kind {
                TokenKind::Underscore => {
                    let s = self.peek().span;
                    self.advance();
                    ("_".to_string(), s, true, false)
                }
                TokenKind::Identifier(_) => {
                    let (n, sp) = self.take_identifier()?;
                    if params.is_empty()
                        && ext_receiver_ty.is_some()
                        && n == "self"
                        && !matches!(self.peek().kind, TokenKind::Colon)
                    {
                        (n, sp, false, true)
                    } else {
                        (n, sp, false, false)
                    }
                }
                _ => {
                    return Err(ParseError::UnexpectedToken {
                        message: "expected parameter name or `_`".to_string(),
                        span: Some(self.peek().span),
                    });
                }
            };
            let ty = if implicit_self {
                ext_receiver_ty
                    .expect("checked implicit self precondition")
                    .clone()
            } else {
                self.expect_colon()?;
                self.parse_type_expr()?
            };
            if is_params {
                if !matches!(ty, TypeExpr::Array(_)) {
                    return Err(ParseError::UnexpectedToken {
                        message: "`params` parameter must have array type `[T]`".to_string(),
                        span: Some(name_span),
                    });
                }
            } else if seen_params {
                return Err(ParseError::UnexpectedToken {
                    message: "`params` parameter must be the last parameter".to_string(),
                    span: Some(name_span),
                });
            }
            let default_value = if matches!(self.peek().kind, TokenKind::Eq) {
                self.advance();
                seen_default = true;
                Some(Box::new(self.parse_expression()?))
            } else {
                if seen_default {
                    return Err(ParseError::UnexpectedToken {
                        message: "parameters without default value cannot follow defaulted parameters"
                            .to_string(),
                        span: Some(self.peek().span),
                    });
                }
                None
            };
            params.push(Param {
                name,
                name_span,
                is_wildcard,
                is_params,
                ty,
                default_value,
            });
            match self.peek().kind {
                TokenKind::Comma => {
                    self.advance();
                    if matches!(self.peek().kind, TokenKind::RParen) {
                        return Err(ParseError::UnexpectedToken {
                            message: "trailing comma in parameter list".to_string(),
                            span: Some(self.peek().span),
                        });
                    }
                }
                TokenKind::RParen => break,
                _ => {
                    return Err(ParseError::UnexpectedToken {
                        message: "expected `,` or `)`".to_string(),
                        span: Some(self.peek().span),
                    });
                }
            }
        }
        Ok(params)
    }

    fn parse_block_items_until_rbrace(&mut self) -> Result<Vec<AstNode>, ParseError> {
        let mut items = Vec::new();
        while !matches!(self.peek().kind, TokenKind::RBrace) {
            if matches!(self.peek().kind, TokenKind::Eof) {
                return Err(ParseError::UnexpectedEof { expected: "}" });
            }
            items.push(self.parse_block_item()?);
        }
        Ok(items)
    }

    fn parse_block_item(&mut self) -> Result<AstNode, ParseError> {
        let token = self.peek().clone();
        match token.kind {
            TokenKind::Identifier(ref s) if s == "internal" => Err(ParseError::UnexpectedToken {
                message: "`internal` functions must be declared at the top level and must have no body (use `internal func name(args);`)".to_string(),
                span: Some(self.peek().span),
            }),
            TokenKind::Identifier(ref s) if s == "func" => Err(ParseError::UnexpectedToken {
                message: "function definitions are top-level only".to_string(),
                span: Some(self.peek().span),
            }),
            TokenKind::Async => Err(ParseError::UnexpectedToken {
                message: "`async` functions must be declared at the top level".to_string(),
                span: Some(self.peek().span),
            }),
            TokenKind::Await => {
                let e = self.parse_expression()?;
                self.expect_semicolon()?;
                Ok(e)
            },
            TokenKind::Struct => Err(ParseError::UnexpectedToken {
                message: "`struct` declarations are top-level only".to_string(),
                span: Some(self.peek().span),
            }),
            TokenKind::Enum => Err(ParseError::UnexpectedToken {
                message: "`enum` declarations are top-level only".to_string(),
                span: Some(self.peek().span),
            }),
            TokenKind::Import | TokenKind::Export => Err(ParseError::UnexpectedToken {
                message: "imports/exports are top-level only".to_string(),
                span: Some(self.peek().span),
            }),
            TokenKind::Identifier(ref s) if s == "return" => self.parse_return_statement(),
            TokenKind::Let => self.parse_let_statement(false, false),
            TokenKind::Const => self.parse_const_statement(false),
            TokenKind::If => self.parse_if_statement(),
            TokenKind::Match => {
                let m = self.parse_match_expression()?;
                if matches!(self.peek().kind, TokenKind::Semicolon) {
                    self.advance();
                }
                Ok(m)
            }
            TokenKind::While => self.parse_while_statement(),
            TokenKind::Break => {
                let span = self.peek().span;
                self.advance();
                self.expect_semicolon()?;
                Ok(AstNode::Break { span })
            }
            TokenKind::Continue => {
                let span = self.peek().span;
                self.advance();
                self.expect_semicolon()?;
                Ok(AstNode::Continue { span })
            }
            TokenKind::LBrace => self.parse_block_statement(),
            TokenKind::LBracket | TokenKind::RBracket => Err(ParseError::UnexpectedToken {
                message: "unexpected token; expected a statement".to_string(),
                span: Some(self.peek().span),
            }),
            TokenKind::FatArrow => Err(ParseError::UnexpectedToken {
                message: "unexpected `=>`".to_string(),
                span: Some(self.peek().span),
            }),
            TokenKind::LParen => {
                let open_span = token.span;
                let pat = self.parse_pattern()?;
                self.expect_eq()?;
                let value = Box::new(self.parse_expression()?);
                self.expect_semicolon()?;
                Ok(AstNode::Assign {
                    pattern: pat,
                    value,
                    span: open_span,
                })
            }
            TokenKind::SingleLineComment(text) => {
                self.advance();
                Ok(AstNode::SingleLineComment(text))
            }
            TokenKind::MultiLineComment(text) => {
                self.advance();
                Ok(AstNode::MultiLineComment(text))
            }
            TokenKind::Identifier(name) => {
                let name = name.clone();
                let tgt_span = token.span;
                self.advance();
                if matches!(self.peek().kind, TokenKind::LParen) {
                    return self.finish_call_after_callee(name, tgt_span);
                }
                if matches!(self.peek().kind, TokenKind::Lt) {
                    if !self.lt_starts_generic_args() {
                        return Err(ParseError::UnexpectedToken {
                            message: "unexpected `<` after identifier; expected call `(`".to_string(),
                            span: Some(self.peek().span),
                        });
                    }

                    self.advance(); // '<'
                    let mut type_args = Vec::new();
                    loop {
                        type_args.push(self.parse_type_expr()?);
                        if matches!(self.peek().kind, TokenKind::Comma) {
                            self.advance();
                            continue;
                        }
                        if matches!(self.peek().kind, TokenKind::Gt) {
                            self.advance();
                            break;
                        }
                        return Err(ParseError::UnexpectedToken {
                            message: "expected `,` or `>` in generic type arguments".to_string(),
                            span: Some(self.peek().span),
                        });
                    }

                    self.expect_lparen()?;
                    let arguments = self.parse_call_argument_list()?;
                    self.expect_rparen()?;
                    self.expect_semicolon()?;
                    return Ok(AstNode::Call {
                        callee: name,
                        type_args,
                        arguments,
                        span: tgt_span,
                    });
                }
                let lhs = self.parse_tuple_field_suffix(AstNode::Identifier {
                    name,
                    span: tgt_span,
                })?;
                if let Some(op) = self.compound_op_from_peek() {
                    let op = op;
                    self.advance();
                    let rhs = Box::new(self.parse_expression()?);
                    self.expect_semicolon()?;
                    return Ok(AstNode::CompoundAssign {
                        lhs: Box::new(lhs),
                        op,
                        rhs,
                        span: tgt_span,
                    });
                }
                match self.peek().kind {
                    TokenKind::Eq => {
                        self.advance();
                        let rhs = Box::new(self.parse_expression()?);
                        self.expect_semicolon()?;
                        Ok(match lhs {
                            AstNode::Identifier { name, span } => AstNode::Assign {
                                pattern: Pattern::Binding { name, name_span: span },
                                value: rhs,
                                span,
                            },
                            _ => AstNode::AssignExpr {
                                lhs: Box::new(lhs),
                                rhs,
                                span: tgt_span,
                            },
                        })
                    }
                    _ => Err(ParseError::UnexpectedToken {
                        message: "expected `=`, compound assignment (`+=`, …), or `(` after identifier"
                            .to_string(),
                        span: Some(self.peek().span),
                    }),
                }
            }
            TokenKind::IntegerLiteral { .. }
            | TokenKind::FloatLiteral { .. }
            | TokenKind::StringLiteral { .. } => {
                Err(ParseError::UnexpectedToken {
                    message: "unexpected literal; expected a statement".to_string(),
                    span: Some(self.peek().span),
                })
            }
            TokenKind::RParen
            | TokenKind::RBrace
            | TokenKind::Colon
            | TokenKind::Semicolon
            | TokenKind::Comma
            | TokenKind::Plus
            | TokenKind::Minus
            | TokenKind::Star
            | TokenKind::Slash
            | TokenKind::Percent
            | TokenKind::Tilde
            | TokenKind::Eq
            | TokenKind::EqEq
            | TokenKind::Ne
            | TokenKind::Lt
            | TokenKind::Gt
            | TokenKind::Le
            | TokenKind::Ge
            | TokenKind::ShiftLeft
            | TokenKind::ShiftRight
            | TokenKind::AndAnd
            | TokenKind::AndEq
            | TokenKind::Amp
            | TokenKind::OrOr
            | TokenKind::OrEq
            | TokenKind::Pipe
            | TokenKind::Caret
            | TokenKind::CaretEq
            | TokenKind::PlusEq
            | TokenKind::MinusEq
            | TokenKind::StarEq
            | TokenKind::SlashEq
            | TokenKind::PercentEq
            | TokenKind::ShiftLeftEq
            | TokenKind::ShiftRightEq
            | TokenKind::Bang
            | TokenKind::Dot
            | TokenKind::DotDot
            | TokenKind::ColonColon
            | TokenKind::From
            | TokenKind::As => Err(ParseError::UnexpectedToken {
                message: "unexpected punctuation".to_string(),
                span: Some(self.peek().span),
            }),
            TokenKind::Underscore
            | TokenKind::True
            | TokenKind::False
            | TokenKind::Else
            | TokenKind::Params
            | TokenKind::Type => Err(ParseError::UnexpectedToken {
                message: "unexpected token; expected a statement".to_string(),
                span: Some(self.peek().span),
            }),
            TokenKind::Eof => Err(ParseError::UnexpectedEof { expected: "}" }),
        }
    }

    fn parse_if_statement(&mut self) -> Result<AstNode, ParseError> {
        let span = self.peek().span;
        self.advance(); // `if`
        if matches!(self.peek().kind, TokenKind::Let) {
            // `if let <pattern> = <expr> { ... } else { ... }`
            self.advance(); // `let`
            let pattern = self.parse_pattern()?;
            self.expect_eq()?;
            let value = Box::new(self.parse_expression()?);
            self.expect_lbrace()?;
            let then_body = self.parse_block_items_until_rbrace()?;
            self.expect_rbrace()?;

            let else_body = if matches!(self.peek().kind, TokenKind::Else) {
                self.advance(); // `else`
                if matches!(self.peek().kind, TokenKind::If) {
                    let nested_if = self.parse_if_statement()?;
                    Some(vec![nested_if])
                } else {
                    self.expect_lbrace()?;
                    let b = self.parse_block_items_until_rbrace()?;
                    self.expect_rbrace()?;
                    Some(b)
                }
            } else {
                None
            };

            Ok(AstNode::IfLet {
                pattern,
                value,
                then_body,
                else_body,
                span,
            })
        } else {
            let condition = Box::new(self.parse_expression()?);
            self.expect_lbrace()?;
            let then_body = self.parse_block_items_until_rbrace()?;
            self.expect_rbrace()?;
            let else_body = if matches!(self.peek().kind, TokenKind::Else) {
                self.advance(); // `else`

                if matches!(self.peek().kind, TokenKind::If) {
                    let nested_if = self.parse_if_statement()?;
                    Some(vec![nested_if])
                } else {
                    self.expect_lbrace()?;
                    let b = self.parse_block_items_until_rbrace()?;
                    self.expect_rbrace()?;
                    Some(b)
                }
            } else {
                None
            };
            Ok(AstNode::If {
                condition,
                then_body,
                else_body,
                span,
            })
        }
    }

    fn parse_while_statement(&mut self) -> Result<AstNode, ParseError> {
        let span = self.peek().span;
        self.advance(); // `while`
        let condition = Box::new(self.parse_expression()?);
        self.expect_lbrace()?;
        let body = self.parse_block_items_until_rbrace()?;
        self.expect_rbrace()?;
        Ok(AstNode::While {
            condition,
            body,
            span,
        })
    }

    fn compound_op_from_peek(&self) -> Option<CompoundOp> {
        match self.peek().kind {
            TokenKind::PlusEq => Some(CompoundOp::Add),
            TokenKind::MinusEq => Some(CompoundOp::Sub),
            TokenKind::StarEq => Some(CompoundOp::Mul),
            TokenKind::SlashEq => Some(CompoundOp::Div),
            TokenKind::PercentEq => Some(CompoundOp::Mod),
            TokenKind::AndEq => Some(CompoundOp::BitAnd),
            TokenKind::OrEq => Some(CompoundOp::BitOr),
            TokenKind::CaretEq => Some(CompoundOp::BitXor),
            TokenKind::ShiftLeftEq => Some(CompoundOp::ShiftLeft),
            TokenKind::ShiftRightEq => Some(CompoundOp::ShiftRight),
            _ => None,
        }
    }

    fn parse_block_statement(&mut self) -> Result<AstNode, ParseError> {
        self.advance(); // `{`
        let body = self.parse_block_items_until_rbrace()?;
        let closing_tok = self.peek().clone();
        self.expect_rbrace()?;
        Ok(AstNode::Block {
            body,
            closing_span: closing_tok.span,
        })
    }

    fn parse_return_statement(&mut self) -> Result<AstNode, ParseError> {
        let ret_span = self.peek().span;
        self.advance(); // `return`
        if matches!(self.peek().kind, TokenKind::Semicolon) {
            self.advance();
            return Ok(AstNode::Return {
                value: None,
                span: ret_span,
            });
        }
        let value = self.parse_expression()?;
        self.expect_semicolon()?;
        Ok(AstNode::Return {
            value: Some(Box::new(value)),
            span: ret_span,
        })
    }

    fn parse_call_statement(&mut self) -> Result<AstNode, ParseError> {
        let (callee, span) = self.take_identifier()?;
        if matches!(self.peek().kind, TokenKind::Lt) {
            if !self.lt_starts_generic_args() {
                return Err(ParseError::UnexpectedToken {
                    message: "expected call argument list after generic arguments".to_string(),
                    span: Some(self.peek().span),
                });
            }

            self.advance(); // '<'
            let mut type_args = Vec::new();
            loop {
                type_args.push(self.parse_type_expr()?);
                if matches!(self.peek().kind, TokenKind::Comma) {
                    self.advance();
                    continue;
                }
                if matches!(self.peek().kind, TokenKind::Gt) {
                    self.advance();
                    break;
                }
                return Err(ParseError::UnexpectedToken {
                    message: "expected `,` or `>` in generic type arguments".to_string(),
                    span: Some(self.peek().span),
                });
            }

            self.expect_lparen()?;
            let arguments = self.parse_call_argument_list()?;
            self.expect_rparen()?;
            self.expect_semicolon()?;
            return Ok(AstNode::Call {
                callee,
                type_args,
                arguments,
                span,
            });
        }

        self.finish_call_after_callee(callee, span)
    }

    fn finish_call_after_callee(
        &mut self,
        callee: String,
        span: Span,
    ) -> Result<AstNode, ParseError> {
        self.expect_lparen()?;
        let arguments = self.parse_call_argument_list()?;
        self.expect_rparen()?;
        self.expect_semicolon()?;
        Ok(AstNode::Call {
            callee,
            type_args: Vec::new(),
            arguments,
            span,
        })
    }

    fn parse_call_argument_list(&mut self) -> Result<Vec<CallArg>, ParseError> {
        let mut args = Vec::new();
        if matches!(self.peek().kind, TokenKind::RParen) {
            return Ok(args);
        }
        loop {
            if let TokenKind::Identifier(_) = self.peek().kind {
                if matches!(self.peek_n(1).kind, TokenKind::Colon) {
                    let (name, name_span) = self.take_identifier()?;
                    self.expect_colon()?;
                    let value = self.parse_expression()?;
                    args.push(CallArg::Named {
                        name,
                        name_span,
                        value,
                    });
                } else {
                    args.push(CallArg::Positional(self.parse_expression()?));
                }
            } else {
                args.push(CallArg::Positional(self.parse_expression()?));
            }
            match self.peek().kind {
                TokenKind::RParen => break,
                TokenKind::Comma => {
                    self.advance();
                    if matches!(self.peek().kind, TokenKind::RParen) {
                        return Err(ParseError::UnexpectedToken {
                            message: "trailing comma in argument list".to_string(),
                            span: Some(self.peek().span),
                        });
                    }
                }
                _ => {
                    return Err(ParseError::UnexpectedToken {
                        message: "expected `,` or `)`".to_string(),
                        span: Some(self.peek().span),
                    });
                }
            }
        }
        Ok(args)
    }

    fn parse_expression_list(&mut self) -> Result<Vec<AstNode>, ParseError> {
        let mut args = Vec::new();
        if matches!(self.peek().kind, TokenKind::RParen) {
            return Ok(args);
        }
        loop {
            args.push(self.parse_expression()?);
            match self.peek().kind {
                TokenKind::RParen => break,
                TokenKind::Comma => {
                    self.advance();
                    if matches!(self.peek().kind, TokenKind::RParen) {
                        return Err(ParseError::UnexpectedToken {
                            message: "trailing comma in argument list".to_string(),
                            span: Some(self.peek().span),
                        });
                    }
                }
                _ => {
                    return Err(ParseError::UnexpectedToken {
                        message: "expected `,` or `)`".to_string(),
                        span: Some(self.peek().span),
                    });
                }
            }
        }
        Ok(args)
    }

    fn parse_expression(&mut self) -> Result<AstNode, ParseError> {
        if let Some(lambda) = self.try_parse_lambda_expression()? {
            return Ok(lambda);
        }
        self.parse_or_expression()
    }

    fn try_parse_lambda_expression(&mut self) -> Result<Option<AstNode>, ParseError> {
        let saved = self.position;
        let span = self.peek().span;

        if let TokenKind::Identifier(name) = self.peek().kind.clone() {
            if matches!(self.peek_n(1).kind, TokenKind::FatArrow) {
                self.advance(); // param
                self.advance(); // =>
                let body = if matches!(self.peek().kind, TokenKind::LBrace) {
                    self.advance();
                    let body = self.parse_block_items_until_rbrace()?;
                    self.expect_rbrace()?;
                    LambdaBody::Block(body)
                } else {
                    LambdaBody::Expr(self.parse_expression()?)
                };
                return Ok(Some(AstNode::Lambda {
                    params: vec![LambdaParam {
                        name,
                        name_span: span,
                    }],
                    body: Box::new(body),
                    span,
                }));
            }
        }

        self.position = saved;
        if !matches!(self.peek().kind, TokenKind::LParen) {
            return Ok(None);
        }
        self.advance(); // (
        let mut params: Vec<LambdaParam> = Vec::new();
        if !matches!(self.peek().kind, TokenKind::RParen) {
            loop {
                if !matches!(self.peek().kind, TokenKind::Identifier(_)) {
                    self.position = saved;
                    return Ok(None);
                }
                let (name, name_span) = self.take_identifier()?;
                params.push(LambdaParam { name, name_span });
                if matches!(self.peek().kind, TokenKind::Comma) {
                    self.advance();
                    continue;
                }
                break;
            }
        }
        if !matches!(self.peek().kind, TokenKind::RParen) {
            self.position = saved;
            return Ok(None);
        }
        self.advance(); // )
        if !matches!(self.peek().kind, TokenKind::FatArrow) {
            self.position = saved;
            return Ok(None);
        }
        self.advance(); // =>
        let body = if matches!(self.peek().kind, TokenKind::LBrace) {
            self.advance();
            let body = self.parse_block_items_until_rbrace()?;
            self.expect_rbrace()?;
            LambdaBody::Block(body)
        } else {
            LambdaBody::Expr(self.parse_expression()?)
        };
        Ok(Some(AstNode::Lambda {
            params,
            body: Box::new(body),
            span,
        }))
    }

    fn parse_or_expression(&mut self) -> Result<AstNode, ParseError> {
        let mut left = self.parse_and_expression()?;
        loop {
            if !matches!(self.peek().kind, TokenKind::OrOr) {
                break;
            }
            let op_span = self.peek().span;
            self.advance();
            let right = self.parse_and_expression()?;
            left = AstNode::BinaryOp {
                left: Box::new(left),
                op: BinaryOp::Or,
                right: Box::new(right),
                span: op_span,
            };
        }
        Ok(left)
    }

    fn parse_and_expression(&mut self) -> Result<AstNode, ParseError> {
        let mut left = self.parse_bit_or_expression()?;
        loop {
            if !matches!(self.peek().kind, TokenKind::AndAnd) {
                break;
            }
            let op_span = self.peek().span;
            self.advance();
            let right = self.parse_bit_or_expression()?;
            left = AstNode::BinaryOp {
                left: Box::new(left),
                op: BinaryOp::And,
                right: Box::new(right),
                span: op_span,
            };
        }
        Ok(left)
    }

    fn parse_bit_or_expression(&mut self) -> Result<AstNode, ParseError> {
        let mut left = self.parse_bit_xor_expression()?;
        loop {
            if !matches!(self.peek().kind, TokenKind::Pipe) {
                break;
            }
            let op_span = self.peek().span;
            self.advance();
            let right = self.parse_bit_xor_expression()?;
            left = AstNode::BinaryOp {
                left: Box::new(left),
                op: BinaryOp::BitOr,
                right: Box::new(right),
                span: op_span,
            };
        }
        Ok(left)
    }

    fn parse_bit_xor_expression(&mut self) -> Result<AstNode, ParseError> {
        let mut left = self.parse_bit_and_expression()?;
        loop {
            if !matches!(self.peek().kind, TokenKind::Caret) {
                break;
            }
            let op_span = self.peek().span;
            self.advance();
            let right = self.parse_bit_and_expression()?;
            left = AstNode::BinaryOp {
                left: Box::new(left),
                op: BinaryOp::BitXor,
                right: Box::new(right),
                span: op_span,
            };
        }
        Ok(left)
    }

    fn parse_bit_and_expression(&mut self) -> Result<AstNode, ParseError> {
        let mut left = self.parse_equality_expression()?;
        loop {
            if !matches!(self.peek().kind, TokenKind::Amp) {
                break;
            }
            let op_span = self.peek().span;
            self.advance();
            let right = self.parse_equality_expression()?;
            left = AstNode::BinaryOp {
                left: Box::new(left),
                op: BinaryOp::BitAnd,
                right: Box::new(right),
                span: op_span,
            };
        }
        Ok(left)
    }

    fn parse_equality_expression(&mut self) -> Result<AstNode, ParseError> {
        let mut left = self.parse_comparison_expression()?;
        loop {
            let op = match self.peek().kind {
                TokenKind::EqEq => Some(BinaryOp::Eq),
                TokenKind::Ne => Some(BinaryOp::Ne),
                _ => None,
            };
            let Some(op) = op else {
                break;
            };
            let op_span = self.peek().span;
            self.advance();
            let right = self.parse_comparison_expression()?;
            left = AstNode::BinaryOp {
                left: Box::new(left),
                op,
                right: Box::new(right),
                span: op_span,
            };
        }
        Ok(left)
    }

    fn parse_comparison_expression(&mut self) -> Result<AstNode, ParseError> {
        let mut left = self.parse_shift_expression()?;
        loop {
            let op = match self.peek().kind {
                TokenKind::Lt => Some(BinaryOp::Lt),
                TokenKind::Gt => Some(BinaryOp::Gt),
                TokenKind::Le => Some(BinaryOp::Le),
                TokenKind::Ge => Some(BinaryOp::Ge),
                _ => None,
            };
            let Some(op) = op else {
                break;
            };
            let op_span = self.peek().span;
            self.advance();
            let right = self.parse_shift_expression()?;
            left = AstNode::BinaryOp {
                left: Box::new(left),
                op,
                right: Box::new(right),
                span: op_span,
            };
        }
        Ok(left)
    }

    fn parse_shift_expression(&mut self) -> Result<AstNode, ParseError> {
        let mut left = self.parse_additive_expression()?;
        loop {
            let op = match self.peek().kind {
                TokenKind::ShiftLeft => Some(BinaryOp::ShiftLeft),
                TokenKind::ShiftRight => Some(BinaryOp::ShiftRight),
                _ => None,
            };
            let Some(op) = op else {
                break;
            };
            let op_span = self.peek().span;
            self.advance();
            let right = self.parse_additive_expression()?;
            left = AstNode::BinaryOp {
                left: Box::new(left),
                op,
                right: Box::new(right),
                span: op_span,
            };
        }
        Ok(left)
    }

    fn parse_additive_expression(&mut self) -> Result<AstNode, ParseError> {
        let mut left = self.parse_multiplicative_expression()?;
        loop {
            let op = match self.peek().kind {
                TokenKind::Plus => Some(BinaryOp::Add),
                TokenKind::Minus => Some(BinaryOp::Sub),
                _ => None,
            };
            let Some(op) = op else {
                break;
            };
            let op_span = self.peek().span;
            self.advance();
            let right = self.parse_multiplicative_expression()?;
            left = AstNode::BinaryOp {
                left: Box::new(left),
                op,
                right: Box::new(right),
                span: op_span,
            };
        }
        Ok(left)
    }

    fn parse_multiplicative_expression(&mut self) -> Result<AstNode, ParseError> {
        let mut left = self.parse_unary_expression()?;
        loop {
            let op = match self.peek().kind {
                TokenKind::Star => Some(BinaryOp::Mul),
                TokenKind::Slash => Some(BinaryOp::Div),
                TokenKind::Percent => Some(BinaryOp::Mod),
                _ => None,
            };
            let Some(op) = op else {
                break;
            };
            let op_span = self.peek().span;
            self.advance();
            let right = self.parse_unary_expression()?;
            left = AstNode::BinaryOp {
                left: Box::new(left),
                op,
                right: Box::new(right),
                span: op_span,
            };
        }
        Ok(left)
    }

    fn parse_unary_expression(&mut self) -> Result<AstNode, ParseError> {
        match self.peek().kind {
            TokenKind::Plus => {
                let span = self.peek().span;
                self.advance();
                let operand = self.parse_unary_expression()?;
                Ok(AstNode::UnaryOp {
                    op: UnaryOp::Plus,
                    operand: Box::new(operand),
                    span,
                })
            }
            TokenKind::Minus => {
                let span = self.peek().span;
                self.advance();
                let operand = self.parse_unary_expression()?;
                Ok(AstNode::UnaryOp {
                    op: UnaryOp::Minus,
                    operand: Box::new(operand),
                    span,
                })
            }
            TokenKind::Tilde => {
                let span = self.peek().span;
                self.advance();
                let operand = self.parse_unary_expression()?;
                Ok(AstNode::UnaryOp {
                    op: UnaryOp::BitNot,
                    operand: Box::new(operand),
                    span,
                })
            }
            TokenKind::Bang => {
                let span = self.peek().span;
                self.advance();
                let operand = self.parse_unary_expression()?;
                Ok(AstNode::UnaryOp {
                    op: UnaryOp::Not,
                    operand: Box::new(operand),
                    span,
                })
            }
            TokenKind::Await => {
                let span = self.peek().span;
                self.advance();
                let operand = self.parse_unary_expression()?;
                Ok(AstNode::Await {
                    expr: Box::new(operand),
                    span,
                })
            }
            _ => {
                let e = self.parse_primary_expression()?;
                self.parse_tuple_field_suffix(e)
            }
        }
    }

    fn parse_tuple_field_suffix(&mut self, mut e: AstNode) -> Result<AstNode, ParseError> {
        while matches!(
            self.peek().kind,
            TokenKind::Dot | TokenKind::LBracket | TokenKind::LParen
        ) {
            match self.peek().kind {
                TokenKind::Dot => {
                    self.advance();
                    let t = self.peek().clone();
                    match t.kind {
                        TokenKind::IntegerLiteral {
                            original, radix, ..
                        } => {
                            let span = t.span;
                            self.advance();
                            let digits = if radix == 10 {
                                original.replace('_', "")
                            } else if original.len() >= 2 {
                                original[2..].replace('_', "")
                            } else {
                                original.replace('_', "")
                            };
                            let index = u32::from_str_radix(&digits, radix).map_err(|_| {
                                ParseError::UnexpectedToken {
                                    message: "tuple index after `.` must fit in u32".to_string(),
                                    span: Some(span),
                                }
                            })?;
                            e = AstNode::TupleField {
                                base: Box::new(e),
                                index,
                                span,
                            };
                        }
                        TokenKind::Identifier(field_name) => {
                            let span = t.span;
                            self.advance();
                            e = AstNode::FieldAccess {
                                base: Box::new(e),
                                field: field_name,
                                span,
                            };
                        }
                        _ => {
                            return Err(ParseError::UnexpectedToken {
                                message: "expected tuple index after `.`".to_string(),
                                span: Some(self.peek().span),
                            });
                        }
                    }
                }
                TokenKind::LBracket => {
                    let open_span = self.peek().span;
                    self.advance();
                    let idx = self.parse_expression()?;
                    self.expect_rbracket()?;
                    e = AstNode::ArrayIndex {
                        base: Box::new(e),
                        index: Box::new(idx),
                        span: open_span,
                    };
                }
                TokenKind::LParen => {
                    let call_span = self.peek().span;
                    self.advance();
                    let arguments = self.parse_call_argument_list()?;
                    self.expect_rparen()?;
                    match e {
                        AstNode::Identifier { name, .. } => {
                            e = AstNode::Call {
                                callee: name,
                                type_args: Vec::new(),
                                arguments,
                                span: call_span,
                            };
                        }
                        AstNode::FieldAccess { base, field, .. } => {
                            e = AstNode::MethodCall {
                                receiver: base,
                                method: field,
                                arguments,
                                span: call_span,
                            };
                        }
                        other => {
                            e = AstNode::Invoke {
                                callee: Box::new(other),
                                arguments,
                                span: call_span,
                            };
                        }
                    }
                }
                _ => unreachable!(),
            }
        }
        Ok(e)
    }

    fn parse_primary_expression(&mut self) -> Result<AstNode, ParseError> {
        let token = self.peek().clone();
        match token.kind {
            TokenKind::Match => self.parse_match_expression(),
            TokenKind::LBrace => {
                // Dict literal: `{ key: value, key2: value2, ... }`
                // Used as an expression (e.g. `let d: Dict<K,V> = { ... };`).
                let span = token.span;
                self.advance(); // `{`

                let mut entries: Vec<(AstNode, AstNode)> = Vec::new();
                while matches!(
                    self.peek().kind,
                    TokenKind::SingleLineComment(_) | TokenKind::MultiLineComment(_)
                ) {
                    self.advance();
                }

                if matches!(self.peek().kind, TokenKind::RBrace) {
                    self.advance();
                    return Ok(AstNode::DictLiteral { entries, span });
                }

                loop {
                    while matches!(
                        self.peek().kind,
                        TokenKind::SingleLineComment(_) | TokenKind::MultiLineComment(_)
                    ) {
                        self.advance();
                    }

                    let key = self.parse_expression()?;
                    self.expect_colon()?;
                    let value = self.parse_expression()?;
                    entries.push((key, value));

                    while matches!(
                        self.peek().kind,
                        TokenKind::SingleLineComment(_) | TokenKind::MultiLineComment(_)
                    ) {
                        self.advance();
                    }

                    match self.peek().kind {
                        TokenKind::Comma => {
                            self.advance();
                            while matches!(
                                self.peek().kind,
                                TokenKind::SingleLineComment(_) | TokenKind::MultiLineComment(_)
                            ) {
                                self.advance();
                            }
                            if matches!(self.peek().kind, TokenKind::RBrace) {
                                self.advance();
                                break;
                            }
                            continue;
                        }
                        TokenKind::RBrace => {
                            self.advance();
                            break;
                        }
                        _ => {
                            return Err(ParseError::UnexpectedToken {
                                message: "expected `,` or `}` in dict literal".to_string(),
                                span: Some(self.peek().span),
                            })
                        }
                    }
                }

                Ok(AstNode::DictLiteral { entries, span })
            }
            TokenKind::StringLiteral { value, original } => {
                let span = token.span;
                self.advance();
                Ok(AstNode::StringLiteral {
                    value,
                    original,
                    span,
                })
            }
            TokenKind::IntegerLiteral {
                value,
                original,
                radix,
            } => {
                let span = token.span;
                self.advance();
                Ok(AstNode::IntegerLiteral {
                    value,
                    original,
                    radix,
                    span,
                })
            }
            TokenKind::FloatLiteral { original, cleaned } => {
                let span = token.span;
                self.advance();
                Ok(AstNode::FloatLiteral {
                    original,
                    cleaned,
                    span,
                })
            }
            TokenKind::True => {
                let span = token.span;
                self.advance();
                Ok(AstNode::BoolLiteral { value: true, span })
            }
            TokenKind::False => {
                let span = token.span;
                self.advance();
                Ok(AstNode::BoolLiteral { value: false, span })
            }
            TokenKind::Identifier(name) => {
                let span = token.span;
                self.advance();
                if matches!(self.peek().kind, TokenKind::LParen) {
                    self.expect_lparen()?;
                    let arguments = self.parse_call_argument_list()?;
                    self.expect_rparen()?;
                    Ok(AstNode::Call {
                        callee: name,
                        type_args: Vec::new(),
                        arguments,
                        span,
                    })
                } else if matches!(self.peek().kind, TokenKind::Lt) {
                    // Generic-call syntax: `callee<type_args>(args...)`.
                    // Heuristic disambiguation: only treat `<...>` as generics if
                    // the first token after `<` can start a type expression.
                    if !self.lt_starts_generic_args() {
                        return Ok(AstNode::Identifier { name, span });
                    }

                    self.advance(); // '<'
                    let mut type_args = Vec::new();
                    loop {
                        type_args.push(self.parse_type_expr()?);
                        if matches!(self.peek().kind, TokenKind::Comma) {
                            self.advance();
                            continue;
                        }
                        if matches!(self.peek().kind, TokenKind::Gt) {
                            self.advance();
                            break;
                        }
                        return Err(ParseError::UnexpectedToken {
                            message: "expected `,` or `>` in generic type arguments".to_string(),
                            span: Some(self.peek().span),
                        });
                    }

                    // Either:
                    // - generic call: `callee<type_args>(args...)`
                    // - enum constructor: `EnumTypeExpr<type_args>::Variant(...)`
                    // - struct literal: `Struct<type_args> { ... }`
                    // - unit-struct type value: `UnitStruct<type_args>`
                    if matches!(self.peek().kind, TokenKind::ColonColon) {
                        self.advance(); // `::`
                        let (variant, _variant_span) = self.take_identifier()?;
                        let mut payloads = Vec::new();
                        if matches!(self.peek().kind, TokenKind::LParen) {
                            self.expect_lparen()?;
                            payloads = self.parse_expression_list()?;
                            self.expect_rparen()?;
                        }
                        Ok(AstNode::EnumVariantCtor {
                            enum_name: name,
                            type_args,
                            variant,
                            payloads,
                            span,
                        })
                    } else if matches!(self.peek().kind, TokenKind::LBrace) {
                        self.parse_struct_literal_after_name(name, type_args, span)
                    } else if matches!(self.peek().kind, TokenKind::LParen) {
                        self.expect_lparen()?;
                        let arguments = self.parse_call_argument_list()?;
                        self.expect_rparen()?;
                        Ok(AstNode::Call {
                            callee: name,
                            type_args,
                            arguments,
                            span,
                        })
                    } else {
                        let mut parts = Vec::new();
                        for a in &type_args {
                            parts.push(Self::type_expr_receiver_key(a).ok_or(
                                ParseError::UnexpectedToken {
                                    message:
                                        "type value arguments must be concrete compile-time types"
                                            .to_string(),
                                    span: Some(span),
                                },
                            )?);
                        }
                        Ok(AstNode::TypeValue {
                            type_name: format!("{}<{}>", name, parts.join(", ")),
                            span,
                        })
                    }
                } else if matches!(self.peek().kind, TokenKind::LBrace) {
                    // Disambiguate from control-flow blocks like `if cond { ... }`.
                    // Treat `{` as a struct-literal only when the first token inside
                    // looks like empty `{}`, `..base`, or `field: <expr>`.
                    let after_lbrace = self.peek_n(1).kind.clone();
                    let starts_struct_lit = matches!(after_lbrace, TokenKind::RBrace)
                        || matches!(after_lbrace, TokenKind::DotDot)
                        || (matches!(after_lbrace, TokenKind::Identifier(_))
                            && matches!(self.peek_n(2).kind, TokenKind::Colon));

                    if starts_struct_lit {
                        self.parse_struct_literal_after_name(name, Vec::new(), span)
                    } else {
                        Ok(AstNode::Identifier { name, span })
                    }
                } else if matches!(self.peek().kind, TokenKind::ColonColon) {
                    // Enum variant constructor: `EnumName::Variant(...)`.
                    self.advance(); // `::`
                    let (variant, _variant_span) = self.take_identifier()?;
                    if matches!(self.peek().kind, TokenKind::LParen) {
                        self.expect_lparen()?;
                        let arguments = self.parse_call_argument_list()?;
                        self.expect_rparen()?;
                        return Ok(AstNode::TypeMethodCall {
                            type_name: name,
                            method: variant,
                            arguments,
                            span,
                        });
                    }
                    let mut payloads = Vec::new();
                    if matches!(self.peek().kind, TokenKind::LParen) {
                        self.expect_lparen()?;
                        payloads = self.parse_expression_list()?;
                        self.expect_rparen()?;
                    }
                    Ok(AstNode::EnumVariantCtor {
                        enum_name: name,
                        type_args: Vec::new(),
                        variant,
                        payloads,
                        span,
                    })
                } else {
                    Ok(AstNode::Identifier { name, span })
                }
            }
            TokenKind::LParen => {
                let open = self.peek().span;
                self.advance();
                if matches!(self.peek().kind, TokenKind::RParen) {
                    self.advance();
                    return Ok(AstNode::UnitLiteral { span: open });
                }
                let first = self.parse_expression()?;
                if matches!(self.peek().kind, TokenKind::RParen) {
                    self.advance();
                    return Ok(first);
                }
                if matches!(self.peek().kind, TokenKind::Comma) {
                    self.advance();
                    let mut elements = vec![first];
                    loop {
                        if matches!(self.peek().kind, TokenKind::RParen) {
                            self.advance();
                            break;
                        }
                        elements.push(self.parse_expression()?);
                        match self.peek().kind {
                            TokenKind::RParen => {
                                self.advance();
                                break;
                            }
                            TokenKind::Comma => {
                                self.advance();
                                if matches!(self.peek().kind, TokenKind::RParen) {
                                    self.advance();
                                    break;
                                }
                            }
                            _ => {
                                return Err(ParseError::UnexpectedToken {
                                    message: "expected `,` or `)` in tuple".to_string(),
                                    span: Some(self.peek().span),
                                });
                            }
                        }
                    }
                    return Ok(AstNode::TupleLiteral {
                        elements,
                        span: open,
                    });
                }
                Err(ParseError::UnexpectedToken {
                    message: "expected `,` or `)` after expression".to_string(),
                    span: Some(self.peek().span),
                })
            }
            TokenKind::LBracket => {
                let open = self.peek().span;
                self.advance();
                let mut elements = Vec::new();
                if matches!(self.peek().kind, TokenKind::RBracket) {
                    self.advance();
                    return Ok(AstNode::ArrayLiteral { elements, span: open });
                }
                loop {
                    if matches!(self.peek().kind, TokenKind::RBracket) {
                        self.advance();
                        break;
                    }
                    elements.push(self.parse_expression()?);
                    match self.peek().kind {
                        TokenKind::Comma => {
                            self.advance();
                            if matches!(self.peek().kind, TokenKind::RBracket) {
                                self.advance();
                                break;
                            }
                        }
                        TokenKind::RBracket => {
                            self.advance();
                            break;
                        }
                        _ => {
                            return Err(ParseError::UnexpectedToken {
                                message: "expected `,` or `]` in array literal".to_string(),
                                span: Some(self.peek().span),
                            });
                        }
                    }
                }
                Ok(AstNode::ArrayLiteral { elements, span: open })
            }
            _ => Err(ParseError::UnexpectedToken {
                message: "expected expression".to_string(),
                span: Some(self.peek().span),
            }),
        }
    }

    fn parse_match_expression(&mut self) -> Result<AstNode, ParseError> {
        let span = self.peek().span;
        self.advance(); // `match`
        let scrutinee = Box::new(self.parse_expression()?);
        self.expect_lbrace()?;

        let mut arms: Vec<crate::ast::MatchArm> = Vec::new();
        while !matches!(self.peek().kind, TokenKind::RBrace) {
            if matches!(self.peek().kind, TokenKind::Eof) {
                return Err(ParseError::UnexpectedEof { expected: "match arms `}`" });
            }

            while matches!(
                self.peek().kind,
                TokenKind::SingleLineComment(_) | TokenKind::MultiLineComment(_)
            ) {
                self.advance();
            }

            let arm_span = self.peek().span;

            // Patterns (allow `pat1 | pat2 | ...`).
            let mut patterns = Vec::new();
            patterns.push(self.parse_pattern()?);
            while matches!(self.peek().kind, TokenKind::Pipe) {
                self.advance();
                patterns.push(self.parse_pattern()?);
            }

            // Optional guard: `pat if <expr> => ...`.
            let guard = if matches!(self.peek().kind, TokenKind::If) {
                self.advance(); // `if`
                Some(Box::new(self.parse_expression()?))
            } else {
                None
            };

            // `=>`
            match self.peek().kind {
                TokenKind::FatArrow => {
                    self.advance();
                }
                _ => {
                    return Err(ParseError::UnexpectedToken {
                        message: "expected `=>` in match arm".to_string(),
                        span: Some(self.peek().span),
                    });
                }
            }

            // Arm body is a general expression; blocks evaluate to `()`.
            let body = if matches!(self.peek().kind, TokenKind::LBrace) {
                Box::new(self.parse_block_statement()?)
            } else {
                Box::new(self.parse_expression()?)
            };

            // Allow comments between an arm body and the separator (` , ` or `}`).
            while matches!(
                self.peek().kind,
                TokenKind::SingleLineComment(_) | TokenKind::MultiLineComment(_)
            ) {
                self.advance();
            }

            // Optional comma between arms.
            match self.peek().kind {
                TokenKind::Comma => {
                    self.advance();
                    while matches!(
                        self.peek().kind,
                        TokenKind::SingleLineComment(_) | TokenKind::MultiLineComment(_)
                    ) {
                        self.advance();
                    }
                    // Trailing comma is allowed; we finish the `match` loop based on the
                    // next token at the top-level `while` condition.
                }
                TokenKind::RBrace => {}
                _ => {
                    return Err(ParseError::UnexpectedToken {
                        message: "expected `,` or `}` after match arm".to_string(),
                        span: Some(self.peek().span),
                    })
                }
            }

            arms.push(crate::ast::MatchArm {
                patterns,
                guard,
                body,
                span: arm_span,
            });
        }

        self.expect_rbrace()?;
        Ok(AstNode::Match {
            scrutinee,
            arms,
            span,
        })
    }

    fn take_identifier(&mut self) -> Result<(String, Span), ParseError> {
        let t = self.peek().clone();
        match t.kind {
            TokenKind::Identifier(s) => {
                self.advance();
                Ok((s, t.span))
            }
            _ => Err(ParseError::UnexpectedToken {
                message: "expected identifier".to_string(),
                span: Some(self.peek().span),
            }),
        }
    }

    fn expect_colon(&mut self) -> Result<(), ParseError> {
        match self.peek().kind {
            TokenKind::Colon => {
                self.advance();
                Ok(())
            }
            _ => Err(ParseError::UnexpectedToken {
                message: "expected `:`".to_string(),
                span: Some(self.peek().span),
            }),
        }
    }

    fn expect_semicolon(&mut self) -> Result<(), ParseError> {
        match self.peek().kind {
            TokenKind::Semicolon => {
                self.advance();
                Ok(())
            }
            _ => Err(ParseError::UnexpectedToken {
                message: "expected `;`".to_string(),
                span: Some(self.peek().span),
            }),
        }
    }

    fn expect_lparen(&mut self) -> Result<(), ParseError> {
        match self.peek().kind {
            TokenKind::LParen => {
                self.advance();
                Ok(())
            }
            _ => Err(ParseError::UnexpectedToken {
                message: "expected `(`".to_string(),
                span: Some(self.peek().span),
            }),
        }
    }

    fn expect_lbracket(&mut self) -> Result<(), ParseError> {
        match self.peek().kind {
            TokenKind::LBracket => {
                self.advance();
                Ok(())
            }
            _ => Err(ParseError::UnexpectedToken {
                message: "expected `[`".to_string(),
                span: Some(self.peek().span),
            }),
        }
    }

    fn expect_rparen(&mut self) -> Result<(), ParseError> {
        match self.peek().kind {
            TokenKind::RParen => {
                self.advance();
                Ok(())
            }
            _ => Err(ParseError::UnexpectedToken {
                message: "expected `)`".to_string(),
                span: Some(self.peek().span),
            }),
        }
    }

    fn expect_rbracket(&mut self) -> Result<(), ParseError> {
        match self.peek().kind {
            TokenKind::RBracket => {
                self.advance();
                Ok(())
            }
            _ => Err(ParseError::UnexpectedToken {
                message: "expected `]`".to_string(),
                span: Some(self.peek().span),
            }),
        }
    }

    fn expect_lbrace(&mut self) -> Result<(), ParseError> {
        match self.peek().kind {
            TokenKind::LBrace => {
                self.advance();
                Ok(())
            }
            _ => Err(ParseError::UnexpectedToken {
                message: "expected `{`".to_string(),
                span: Some(self.peek().span),
            }),
        }
    }

    fn expect_rbrace(&mut self) -> Result<(), ParseError> {
        match self.peek().kind {
            TokenKind::RBrace => {
                self.advance();
                Ok(())
            }
            _ => Err(ParseError::UnexpectedToken {
                message: "expected `}`".to_string(),
                span: Some(self.peek().span),
            }),
        }
    }

    fn expect_eq(&mut self) -> Result<(), ParseError> {
        match self.peek().kind {
            TokenKind::Eq => {
                self.advance();
                Ok(())
            }
            _ => Err(ParseError::UnexpectedToken {
                message: "expected `=`".to_string(),
                span: Some(self.peek().span),
            }),
        }
    }

    fn peek(&self) -> &Token {
        if self.position < self.tokens.len() {
            &self.tokens[self.position]
        } else {
            &EOF_SENTINEL
        }
    }

    fn peek_n(&self, n: usize) -> &Token {
        let idx = self.position + n;
        if idx < self.tokens.len() {
            &self.tokens[idx]
        } else {
            &EOF_SENTINEL
        }
    }

    fn advance(&mut self) {
        if self.position < self.tokens.len() {
            self.position += 1;
        }
    }

    fn is_at_end(&self) -> bool {
        matches!(self.peek().kind, TokenKind::Eof)
    }

    /// Parse a single type expression from a source fragment (used by semantic for `Type::method` resolution).
    pub fn parse_type_expr_from_source(source: &str) -> Result<TypeExpr, ParseError> {
        let mut lexer = Lexer::new(source);
        let tokens = lexer.tokenize().map_err(|e| ParseError::UnexpectedToken {
            message: e.message,
            span: Some(e.span),
        })?;
        let mut parser = Parser::new(tokens);
        let ty = parser.parse_type_expr()?;
        if !parser.is_at_end() {
            return Err(ParseError::UnexpectedToken {
                message: "expected end of type expression".to_string(),
                span: Some(parser.peek().span),
            });
        }
        Ok(ty)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::TypeExpr;
    use crate::error::Span;
    use crate::lexer::Lexer;

    fn named_type_name(te: &TypeExpr) -> Option<&str> {
        match te {
            TypeExpr::Named(s) => Some(s.as_str()),
            _ => None,
        }
    }

    #[test]
    fn test_parse_single_line_comment() {
        let mut lexer = Lexer::new("// comment");
        let tokens = lexer.tokenize().unwrap();
        let mut parser = Parser::new(tokens);
        let ast = parser.parse().unwrap();

        match ast {
            AstNode::Program(nodes) => {
                assert_eq!(nodes.len(), 1);
                assert!(matches!(nodes[0], AstNode::SingleLineComment(_)));
            }
            _ => panic!("Expected Program node"),
        }
    }

    #[test]
    fn test_parse_multi_line_comment() {
        let mut lexer = Lexer::new("/* comment */");
        let tokens = lexer.tokenize().unwrap();
        let mut parser = Parser::new(tokens);
        let ast = parser.parse().unwrap();

        match ast {
            AstNode::Program(nodes) => {
                assert_eq!(nodes.len(), 1);
                assert!(matches!(nodes[0], AstNode::MultiLineComment(_)));
            }
            _ => panic!("Expected Program node"),
        }
    }

    #[test]
    fn test_parse_mixed_comments() {
        let source = "// First\n/* Second */\n// Third";
        let mut lexer = Lexer::new(source);
        let tokens = lexer.tokenize().unwrap();
        let mut parser = Parser::new(tokens);
        let ast = parser.parse().unwrap();

        match ast {
            AstNode::Program(nodes) => {
                assert_eq!(nodes.len(), 3);
            }
            _ => panic!("Expected Program node"),
        }
    }

    #[test]
    fn test_parse_decimal_integer() {
        let mut lexer = Lexer::new("42");
        let tokens = lexer.tokenize().unwrap();
        let mut parser = Parser::new(tokens);
        let ast = parser.parse().unwrap();

        match ast {
            AstNode::Program(nodes) => {
                assert_eq!(nodes.len(), 1);
                assert!(matches!(
                    nodes[0],
                    AstNode::IntegerLiteral {
                        value: 42,
                        ref original,
                        radix: 10,
                        ..
                    } if original == "42"
                ));
            }
            _ => panic!("Expected Program node"),
        }
    }

    #[test]
    fn test_parse_float_literal() {
        let mut lexer = Lexer::new("12_34_5.78_90");
        let tokens = lexer.tokenize().unwrap();
        let mut parser = Parser::new(tokens);
        let ast = parser.parse().unwrap();

        match ast {
            AstNode::Program(nodes) => {
                assert_eq!(nodes.len(), 1);
                assert!(matches!(
                    nodes[0],
                    AstNode::FloatLiteral {
                        ref original,
                        ref cleaned,
                        ..
                    } if original == "12_34_5.78_90" && cleaned == "12345.7890"
                ));
            }
            _ => panic!("Expected Program node"),
        }
    }

    #[test]
    fn parse_export_internal_enum_marks_export_and_internal() {
        let src = r#"export internal enum Method { Get }"#;
        let mut lexer = Lexer::new(src);
        let tokens = lexer.tokenize().unwrap();
        let mut parser = Parser::new(tokens);
        parser
            .parse()
            .expect_err("internal enums are no longer supported; only internal functions/async funcs");
    }

    #[test]
    fn parse_internal_export_enum_marks_export_and_internal() {
        let src = r#"internal export enum Method { Get }"#;
        let mut lexer = Lexer::new(src);
        let tokens = lexer.tokenize().unwrap();
        let mut parser = Parser::new(tokens);
        parser
            .parse()
            .expect_err("internal export enum declarations must be rejected");
    }

    #[test]
    fn test_parse_binary_integer() {
        let mut lexer = Lexer::new("0b1010");
        let tokens = lexer.tokenize().unwrap();
        let mut parser = Parser::new(tokens);
        let ast = parser.parse().unwrap();

        match ast {
            AstNode::Program(nodes) => {
                assert_eq!(nodes.len(), 1);
                assert!(matches!(
                    nodes[0],
                    AstNode::IntegerLiteral {
                        value: 10,
                        ref original,
                        radix: 2,
                        ..
                    } if original == "0b1010"
                ));
            }
            _ => panic!("Expected Program node"),
        }
    }

    #[test]
    fn test_parse_octal_integer() {
        let mut lexer = Lexer::new("0o755");
        let tokens = lexer.tokenize().unwrap();
        let mut parser = Parser::new(tokens);
        let ast = parser.parse().unwrap();

        match ast {
            AstNode::Program(nodes) => {
                assert_eq!(nodes.len(), 1);
                assert!(matches!(
                    nodes[0],
                    AstNode::IntegerLiteral {
                        value: 493,
                        ref original,
                        radix: 8,
                        ..
                    } if original == "0o755"
                ));
            }
            _ => panic!("Expected Program node"),
        }
    }

    #[test]
    fn test_parse_string_literal() {
        let mut lexer = Lexer::new(r#""Hello, world!""#);
        let tokens = lexer.tokenize().unwrap();
        let mut parser = Parser::new(tokens);
        let ast = parser.parse().unwrap();

        match ast {
            AstNode::Program(nodes) => {
                assert_eq!(nodes.len(), 1);
                match &nodes[0] {
                    AstNode::StringLiteral {
                        value,
                        original,
                        ..
                    } => {
                        assert_eq!(value, "Hello, world!");
                        assert_eq!(original, "\"Hello, world!\"");
                    }
                    _ => panic!("expected StringLiteral"),
                }
            }
            _ => panic!("Expected Program node"),
        }
    }

    #[test]
    fn test_parse_example0_2() {
        let src = include_str!("../examples/basics/example0_2.vc");
        let mut lexer = Lexer::new(src);
        let tokens = lexer.tokenize().expect("lex example0_2.vc");
        let mut parser = Parser::new(tokens);
        let ast = parser.parse().expect("parse example0_2.vc");
        let AstNode::Program(nodes) = ast else {
            panic!("expected Program");
        };
        let strings = nodes
            .iter()
            .filter(|n| matches!(n, AstNode::StringLiteral { .. }))
            .count();
        assert!(strings >= 10, "expected many string nodes, got {}", strings);
    }

    #[test]
    fn test_parse_strings_and_comments() {
        let src = r#""a" // c
"b""#;
        let mut lexer = Lexer::new(src);
        let tokens = lexer.tokenize().unwrap();
        let mut parser = Parser::new(tokens);
        let ast = parser.parse().unwrap();
        match ast {
            AstNode::Program(nodes) => {
                assert_eq!(nodes.len(), 3);
                assert!(matches!(
                    nodes[0],
                    AstNode::StringLiteral {
                        ref value,
                        ..
                    } if value == "a"
                ));
                assert!(matches!(nodes[1], AstNode::SingleLineComment(_)));
                assert!(matches!(
                    nodes[2],
                    AstNode::StringLiteral {
                        ref value,
                        ..
                    } if value == "b"
                ));
            }
            _ => panic!("Expected Program node"),
        }
    }

    #[test]
    fn test_parse_hex_integer() {
        let mut lexer = Lexer::new("0xdeadbeef");
        let tokens = lexer.tokenize().unwrap();
        let mut parser = Parser::new(tokens);
        let ast = parser.parse().unwrap();

        match ast {
            AstNode::Program(nodes) => {
                assert_eq!(nodes.len(), 1);
                assert!(matches!(
                    nodes[0],
                    AstNode::IntegerLiteral {
                        value: 0xdeadbeef,
                        ref original,
                        radix: 16,
                        ..
                    } if original == "0xdeadbeef"
                ));
            }
            _ => panic!("Expected Program node"),
        }
    }

    #[test]
    fn test_parse_error_display() {
        let err = ParseError::UnexpectedEof {
            expected: "statement",
        };
        assert_eq!(
            err.to_string(),
            "parse error: unexpected end of file, expected statement"
        );
        assert_eq!(
            err.format_with_file("foo.vc"),
            "foo.vc: parse error: unexpected end of file, expected statement"
        );
    }

    #[test]
    fn test_parse_error_unexpected_token_with_span() {
        let span = Span::new(3, 5, 1);
        let err = ParseError::UnexpectedToken {
            message: "bad token".to_string(),
            span: Some(span),
        };
        assert_eq!(err.to_string(), "3:5: parse error: bad token");
        assert_eq!(
            err.format_with_file("x.vc"),
            "x.vc:3:5: parse error: bad token"
        );
    }

    #[test]
    fn test_parse_mixed_literals() {
        let source = "42 0b1010 0xaa /* comment */";
        let mut lexer = Lexer::new(source);
        let tokens = lexer.tokenize().unwrap();
        let mut parser = Parser::new(tokens);
        let ast = parser.parse().unwrap();

        match ast {
            AstNode::Program(nodes) => {
                assert_eq!(nodes.len(), 4);
                assert!(matches!(
                    nodes[0],
                    AstNode::IntegerLiteral {
                        value: 42,
                        radix: 10,
                        ..
                    }
                ));
                assert!(matches!(
                    nodes[1],
                    AstNode::IntegerLiteral {
                        value: 10,
                        radix: 2,
                        ..
                    }
                ));
                assert!(matches!(
                    nodes[2],
                    AstNode::IntegerLiteral {
                        value: 170,
                        radix: 16,
                        ..
                    }
                ));
                assert!(matches!(nodes[3], AstNode::MultiLineComment(_)));
            }
            _ => panic!("Expected Program node"),
        }
    }

    #[test]
    fn test_parse_func_main_empty_body() {
        let mut lexer = Lexer::new("func main() {}");
        let tokens = lexer.tokenize().unwrap();
        let mut parser = Parser::new(tokens);
        let ast = parser.parse().unwrap();
        match ast {
            AstNode::Program(nodes) => {
                assert_eq!(nodes.len(), 1);
                match &nodes[0] {
                    AstNode::Function {
                        name,
                        params,
                        return_type,
                        body,
                        ..
                    } => {
                        assert_eq!(name, "main");
                        assert!(params.is_empty());
                        assert!(return_type.is_none());
                        assert!(body.is_empty());
                    }
                    _ => panic!("Expected Function"),
                }
            }
            _ => panic!("Expected Program"),
        }
    }

    #[test]
    fn test_parse_let_assign_var_and_block() {
        let src = "func main() { let x: Int; x = 1; let y = 2; { let z = 3; } }";
        let mut lexer = Lexer::new(src);
        let tokens = lexer.tokenize().unwrap();
        let mut parser = Parser::new(tokens);
        let ast = parser.parse().unwrap();
        let AstNode::Program(nodes) = ast else {
            panic!("expected Program");
        };
        let AstNode::Function { body, .. } = &nodes[0] else {
            panic!("expected Function");
        };
        assert!(matches!(body[0], AstNode::Let { .. }));
        assert!(matches!(body[1], AstNode::Assign { .. }));
        assert!(matches!(body[2], AstNode::Let { .. }));
        assert!(matches!(body[3], AstNode::Block { .. }));
    }

    #[test]
    fn test_parse_if_optional_else() {
        let src = "func main() { if 1 > 0 { let x: Int = 1; } }";
        let mut lexer = Lexer::new(src);
        let tokens = lexer.tokenize().unwrap();
        let mut parser = Parser::new(tokens);
        let ast = parser.parse().unwrap();
        let AstNode::Program(nodes) = ast else {
            panic!("Program");
        };
        let AstNode::Function { body, .. } = &nodes[0] else {
            panic!("Function");
        };
        let AstNode::If {
            then_body,
            else_body,
            ..
        } = &body[0]
        else {
            panic!("If");
        };
        assert!(else_body.is_none());
        assert_eq!(then_body.len(), 1);
    }

    #[test]
    fn test_parse_if_else_blocks() {
        let src = "func main() { if false { } else { } }";
        let mut lexer = Lexer::new(src);
        let tokens = lexer.tokenize().unwrap();
        let mut parser = Parser::new(tokens);
        let ast = parser.parse().unwrap();
        let AstNode::Program(nodes) = ast else {
            panic!("Program");
        };
        let AstNode::Function { body, .. } = &nodes[0] else {
            panic!("Function");
        };
        let AstNode::If { else_body, .. } = &body[0] else {
            panic!("If");
        };
        assert!(else_body.is_some());
    }

    #[test]
    fn test_parse_if_else_if_chain() {
        let src = "func main() { if false { let x: Int = 1; } else if true { let y: Int = 2; } }";
        let mut lexer = Lexer::new(src);
        let tokens = lexer.tokenize().unwrap();
        let mut parser = Parser::new(tokens);
        let ast = parser.parse().unwrap();

        let AstNode::Program(nodes) = ast else {
            panic!("Program");
        };
        let AstNode::Function { body, .. } = &nodes[0] else {
            panic!("Function");
        };

        let AstNode::If {
            then_body: _,
            else_body,
            ..
        } = &body[0]
        else {
            panic!("If");
        };

        let else_body = else_body.as_ref().expect("else_body");
        assert_eq!(else_body.len(), 1);

        // `else if ...` should be parsed as `else { if ... { ... } }`
        let AstNode::If { else_body: nested_else, .. } = &else_body[0] else {
            panic!("nested if");
        };

        assert!(nested_else.is_none());
    }

    #[test]
    fn test_parse_if_else_if_with_final_else() {
        let src = "func main() { if false { } else if true { } else { } }";
        let mut lexer = Lexer::new(src);
        let tokens = lexer.tokenize().unwrap();
        let mut parser = Parser::new(tokens);
        let ast = parser.parse().unwrap();

        let AstNode::Program(nodes) = ast else {
            panic!("Program");
        };
        let AstNode::Function { body, .. } = &nodes[0] else {
            panic!("Function");
        };

        let AstNode::If { else_body, .. } = &body[0] else {
            panic!("If");
        };

        let else_body = else_body.as_ref().expect("else_body");
        assert_eq!(else_body.len(), 1);

        let AstNode::If { else_body: nested_else, .. } = &else_body[0] else {
            panic!("nested if");
        };

        assert!(nested_else.is_some());
    }

    #[test]
    fn test_parse_while_break_continue_compound_assign() {
        let src = "func main() { while true { break; continue; } let a: Int = 1; a += 2; }";
        let mut lexer = Lexer::new(src);
        let tokens = lexer.tokenize().unwrap();
        let mut parser = Parser::new(tokens);
        let ast = parser.parse().unwrap();
        let AstNode::Program(nodes) = ast else {
            panic!("Program");
        };
        let AstNode::Function { body, .. } = &nodes[0] else {
            panic!("Function");
        };
        assert!(matches!(body[0], AstNode::While { .. }));
        assert!(matches!(body[1], AstNode::Let { .. }));
        let AstNode::CompoundAssign { op, .. } = &body[2] else {
            panic!("expected compound assign, got {:?}", body[2]);
        };
        assert_eq!(*op, CompoundOp::Add);
    }

    #[test]
    fn test_bitwise_and_tighter_than_or() {
        let src = "func main() { let _ = 1 | 2 & 3; }";
        let mut lexer = Lexer::new(src);
        let tokens = lexer.tokenize().unwrap();
        let mut parser = Parser::new(tokens);
        let ast = parser.parse().unwrap();
        let AstNode::Program(nodes) = ast else {
            panic!("Program");
        };
        let AstNode::Function { body, .. } = &nodes[0] else {
            panic!("Function");
        };
        let AstNode::Let { initializer: Some(init), .. } = &body[0] else {
            panic!("let");
        };
        let AstNode::BinaryOp {
            op: BinaryOp::BitOr,
            right,
            ..
        } = init.as_ref()
        else {
            panic!("expected | at root, got {:?}", init);
        };
        assert!(matches!(
            right.as_ref(),
            AstNode::BinaryOp {
                op: BinaryOp::BitAnd,
                ..
            }
        ));
    }

    #[test]
    fn test_tuple_pattern_rejects_two_rests() {
        let src = "func main() { let (a, .., .., b) = (1, 2, 3, 4); }";
        let mut lexer = Lexer::new(src);
        let tokens = lexer.tokenize().unwrap();
        let mut parser = Parser::new(tokens);
        let err = parser.parse().unwrap_err();
        assert!(
            err.to_string().contains("..") || format!("{err:?}").contains(".."),
            "{err:?}"
        );
    }

    #[test]
    fn test_parse_top_level_let_requires_initializer() {
        let src = "let x: Int;\nfunc main() {}";
        let mut lexer = Lexer::new(src);
        let tokens = lexer.tokenize().unwrap();
        let mut parser = Parser::new(tokens);
        let err = parser.parse().unwrap_err();
        assert!(
            err.to_string().contains("initializer") || format!("{err:?}").contains("initializer"),
            "{err:?}"
        );
    }

    #[test]
    fn test_parse_func_main_with_comment_in_body() {
        let src = "func main() {\n    // nothing\n}";
        let mut lexer = Lexer::new(src);
        let tokens = lexer.tokenize().unwrap();
        let mut parser = Parser::new(tokens);
        let ast = parser.parse().unwrap();
        match ast {
            AstNode::Program(nodes) => {
                assert_eq!(nodes.len(), 1);
                match &nodes[0] {
                    AstNode::Function {
                        name,
                        params,
                        return_type,
                        body,
                        ..
                    } => {
                        assert_eq!(name, "main");
                        assert!(params.is_empty());
                        assert!(return_type.is_none());
                        assert_eq!(body.len(), 1);
                        assert!(matches!(
                            &body[0],
                            AstNode::SingleLineComment(s) if s == " nothing"
                        ));
                    }
                    _ => panic!("Expected Function"),
                }
            }
            _ => panic!("Expected Program"),
        }
    }

    #[test]
    fn test_parse_example1() {
        let src = include_str!("../examples/basics/example1.vc");
        let mut lexer = Lexer::new(src);
        let tokens = lexer.tokenize().unwrap();
        let mut parser = Parser::new(tokens);
        let ast = parser.parse().unwrap();
        let AstNode::Program(nodes) = ast else {
            panic!("expected Program");
        };
        let funcs: Vec<_> = nodes
            .iter()
            .filter(|n| matches!(n, AstNode::Function { .. }))
            .collect();
        assert_eq!(funcs.len(), 1);
        match funcs[0] {
            AstNode::Function { name, body, params, .. } => {
                assert_eq!(name, "main");
                assert!(params.is_empty());
                assert_eq!(body.len(), 1);
                let AstNode::SingleLineComment(text) = &body[0] else {
                    panic!("expected comment in body");
                };
                assert_eq!(text.trim_end_matches('\r'), " nothing to do");
            }
            _ => panic!("expected Function"),
        }
    }

    #[test]
    fn test_parse_internal_print_declaration() {
        let src = "internal func print(s: String);";
        let mut lexer = Lexer::new(src);
        let tokens = lexer.tokenize().unwrap();
        let mut parser = Parser::new(tokens);
        let ast = parser.parse().unwrap();
        match ast {
            AstNode::Program(nodes) => {
                assert_eq!(nodes.len(), 1);
                match &nodes[0] {
                    AstNode::InternalFunction {
                        name,
                        params,
                        return_type,
                        ..
                    } => {
                        assert_eq!(name, "print");
                        assert_eq!(params.len(), 1);
                        assert_eq!(params[0].name, "s");
                        assert_eq!(named_type_name(&params[0].ty), Some("String"));
                        assert!(return_type.is_none());
                    }
                    _ => panic!("expected InternalFunction"),
                }
            }
            _ => panic!("expected Program"),
        }
    }

    #[test]
    fn test_parse_call_hello_world() {
        let src = r#"print("Hello, world!");"#;
        let mut lexer = Lexer::new(src);
        let tokens = lexer.tokenize().unwrap();
        let mut parser = Parser::new(tokens);
        let ast = parser.parse().unwrap();
        match ast {
            AstNode::Program(nodes) => {
                assert_eq!(nodes.len(), 1);
                match &nodes[0] {
                    AstNode::Call {
                        callee,
                        arguments,
                        ..
                    } => {
                        assert_eq!(callee, "print");
                        assert_eq!(arguments.len(), 1);
                        match &arguments[0] {
                            CallArg::Positional(AstNode::StringLiteral {
                                value,
                                original,
                                ..
                            }) => {
                                assert_eq!(value, "Hello, world!");
                                assert_eq!(original, "\"Hello, world!\"");
                            }
                            _ => panic!("expected string arg"),
                        }
                    }
                    _ => panic!("expected Call"),
                }
            }
            _ => panic!("expected Program"),
        }
    }

    #[test]
    fn test_parse_example2() {
        let src = include_str!("../examples/basics/example2_0.vc");
        let mut lexer = Lexer::new(src);
        let tokens = lexer.tokenize().unwrap();
        let mut parser = Parser::new(tokens);
        let ast = parser.parse().unwrap();
        let AstNode::Program(nodes) = ast else {
            panic!("expected Program");
        };
        let internals: Vec<_> = nodes
            .iter()
            .filter(|n| matches!(n, AstNode::InternalFunction { .. }))
            .collect();
        let funcs: Vec<_> = nodes
            .iter()
            .filter(|n| matches!(n, AstNode::Function { .. }))
            .collect();
        assert_eq!(internals.len(), 1);
        assert_eq!(funcs.len(), 1);
        match internals[0] {
            AstNode::InternalFunction {
                name,
                params,
                return_type,
                ..
            } => {
                assert_eq!(name, "print");
                assert_eq!(params.len(), 1);
                assert_eq!(params[0].name, "s");
                assert_eq!(named_type_name(&params[0].ty), Some("String"));
                assert!(return_type.is_none());
            }
            _ => panic!("internal print"),
        }
        match funcs[0] {
            AstNode::Function { name, params, body, .. } => {
                assert_eq!(name, "main");
                assert!(params.is_empty());
                assert_eq!(body.len(), 1);
                match &body[0] {
                    AstNode::Call {
                        callee,
                        arguments,
                        ..
                    } => {
                        assert_eq!(callee, "print");
                        assert_eq!(arguments.len(), 1);
                        match &arguments[0] {
                            CallArg::Positional(AstNode::StringLiteral {
                                value,
                                original,
                                ..
                            }) => {
                                assert_eq!(value, "Hello, world!");
                                assert_eq!(original, "\"Hello, world!\"");
                            }
                            _ => panic!("expected string arg"),
                        }
                    }
                    _ => panic!("expected call in main"),
                }
            }
            _ => panic!("expected main function"),
        }
    }

    #[test]
    fn test_internal_function_with_body_is_rejected() {
        let src = "internal func f() { }";
        let mut lexer = Lexer::new(src);
        let tokens = lexer.tokenize().unwrap();
        let mut parser = Parser::new(tokens);
        let err = parser.parse().unwrap_err();
        assert!(
            err.to_string().contains('`') || err.to_string().contains("expected"),
            "{}",
            err
        );
        match err {
            ParseError::UnexpectedToken { message, .. } => {
                assert!(message.contains(';') || message.contains("{"));
            }
            _ => {}
        }
    }

    #[test]
    fn test_internal_inside_function_is_rejected() {
        let src = "func main() { internal func x(); }";
        let mut lexer = Lexer::new(src);
        let tokens = lexer.tokenize().unwrap();
        let mut parser = Parser::new(tokens);
        let err = parser.parse().unwrap_err();
        assert!(err.to_string().contains("internal"));
    }

    #[test]
    fn test_parse_internal_with_return_type() {
        let src = "internal func read_int(): Int;";
        let mut lexer = Lexer::new(src);
        let tokens = lexer.tokenize().unwrap();
        let mut parser = Parser::new(tokens);
        let ast = parser.parse().unwrap();
        match ast {
            AstNode::Program(nodes) => {
                match &nodes[0] {
                    AstNode::InternalFunction {
                        name,
                        params,
                        return_type,
                        ..
                    } => {
                        assert_eq!(name, "read_int");
                        assert!(params.is_empty());
                        assert_eq!(return_type.as_ref().and_then(named_type_name), Some("Int"));
                    }
                    _ => panic!("expected internal"),
                }
            }
            _ => panic!("expected Program"),
        }
    }

    #[test]
    fn test_parse_internal_print_int() {
        let src = "internal func print_int(v: Int);";
        let mut lexer = Lexer::new(src);
        let tokens = lexer.tokenize().unwrap();
        let mut parser = Parser::new(tokens);
        let ast = parser.parse().unwrap();
        match ast {
            AstNode::Program(nodes) => {
                match &nodes[0] {
                    AstNode::InternalFunction {
                        name,
                        params,
                        return_type,
                        ..
                    } => {
                        assert_eq!(name, "print_int");
                        assert_eq!(params.len(), 1);
                        assert_eq!(params[0].name, "v");
                        assert_eq!(named_type_name(&params[0].ty), Some("Int"));
                        assert!(return_type.is_none());
                    }
                    _ => panic!("expected internal"),
                }
            }
            _ => panic!("expected Program"),
        }
    }

    #[test]
    fn test_parse_func_with_return_type_and_return_add() {
        let src = "func add(a: Int, b: Int): Int {\n    return a + b;\n}";
        let mut lexer = Lexer::new(src);
        let tokens = lexer.tokenize().unwrap();
        let mut parser = Parser::new(tokens);
        let ast = parser.parse().unwrap();
        match ast {
            AstNode::Program(nodes) => {
                assert_eq!(nodes.len(), 1);
                let AstNode::Function {
                    name,
                    params,
                    return_type,
                    body,
                    ..
                } = &nodes[0]
                else {
                    panic!("expected Function");
                };
                assert_eq!(name, "add");
                assert_eq!(params.len(), 2);
                assert_eq!(params[0].name, "a");
                assert_eq!(named_type_name(&params[0].ty), Some("Int"));
                assert_eq!(params[1].name, "b");
                assert_eq!(named_type_name(&params[1].ty), Some("Int"));
                assert_eq!(return_type.as_ref().and_then(named_type_name), Some("Int"));
                assert_eq!(body.len(), 1);
                match &body[0] {
                    AstNode::Return { value: Some(v), .. } => match v.as_ref() {
                        AstNode::BinaryOp { left, op, right, .. } => {
                            assert_eq!(*op, BinaryOp::Add);
                            assert!(matches!(
                                left.as_ref(),
                                AstNode::Identifier { name, .. } if name == "a"
                            ));
                            assert!(matches!(
                                right.as_ref(),
                                AstNode::Identifier { name, .. } if name == "b"
                            ));
                        }
                        _ => panic!("expected binary op"),
                    },
                    _ => panic!("expected return with value"),
                }
            }
            _ => panic!("expected Program"),
        }
    }

    #[test]
    fn test_parse_nested_call_print_int_add() {
        let src = "func main() {\n    print_int(add(2, 3));\n}";
        let mut lexer = Lexer::new(src);
        let tokens = lexer.tokenize().unwrap();
        let mut parser = Parser::new(tokens);
        let ast = parser.parse().unwrap();
        match ast {
            AstNode::Program(nodes) => {
                let AstNode::Function { body, .. } = &nodes[0] else {
                    panic!("expected Function");
                };
                assert_eq!(body.len(), 1);
                match &body[0] {
                    AstNode::Call {
                        callee,
                        arguments,
                        ..
                    } => {
                        assert_eq!(callee, "print_int");
                        assert_eq!(arguments.len(), 1);
                        match &arguments[0] {
                            CallArg::Positional(AstNode::Call {
                                callee: inner,
                                arguments: inner_args,
                                ..
                            }) => {
                                assert_eq!(inner, "add");
                                assert_eq!(inner_args.len(), 2);
                                assert!(matches!(
                                    &inner_args[0],
                                    CallArg::Positional(AstNode::IntegerLiteral { value: 2, .. })
                                ));
                                assert!(matches!(
                                    &inner_args[1],
                                    CallArg::Positional(AstNode::IntegerLiteral { value: 3, .. })
                                ));
                            }
                            _ => panic!("expected inner call"),
                        }
                    }
                    _ => panic!("expected call"),
                }
            }
            _ => panic!("expected Program"),
        }
    }

    #[test]
    fn test_parse_example3_0() {
        let src = include_str!("../examples/basics/example3_0.vc");
        let mut lexer = Lexer::new(src);
        let tokens = lexer.tokenize().unwrap();
        let mut parser = Parser::new(tokens);
        let ast = parser.parse().unwrap();
        let AstNode::Program(nodes) = ast else {
            panic!("expected Program");
        };
        let internals: Vec<_> = nodes
            .iter()
            .filter(|n| matches!(n, AstNode::InternalFunction { .. }))
            .collect();
        let funcs: Vec<_> = nodes
            .iter()
            .filter(|n| matches!(n, AstNode::Function { .. }))
            .collect();
        assert_eq!(internals.len(), 1);
        assert_eq!(funcs.len(), 2);
        match internals[0] {
            AstNode::InternalFunction {
                name,
                params,
                return_type,
                ..
            } => {
                assert_eq!(name, "print_int");
                assert_eq!(params.len(), 1);
                assert_eq!(params[0].name, "v");
                assert_eq!(named_type_name(&params[0].ty), Some("Int"));
                assert!(return_type.is_none());
            }
            _ => panic!("print_int internal"),
        }
        let add = funcs
            .iter()
            .find(|f| matches!(f, AstNode::Function { name, .. } if name == "add"))
            .expect("add");
        match add {
            AstNode::Function {
                params,
                return_type,
                body,
                ..
            } => {
                assert_eq!(return_type.as_ref().and_then(named_type_name), Some("Int"));
                assert_eq!(params.len(), 2);
                assert!(body.iter().any(|s| matches!(s, AstNode::Return { .. })));
            }
            _ => panic!("add"),
        }
    }

    #[test]
    fn test_parse_example3_1() {
        let src = include_str!("../examples/basics/example3_1.vc");
        let mut lexer = Lexer::new(src);
        let tokens = lexer.tokenize().expect("lex example3_1.vc");
        let mut parser = Parser::new(tokens);
        let ast = parser.parse().expect("parse example3_1.vc");
        let AstNode::Program(nodes) = ast else {
            panic!("expected Program");
        };
        let internals: Vec<_> = nodes
            .iter()
            .filter(|n| matches!(n, AstNode::InternalFunction { .. }))
            .collect();
        let funcs: Vec<_> = nodes
            .iter()
            .filter(|n| matches!(n, AstNode::Function { .. }))
            .collect();
        assert_eq!(internals.len(), 3, "itos, concat, print");
        assert_eq!(funcs.len(), 2, "add, main");

        let concat = internals
            .iter()
            .find(|n| matches!(**n, AstNode::InternalFunction { name, .. } if name == "concat"))
            .expect("concat internal");
        match *concat {
            AstNode::InternalFunction {
                params,
                return_type,
                ..
            } => {
                assert_eq!(params.len(), 2);
                assert_eq!(params[0].name, "s1");
                assert_eq!(named_type_name(&params[0].ty), Some("String"));
                assert_eq!(params[1].name, "s2");
                assert_eq!(named_type_name(&params[1].ty), Some("String"));
                assert_eq!(return_type.as_ref().and_then(named_type_name), Some("String"));
            }
            _ => panic!("concat"),
        }

        let itos = internals
            .iter()
            .find(|n| matches!(**n, AstNode::InternalFunction { name, .. } if name == "itos"))
            .expect("itos");
        match *itos {
            AstNode::InternalFunction {
                params,
                return_type,
                ..
            } => {
                assert_eq!(params.len(), 1);
                assert_eq!(named_type_name(&params[0].ty), Some("Int"));
                assert_eq!(return_type.as_ref().and_then(named_type_name), Some("String"));
            }
            _ => panic!("itos"),
        }
    }

}

#[cfg(test)]
mod operator_edge_tests {
    use super::*;
    use crate::lexer::Lexer;

    fn parse_expr_via_return(expr_src: &str) -> AstNode {
        let src = format!("func __probe() {{ return {expr_src}; }}");
        let mut lexer = Lexer::new(&src);
        let tokens = lexer.tokenize().expect("lex");
        let mut parser = Parser::new(tokens);
        let ast = parser.parse().expect("parse");
        let AstNode::Program(nodes) = ast else {
            panic!("Program");
        };
        let AstNode::Function { body, .. } = &nodes[0] else {
            panic!("Function");
        };
        let AstNode::Return { value: Some(v), .. } = &body[0] else {
            panic!("Return");
        };
        v.as_ref().clone()
    }

    #[test]
    fn lexer_emits_arithmetic_and_tilde_tokens() {
        let mut lexer = Lexer::new("- * / % ~ +");
        let kinds: Vec<_> = lexer.tokenize().unwrap().into_iter().map(|t| t.kind).collect();
        assert!(matches!(kinds[0], TokenKind::Minus));
        assert!(matches!(kinds[1], TokenKind::Star));
        assert!(matches!(kinds[2], TokenKind::Slash));
        assert!(matches!(kinds[3], TokenKind::Percent));
        assert!(matches!(kinds[4], TokenKind::Tilde));
        assert!(matches!(kinds[5], TokenKind::Plus));
    }

    #[test]
    fn prec_mul_before_add() {
        let e = parse_expr_via_return("1 + 2 * 3");
        match e {
            AstNode::BinaryOp {
                op: BinaryOp::Add,
                left,
                right,
                ..
            } => {
                assert!(matches!(*left, AstNode::IntegerLiteral { value: 1, .. }));
                match *right {
                    AstNode::BinaryOp {
                        op: BinaryOp::Mul,
                        ref left,
                        ref right,
                        ..
                    } => {
                        assert!(matches!(
                            **left,
                            AstNode::IntegerLiteral { value: 2, .. }
                        ));
                        assert!(matches!(
                            **right,
                            AstNode::IntegerLiteral { value: 3, .. }
                        ));
                    }
                    _ => panic!("right"),
                }
            }
            _ => panic!("root"),
        }
    }

    #[test]
    fn prec_doc_example_12() {
        let e = parse_expr_via_return("1 + 2 * -3 + 17");
        match e {
            AstNode::BinaryOp {
                op: BinaryOp::Add,
                left,
                right, ..
            } => {
                match *left {
                    AstNode::BinaryOp {
                        op: BinaryOp::Add,
                        left: l,
                        right: r, ..
                    } => {
                        assert!(matches!(*l, AstNode::IntegerLiteral { value: 1, .. }));
                        match *r {
                            AstNode::BinaryOp {
                                op: BinaryOp::Mul,
                                left: a,
                                right: b,
                                ..
                            } => {
                                assert!(matches!(*a, AstNode::IntegerLiteral { value: 2, .. }));
                                match *b {
                                    AstNode::UnaryOp {
                                        op: UnaryOp::Minus,
                                        operand,
                                        ..
                                    } => {
                                        assert!(matches!(
                                            *operand,
                                            AstNode::IntegerLiteral { value: 3, .. }
                                        ));
                                    }
                                    _ => panic!("unary -3"),
                                }
                            }
                            _ => panic!("2*-3"),
                        }
                    }
                    _ => panic!("left +"),
                }
                assert!(matches!(
                    *right,
                    AstNode::IntegerLiteral { value: 17, .. }
                ));
            }
            _ => panic!("root"),
        }
    }

    #[test]
    fn prec_parens_multiply() {
        let e = parse_expr_via_return("(1 + 2) * (3 + 4)");
        match e {
            AstNode::BinaryOp {
                op: BinaryOp::Mul,
                left,
                right, ..
            } => {
                match *left {
                    AstNode::BinaryOp {
                        op: BinaryOp::Add,
                        left: a,
                        right: b, ..
                    } => {
                        assert!(matches!(*a, AstNode::IntegerLiteral { value: 1, .. }));
                        assert!(matches!(*b, AstNode::IntegerLiteral { value: 2, .. }));
                    }
                    _ => panic!("1+2"),
                }
                match *right {
                    AstNode::BinaryOp {
                        op: BinaryOp::Add,
                        left: a,
                        right: b, ..
                    } => {
                        assert!(matches!(*a, AstNode::IntegerLiteral { value: 3, .. }));
                        assert!(matches!(*b, AstNode::IntegerLiteral { value: 4, .. }));
                    }
                    _ => panic!("3+4"),
                }
            }
            _ => panic!("root"),
        }
    }

    #[test]
    fn additive_left_associative() {
        let e = parse_expr_via_return("10 - 3 - 2");
        match e {
            AstNode::BinaryOp {
                op: BinaryOp::Sub,
                left,
                right, ..
            } => {
                match *left {
                    AstNode::BinaryOp {
                        op: BinaryOp::Sub,
                        left: a,
                        right: b, ..
                    } => {
                        assert!(matches!(*a, AstNode::IntegerLiteral { value: 10, .. }));
                        assert!(matches!(*b, AstNode::IntegerLiteral { value: 3, .. }));
                    }
                    _ => panic!("10-3"),
                }
                assert!(matches!(
                    *right,
                    AstNode::IntegerLiteral { value: 2, .. }
                ));
            }
            _ => panic!("root"),
        }
    }

    #[test]
    fn multiplicative_left_associative() {
        let e = parse_expr_via_return("24 / 4 / 2");
        match e {
            AstNode::BinaryOp {
                op: BinaryOp::Div,
                left,
                right, ..
            } => {
                match *left {
                    AstNode::BinaryOp {
                        op: BinaryOp::Div,
                        left: a,
                        right: b, ..
                    } => {
                        assert!(matches!(*a, AstNode::IntegerLiteral { value: 24, .. }));
                        assert!(matches!(*b, AstNode::IntegerLiteral { value: 4, .. }));
                    }
                    _ => panic!("24/4"),
                }
                assert!(matches!(
                    *right,
                    AstNode::IntegerLiteral { value: 2, .. }
                ));
            }
            _ => panic!("root"),
        }
    }

    #[test]
    fn unary_minus_literal() {
        let e = parse_expr_via_return("-4");
        match e {
            AstNode::UnaryOp {
                op: UnaryOp::Minus,
                operand, ..
            } => {
                assert!(matches!(
                    *operand,
                    AstNode::IntegerLiteral { value: 4, .. }
                ));
            }
            _ => panic!("unary -"),
        }
    }

    #[test]
    fn unary_plus_literal() {
        let e = parse_expr_via_return("+5");
        match e {
            AstNode::UnaryOp {
                op: UnaryOp::Plus,
                operand, ..
            } => {
                assert!(matches!(
                    *operand,
                    AstNode::IntegerLiteral { value: 5, .. }
                ));
            }
            _ => panic!("unary +"),
        }
    }

    #[test]
    fn unary_tilde_literal() {
        let e = parse_expr_via_return("~0");
        match e {
            AstNode::UnaryOp {
                op: UnaryOp::BitNot,
                operand, ..
            } => {
                assert!(matches!(
                    *operand,
                    AstNode::IntegerLiteral { value: 0, .. }
                ));
            }
            _ => panic!("~"),
        }
    }

    #[test]
    fn unary_chain_tilde_then_neg() {
        let e = parse_expr_via_return("~-1");
        match e {
            AstNode::UnaryOp {
                op: UnaryOp::BitNot,
                operand, ..
            } => {
                match *operand {
                    AstNode::UnaryOp {
                        op: UnaryOp::Minus,
                        operand: inner, ..
                    } => {
                        assert!(matches!(
                            *inner,
                            AstNode::IntegerLiteral { value: 1, .. }
                        ));
                    }
                    _ => panic!("inner -1"),
                }
            }
            _ => panic!("~-1"),
        }
    }

    #[test]
    fn unary_double_minus() {
        let e = parse_expr_via_return("--1");
        match e {
            AstNode::UnaryOp {
                op: UnaryOp::Minus,
                operand, ..
            } => {
                match *operand {
                    AstNode::UnaryOp {
                        op: UnaryOp::Minus,
                        operand: inner, ..
                    } => {
                        assert!(matches!(
                            *inner,
                            AstNode::IntegerLiteral { value: 1, .. }
                        ));
                    }
                    _ => panic!("inner"),
                }
            }
            _ => panic!("--"),
        }
    }

    #[test]
    fn string_concat_same_precedence_as_add() {
        let e = parse_expr_via_return("\"a\" + \"b\" + \"c\"");
        match e {
            AstNode::BinaryOp {
                op: BinaryOp::Add,
                left,
                right, ..
            } => {
                match *left {
                    AstNode::BinaryOp {
                        op: BinaryOp::Add,
                        left: a,
                        right: b, ..
                    } => {
                        assert!(matches!(
                            *a,
                            AstNode::StringLiteral { ref value, .. } if value == "a"
                        ));
                        assert!(matches!(
                            *b,
                            AstNode::StringLiteral { ref value, .. } if value == "b"
                        ));
                    }
                    _ => panic!("a+b"),
                }
                assert!(matches!(
                    *right,
                    AstNode::StringLiteral { ref value, .. } if value == "c"
                ));
            }
            _ => panic!("root"),
        }
    }

    #[test]
    fn mod_div_mul_precedence() {
        let e = parse_expr_via_return("10 + 9 % 3 * 2");
        match e {
            AstNode::BinaryOp {
                op: BinaryOp::Add,
                left,
                right, ..
            } => {
                assert!(matches!(*left, AstNode::IntegerLiteral { value: 10, .. }));
                match *right {
                    AstNode::BinaryOp {
                        op: BinaryOp::Mul,
                        left: l,
                        right: r, ..
                    } => {
                        assert!(matches!(
                            *l,
                            AstNode::BinaryOp {
                                op: BinaryOp::Mod,
                                ..
                            }
                        ));
                        assert!(matches!(
                            *r,
                            AstNode::IntegerLiteral { value: 2, .. }
                        ));
                    }
                    _ => panic!("right"),
                }
            }
            _ => panic!("root"),
        }
    }

    #[test]
    fn call_with_binary_arguments() {
        let src = "func f() { return g(1 + 2, 3 * 4); }";
        let mut lexer = Lexer::new(src);
        let tokens = lexer.tokenize().unwrap();
        let mut parser = Parser::new(tokens);
        let ast = parser.parse().unwrap();
        let AstNode::Program(nodes) = ast else {
            panic!();
        };
        let AstNode::Function { body, .. } = &nodes[0] else {
            panic!();
        };
        let AstNode::Return { value: Some(v), .. } = &body[0] else {
            panic!();
        };
        match v.as_ref() {
            AstNode::Call {
                callee,
                arguments, ..
            } => {
                assert_eq!(callee, "g");
                assert_eq!(arguments.len(), 2);
                assert!(matches!(
                    &arguments[0],
                    CallArg::Positional(AstNode::BinaryOp {
                        op: BinaryOp::Add,
                        ..
                    })
                ));
                assert!(matches!(
                    &arguments[1],
                    CallArg::Positional(AstNode::BinaryOp {
                        op: BinaryOp::Mul,
                        ..
                    })
                ));
            }
            _ => panic!("call"),
        }
    }

    #[test]
    fn unary_minus_before_call() {
        let src = "func f() { return -neg(1); }";
        let mut lexer = Lexer::new(src);
        let tokens = lexer.tokenize().unwrap();
        let mut parser = Parser::new(tokens);
        let ast = parser.parse().unwrap();
        let AstNode::Program(nodes) = ast else {
            panic!();
        };
        let AstNode::Function { body, .. } = &nodes[0] else {
            panic!();
        };
        let AstNode::Return { value: Some(v), .. } = &body[0] else {
            panic!();
        };
        match v.as_ref() {
            AstNode::UnaryOp {
                op: UnaryOp::Minus,
                operand, ..
            } => {
                assert!(matches!(
                    operand.as_ref(),
                    AstNode::Call {
                        callee,
                        ..
                    } if callee == "neg"
                ));
            }
            _ => panic!("-call"),
        }
    }

    #[test]
    fn parse_example4_file() {
        let src = include_str!("../examples/operators/example4.vc");
        let mut lexer = Lexer::new(src);
        let tokens = lexer.tokenize().expect("lex example4");
        let mut parser = Parser::new(tokens);
        parser.parse().expect("parse example4");
    }

    #[test]
    fn priority_chains_from_example4() {
        let e = parse_expr_via_return("(1 + 2) * (-3 + 17)");
        match e {
            AstNode::BinaryOp {
                op: BinaryOp::Mul,
                left,
                right, ..
            } => {
                assert!(matches!(
                    *left,
                    AstNode::BinaryOp {
                        op: BinaryOp::Add,
                        ..
                    }
                ));
                assert!(matches!(
                    *right,
                    AstNode::BinaryOp {
                        op: BinaryOp::Add,
                        ..
                    }
                ));
            }
            _ => panic!("(1+2)*(-3+17)"),
        }
    }

    #[test]
    fn unary_minus_string_concat_not_allowed_as_sub() {
        let e = parse_expr_via_return("\"x\" + -1");
        match e {
            AstNode::BinaryOp {
                op: BinaryOp::Add,
                left,
                right, ..
            } => {
                assert!(matches!(
                    *left,
                    AstNode::StringLiteral { ref value, .. } if value == "x"
                ));
                match *right {
                    AstNode::UnaryOp {
                        op: UnaryOp::Minus,
                        operand, ..
                    } => {
                        assert!(matches!(
                            *operand,
                            AstNode::IntegerLiteral { value: 1, .. }
                        ));
                    }
                    _ => panic!("-1"),
                }
            }
            _ => panic!("concat"),
        }
    }

    #[test]
    fn parens_only_inner() {
        let e = parse_expr_via_return("(((42)))");
        assert!(matches!(
            e,
            AstNode::IntegerLiteral { value: 42, .. }
        ));
    }

    #[test]
    fn mul_sub_combo() {
        let e = parse_expr_via_return("6 * 2 - 1");
        match e {
            AstNode::BinaryOp {
                op: BinaryOp::Sub,
                left,
                right, ..
            } => {
                assert!(matches!(
                    *left,
                    AstNode::BinaryOp {
                        op: BinaryOp::Mul,
                        ..
                    }
                ));
                assert!(matches!(
                    *right,
                    AstNode::IntegerLiteral { value: 1, .. }
                ));
            }
            _ => panic!("6*2-1"),
        }
    }

    #[test]
    fn percent_add_precedence() {
        let e = parse_expr_via_return("10 % 3 + 1");
        match e {
            AstNode::BinaryOp {
                op: BinaryOp::Add,
                left,
                right, ..
            } => {
                assert!(matches!(
                    *left,
                    AstNode::BinaryOp {
                        op: BinaryOp::Mod,
                        ..
                    }
                ));
                assert!(matches!(
                    *right,
                    AstNode::IntegerLiteral { value: 1, .. }
                ));
            }
            _ => panic!("10%3+1"),
        }
    }

    #[test]
    fn unary_double_tilde() {
        let e = parse_expr_via_return("~~1");
        match e {
            AstNode::UnaryOp {
                op: UnaryOp::BitNot,
                operand, ..
            } => {
                match *operand {
                    AstNode::UnaryOp {
                        op: UnaryOp::BitNot,
                        operand: inner, ..
                    } => {
                        assert!(matches!(
                            *inner,
                            AstNode::IntegerLiteral { value: 1, .. }
                        ));
                    }
                    _ => panic!("inner"),
                }
            }
            _ => panic!("~~"),
        }
    }

    #[test]
    fn plus_unary_on_paren_expr() {
        let e = parse_expr_via_return("+(1 + 2)");
        match e {
            AstNode::UnaryOp {
                op: UnaryOp::Plus,
                operand, ..
            } => {
                assert!(matches!(
                    *operand,
                    AstNode::BinaryOp {
                        op: BinaryOp::Add,
                        ..
                    }
                ));
            }
            _ => panic!("+(...)"),
        }
    }

    #[test]
    fn identifier_sub_identifier() {
        let e = parse_expr_via_return("a - b");
        match e {
            AstNode::BinaryOp {
                op: BinaryOp::Sub,
                left,
                right, ..
            } => {
                assert!(matches!(
                    *left,
                    AstNode::Identifier { name, .. } if name == "a"
                ));
                assert!(matches!(
                    *right,
                    AstNode::Identifier { name, .. } if name == "b"
                ));
            }
            _ => panic!("a-b"),
        }
    }

    #[test]
    fn identifier_mul_identifier() {
        let e = parse_expr_via_return("a * b");
        match e {
            AstNode::BinaryOp {
                op: BinaryOp::Mul,
                left,
                right, ..
            } => {
                assert!(matches!(
                    *left,
                    AstNode::Identifier { name, .. } if name == "a"
                ));
                assert!(matches!(
                    *right,
                    AstNode::Identifier { name, .. } if name == "b"
                ));
            }
            _ => panic!("a*b"),
        }
    }

    #[test]
    fn spaced_operators() {
        let e = parse_expr_via_return("1   +   2   *   3");
        assert!(matches!(
            e,
            AstNode::BinaryOp {
                op: BinaryOp::Add,
                ..
            }
        ));
    }

    #[test]
    fn division_simple() {
        let e = parse_expr_via_return("11 / 3");
        match e {
            AstNode::BinaryOp {
                op: BinaryOp::Div,
                left,
                right, ..
            } => {
                assert!(matches!(
                    *left,
                    AstNode::IntegerLiteral { value: 11, .. }
                ));
                assert!(matches!(
                    *right,
                    AstNode::IntegerLiteral { value: 3, .. }
                ));
            }
            _ => panic!("/"),
        }
    }

    #[test]
    fn modulo_simple() {
        let e = parse_expr_via_return("7 % 4");
        match e {
            AstNode::BinaryOp {
                op: BinaryOp::Mod,
                left,
                right, ..
            } => {
                assert!(matches!(
                    *left,
                    AstNode::IntegerLiteral { value: 7, .. }
                ));
                assert!(matches!(
                    *right,
                    AstNode::IntegerLiteral { value: 4, .. }
                ));
            }
            _ => panic!("%"),
        }
    }

    #[test]
    fn negated_parenthesized() {
        let e = parse_expr_via_return("-(1 + 2)");
        match e {
            AstNode::UnaryOp {
                op: UnaryOp::Minus,
                operand, ..
            } => {
                assert!(matches!(
                    *operand,
                    AstNode::BinaryOp {
                        op: BinaryOp::Add,
                        ..
                    }
                ));
            }
            _ => panic!("-( )"),
        }
    }

    #[test]
    fn add_mul_sub_chain() {
        let e = parse_expr_via_return("1 + 2 * 3 - 4");
        match e {
            AstNode::BinaryOp {
                op: BinaryOp::Sub,
                left,
                right, ..
            } => {
                assert!(matches!(
                    *right,
                    AstNode::IntegerLiteral { value: 4, .. }
                ));
                match *left {
                    AstNode::BinaryOp {
                        op: BinaryOp::Add,
                        left: a,
                        right: b, ..
                    } => {
                        assert!(matches!(*a, AstNode::IntegerLiteral { value: 1, .. }));
                        assert!(matches!(
                            *b,
                            AstNode::BinaryOp {
                                op: BinaryOp::Mul,
                                ..
                            }
                        ));
                    }
                    _ => panic!("left"),
                }
            }
            _ => panic!("chain"),
        }
    }

    #[test]
    fn tilde_plus_combo() {
        let e = parse_expr_via_return("~+0");
        match e {
            AstNode::UnaryOp {
                op: UnaryOp::BitNot,
                operand, ..
            } => {
                match *operand {
                    AstNode::UnaryOp {
                        op: UnaryOp::Plus,
                        operand: inner, ..
                    } => {
                        assert!(matches!(
                            *inner,
                            AstNode::IntegerLiteral { value: 0, .. }
                        ));
                    }
                    _ => panic!("+0"),
                }
            }
            _ => panic!("~+"),
        }
    }

    #[test]
    fn prec_cmp_looser_than_add() {
        let e = parse_expr_via_return("1 > 1 + 1");
        match e {
            AstNode::BinaryOp {
                op: BinaryOp::Gt,
                left,
                right, ..
            } => {
                assert!(matches!(*left, AstNode::IntegerLiteral { value: 1, .. }));
                match *right {
                    AstNode::BinaryOp {
                        op: BinaryOp::Add,
                        left: a,
                        right: b, ..
                    } => {
                        assert!(matches!(*a, AstNode::IntegerLiteral { value: 1, .. }));
                        assert!(matches!(*b, AstNode::IntegerLiteral { value: 1, .. }));
                    }
                    _ => panic!("1+1"),
                }
            }
            _ => panic!("root >"),
        }
    }

    #[test]
    fn prec_and_tighter_than_or() {
        let e = parse_expr_via_return("false || true && true");
        match e {
            AstNode::BinaryOp {
                op: BinaryOp::Or,
                left,
                right, ..
            } => {
                assert!(matches!(
                    *left,
                    AstNode::BoolLiteral { value: false, .. }
                ));
                match *right {
                    AstNode::BinaryOp {
                        op: BinaryOp::And,
                        left: a,
                        right: b, ..
                    } => {
                        assert!(matches!(
                            *a,
                            AstNode::BoolLiteral { value: true, .. }
                        ));
                        assert!(matches!(
                            *b,
                            AstNode::BoolLiteral { value: true, .. }
                        ));
                    }
                    _ => panic!("&&"),
                }
            }
            _ => panic!("root ||"),
        }
    }

    #[test]
    fn prec_not_tighter_than_and() {
        let e = parse_expr_via_return("!true && false");
        match e {
            AstNode::BinaryOp {
                op: BinaryOp::And,
                left,
                right, ..
            } => {
                match *left {
                    AstNode::UnaryOp {
                        op: UnaryOp::Not,
                        operand, ..
                    } => {
                        assert!(matches!(
                            *operand,
                            AstNode::BoolLiteral { value: true, .. }
                        ));
                    }
                    _ => panic!("!true"),
                }
                assert!(matches!(
                    *right,
                    AstNode::BoolLiteral { value: false, .. }
                ));
            }
            _ => panic!("root &&"),
        }
    }

    #[test]
    fn eq_same_precedence_chains_left() {
        let e = parse_expr_via_return("0 == 0 == 0");
        match e {
            AstNode::BinaryOp {
                op: BinaryOp::Eq,
                left,
                ..
            } => {
                assert!(matches!(
                    *left,
                    AstNode::BinaryOp {
                        op: BinaryOp::Eq,
                        ..
                    }
                ));
            }
            _ => panic!("== chain"),
        }
    }

    #[test]
    fn parse_default_params_and_named_call() {
        let src = r#"
func foo(a: Int = 1, b: Bool = true) {}
func main() { foo(b: false, a: 7); }
"#;
        let mut lexer = Lexer::new(src);
        let tokens = lexer.tokenize().unwrap();
        let mut parser = Parser::new(tokens);
        let ast = parser.parse().unwrap();
        let AstNode::Program(nodes) = ast else { panic!("program"); };
        let AstNode::Function { params, .. } = &nodes[0] else { panic!("foo"); };
        assert!(params.iter().all(|p| p.default_value.is_some()));
        let AstNode::Function { body, .. } = &nodes[1] else { panic!("main"); };
        let AstNode::Call { arguments, .. } = &body[0] else { panic!("call"); };
        assert!(matches!(&arguments[0], CallArg::Named { name, .. } if name == "b"));
        assert!(matches!(&arguments[1], CallArg::Named { name, .. } if name == "a"));
    }

    #[test]
    fn parse_internal_default_param_is_error() {
        let src = r#"internal func test_func(n: String = "text");"#;
        let mut lexer = Lexer::new(src);
        let tokens = lexer.tokenize().unwrap();
        let mut parser = Parser::new(tokens);
        let err = parser.parse().unwrap_err();
        assert!(
            err.to_string()
                .contains("internal functions cannot have parameters with default values")
        );
    }

    #[test]
    fn parse_params_parameter_ok() {
        let src = r#"func sum(params numbers: [Int]): Int { return 0; }"#;
        let mut lexer = Lexer::new(src);
        let tokens = lexer.tokenize().unwrap();
        let mut parser = Parser::new(tokens);
        let ast = parser.parse().unwrap();
        let AstNode::Program(nodes) = ast else { panic!("program"); };
        let AstNode::Function { params, .. } = &nodes[0] else { panic!("function"); };
        assert_eq!(params.len(), 1);
        assert!(params[0].is_params);
    }

    #[test]
    fn parse_params_not_last_rejected() {
        let src = r#"func bad(params a: [Int], b: Int) {}"#;
        let mut lexer = Lexer::new(src);
        let tokens = lexer.tokenize().unwrap();
        let mut parser = Parser::new(tokens);
        let err = parser.parse().unwrap_err();
        assert!(err.to_string().contains("must be the last parameter"));
    }

    #[test]
    fn parse_params_non_array_rejected() {
        let src = r#"func bad(params a: Int) {}"#;
        let mut lexer = Lexer::new(src);
        let tokens = lexer.tokenize().unwrap();
        let mut parser = Parser::new(tokens);
        let err = parser.parse().unwrap_err();
        assert!(err.to_string().contains("must have array type"));
    }

    #[test]
    fn while_condition_accepts_call_comparison_rhs() {
        let src = r#"
internal func int_array_len(a: [Int]): Int;
func main() {
    let idx = 0;
    let xs: [Int] = [1, 2, 3];
    while idx < int_array_len(xs) {
        idx += 1;
    }
}
"#;
        let mut lexer = Lexer::new(src);
        let tokens = lexer.tokenize().unwrap();
        let mut parser = Parser::new(tokens);
        let ast = parser.parse().expect("parse");
        let AstNode::Program(nodes) = ast else { panic!("program"); };
        assert!(!nodes.is_empty());
    }
}

#[cfg(test)]
mod match_parser_edge_tests {
    use super::*;
    use crate::lexer::Lexer;

    fn parse_ok(src: &str) {
        let mut lexer = Lexer::new(src);
        let tokens = lexer.tokenize().expect("lex");
        let mut parser = Parser::new(tokens);
        parser.parse().expect("parse");
    }

    fn parse_err_contains(src: &str, needle: &str) {
        let mut lexer = Lexer::new(src);
        let tokens = lexer.tokenize().expect("lex");
        let mut parser = Parser::new(tokens);
        let err = parser.parse().expect_err("expected parse error");
        let msg = err.to_string();
        assert!(msg.contains(needle), "expected `{needle}` in `{msg}`");
    }

    #[test]
    fn match_parser_edge_01_no_trailing_comma_with_comment_ok() {
        parse_ok(
            r#"enum Option<T> { None, Some(T) }
               struct Point { x: Int, y: Int, }
               func f() = Option::Some(Point { x: 1, y: 2 });
               func main() {
                   match f() {
                       Option::Some(Point { x, y }) => x + y
                       // no trailing comma
                   }
               }"#,
        );
    }

    #[test]
    fn match_parser_edge_02_trailing_comma_with_comment_ok() {
        parse_ok(
            r#"enum Option<T> { None, Some(T) }
               func f() = Option::None;
               func main() {
                   match f() {
                       Option::None => 0,
                       // trailing comma before brace
                   }
               }"#,
        );
    }

    #[test]
    fn match_parser_edge_03_nested_tuple_array_rest_ok() {
        parse_ok(
            r#"func main() {
                   let a: [Int] = [1, 2, 3, 4];
                   match a {
                       [head, .., tail] => { let _ = head + tail; },
                   }
               }"#,
        );
    }

    #[test]
    fn match_parser_edge_04_alternatives_and_guard_ok() {
        parse_ok(
            r#"enum Option<T> { None, Some(T) }
               func f() = Option::Some(7);
               func main() {
                   let _: Int = match f() {
                       Option::Some(1) | Option::Some(2) if true => 1,
                       Option::Some(x) => x,
                       Option::None => 0,
                   };
               }"#,
        );
    }

    #[test]
    fn match_parser_edge_05_struct_rest_and_wildcards_ok() {
        parse_ok(
            r#"struct S { a: Int, b: Int, c: Int, }
               func mk() = S { a: 1, b: 2, c: 3 };
               func main() {
                   match mk() {
                       S { a: _, b, .. } => { let _ = b; },
                   }
               }"#,
        );
    }

    #[test]
    fn match_parser_edge_06_missing_fat_arrow_err() {
        parse_err_contains(
            r#"enum Option<T> { None, Some(T) }
               func f() = Option::None;
               func main() {
                   match f() {
                       Option::None = 0,
                   }
               }"#,
            "expected `=>`",
        );
    }

    #[test]
    fn match_parser_edge_07_missing_comma_between_arms_err() {
        parse_err_contains(
            r#"enum Option<T> { None, Some(T) }
               func f() = Option::None;
               func main() {
                   match f() {
                       Option::None => 0
                       Option::Some(x) => x,
                   }
               }"#,
            "expected `,` or `}` after match arm",
        );
    }

    #[test]
    fn match_parser_edge_08_double_tuple_rest_err() {
        parse_err_contains(
            r#"func main() {
                   let t = (1, 2, 3);
                   match t {
                       (a, .., b, ..) => a + b,
                   }
               }"#,
            "at most one `..`",
        );
    }

    #[test]
    fn match_parser_edge_09_double_array_rest_parses() {
        parse_ok(
            r#"func main() {
                   let a: [Int] = [1, 2, 3];
                   match a {
                       [x, .., y, ..] => x + y,
                   }
               }"#,
        );
    }

    #[test]
    fn match_parser_edge_10_enum_pattern_type_args_ok() {
        parse_ok(
            r#"enum Option<T> { None, Some(T) }
               func f() = Option::Some(1);
               func main() {
                   match f() {
                       Option<Int>::Some(x) => { let _ = x; },
                       Option<Int>::None => {},
                   }
               }"#,
        );
    }

    #[test]
    fn struct_fields_comma_separated_ok() {
        parse_ok(
            r#"struct Point { x: Int, y: Int }
               func main() { let _ = Point { x: 1, y: 2 }; }"#,
        );
    }

    #[test]
    fn struct_fields_trailing_comma_ok() {
        parse_ok(
            r#"struct Point { x: Int, y: Int, }
               func main() { let _ = Point { x: 1, y: 2 }; }"#,
        );
    }

    #[test]
    fn struct_fields_comments_and_comma_ok() {
        parse_ok(
            r#"struct Point {
                   x: Int, // x
                   y: Int, // y
               }
               func main() { let _ = Point { x: 1, y: 2 }; }"#,
        );
    }

    #[test]
    fn struct_fields_semicolon_rejected() {
        parse_err_contains(
            r#"struct Point { x: Int; y: Int; }
               func main() {}"#,
            "expected `,` or `}` in struct definition",
        );
    }

    #[test]
    fn struct_fields_missing_comma_rejected() {
        parse_err_contains(
            r#"struct Point { x: Int y: Int }
               func main() {}"#,
            "expected `,` or `}` in struct definition",
        );
    }

    #[test]
    fn if_let_parser_edge_01_nested_enum_struct_ok() {
        parse_ok(
            r#"enum Option<T> { None, Some(T) }
               struct Point { x: Int, y: Int, }
               func main() {
                   if let Option::Some(Point { x, y: _ }) = Option::Some(Point { x: 1, y: 2 }) {
                       let _ = x;
                   } else { }
               }"#,
        );
    }

    #[test]
    fn if_let_parser_edge_02_tuple_and_array_pattern_ok() {
        parse_ok(
            r#"func main() {
                   if let ((a, b), arr) = ((1, 2), [3, 4, 5]) {
                       let _ = a + b;
                       let _ = arr;
                   } else { }
               }"#,
        );
    }

    #[test]
    fn if_let_parser_edge_03_struct_rest_nested_ok() {
        parse_ok(
            r#"struct Inner { a: Int, b: Int, c: Int, }
               struct Wrap { i: Inner, t: Int, }
               func main() {
                   if let Wrap { i: Inner { a, .. }, t: _ } = Wrap { i: Inner { a: 1, b: 2, c: 3 }, t: 4 } {
                       let _ = a;
                   } else { }
               }"#,
        );
    }

    #[test]
    fn if_let_parser_edge_04_missing_equals_rejected() {
        parse_err_contains(
            r#"func main() {
                   if let (a, b) (1, 2) { let _ = a + b; } else { }
               }"#,
            "expected `=`",
        );
    }

    #[test]
    fn if_let_parser_edge_05_missing_else_block_ok() {
        parse_ok(
            r#"func main() {
                   if let [x, y] = [1, 2] {
                       let _ = x + y;
                   }
               }"#,
        );
    }
}

#[cfg(test)]
mod extension_parser_tests {
    use super::*;
    use crate::lexer::Lexer;

    #[test]
    fn parse_extension_decl_and_method_calls() {
        let src = r#"
func String::to_string(self): String { return self; }
func Int::max(a: Int, b: Int): Int { return a; }
func main() {
    let s = "x".to_string();
    let n = Int::max(1, 2);
}
"#;
        let mut lexer = Lexer::new(src);
        let tokens = lexer.tokenize().expect("lex");
        let mut parser = Parser::new(tokens);
        parser.parse().expect("parse");
    }

    #[test]
    fn parse_generic_array_extension_type_keyword() {
        let src = r#"
func [type T]::len<T>(self): Int { return 0; }
func main() {}
"#;
        let mut lexer = Lexer::new(src);
        let tokens = lexer.tokenize().expect("lex");
        let mut parser = Parser::new(tokens);
        parser.parse().expect("parse");
    }

    #[test]
    fn parse_generic_array_extension_enum_type_params() {
        let src = r#"
func [Result<type T1, type T2>]::len<T1, T2>(self: [Result<T1, T2>]): Int { return 0; }
func main() {}
"#;
        let mut lexer = Lexer::new(src);
        let tokens = lexer.tokenize().expect("lex");
        let mut parser = Parser::new(tokens);
        parser.parse().expect("parse");
    }

    #[test]
    fn parse_zero_field_struct_literal_method_chain() {
        let src = r#"
struct Void {}
func Void::bar<T>(self, x: T): T { return x; }
func main() {
    let a = Void{}.bar(1);
    let b = Void {}.bar(2);
}
"#;
        let mut lexer = Lexer::new(src);
        let tokens = lexer.tokenize().expect("lex");
        let mut parser = Parser::new(tokens);
        parser.parse().expect("parse");
    }

    #[test]
    fn parse_generic_struct_decl_and_literals() {
        let src = r#"
struct Generic<T> { a: T, b: T }
struct UnitGeneric<T>;
func main() {
    let g = Generic { a: 1, b: 2 };
    let g2 = Generic<Int> { a: 3, b: 4 };
    let u = UnitGeneric<Int>;
}
"#;
        let mut lexer = Lexer::new(src);
        let tokens = lexer.tokenize().expect("lex");
        let mut parser = Parser::new(tokens);
        parser.parse().expect("parse");
    }

    #[test]
    fn lt_comparison_not_parsed_as_generic_args() {
        let src = r#"
func main() {
    let guess_int: Int = 1;
    let secret: Int = 2;
    if guess_int < secret {
        let _ = 0;
    }
}
"#;
        let mut lexer = Lexer::new(src);
        let tokens = lexer.tokenize().expect("lex");
        let mut parser = Parser::new(tokens);
        parser.parse().expect("parse");
    }

    #[test]
    fn generic_call_with_explicit_type_args_still_parses() {
        let src = r#"
func id<T>(x: T): T { return x; }
func main() {
    let x = id<Int>(1);
}
"#;
        let mut lexer = Lexer::new(src);
        let tokens = lexer.tokenize().expect("lex");
        let mut parser = Parser::new(tokens);
        parser.parse().expect("parse");
    }

    #[test]
    fn parse_function_shorthand_with_equals() {
        let src = r#"
func inc(x: Int): Int = x + 1;
func main() {
    let _ = inc(3);
}
"#;
        let mut lexer = Lexer::new(src);
        let tokens = lexer.tokenize().expect("lex");
        let mut parser = Parser::new(tokens);
        parser.parse().expect("parse");
    }

    #[test]
    fn reject_function_shorthand_with_fat_arrow() {
        let src = r#"
func inc(x: Int): Int => x + 1;
func main() {}
"#;
        let mut lexer = Lexer::new(src);
        let tokens = lexer.tokenize().expect("lex");
        let mut parser = Parser::new(tokens);
        let err = parser.parse().expect_err("must fail");
        match err {
            ParseError::UnexpectedToken { message, .. } => {
                assert!(
                    message.contains("function shorthand now uses `=`"),
                    "unexpected message: {message}"
                );
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn parse_type_alias_simple_and_generic() {
        let src = r#"
type UserId = Int;
type Res<T> = Result<T, String>;
func main() {
    let _: UserId = 1;
    let _: Res<Int> = Result<Int, String>::Ok(1);
}
"#;
        let mut lexer = Lexer::new(src);
        let tokens = lexer.tokenize().expect("lex");
        let mut parser = Parser::new(tokens);
        parser.parse().expect("parse");
    }

    #[test]
    fn reject_type_alias_missing_equals() {
        let src = r#"
type UserId Int;
func main() {}
"#;
        let mut lexer = Lexer::new(src);
        let tokens = lexer.tokenize().expect("lex");
        let mut parser = Parser::new(tokens);
        let err = parser.parse().expect_err("must fail");
        match err {
            ParseError::UnexpectedToken { message, .. } => {
                assert!(message.contains("expected `=`"), "{message}");
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn reject_internal_struct_task_default_and_async_await() {
        let src = r#"
internal struct Task<T = ()>;
internal async func sleep(ms: Int): Task;
async func work(): Task {
    await sleep(0);
    return;
}
async func main(): Task {
    await work();
}
"#;
        let mut lexer = Lexer::new(src);
        let tokens = lexer.tokenize().expect("lex");
        let mut parser = Parser::new(tokens);
        parser
            .parse()
            .expect_err("internal struct declarations must be rejected");
    }

}
