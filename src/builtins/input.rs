use crate::ast::{Param, TypeExpr};
use crate::error::Span;
use crate::value::Value;

use super::{type_name_named, BuiltinError, BuiltinImpl};

pub struct InputBuiltin;

impl BuiltinImpl for InputBuiltin {
    fn name(&self) -> &'static str {
        "input"
    }

    fn validate_decl(
        &self,
        params: &[Param],
        return_type: &Option<TypeExpr>,
        name_span: Span,
    ) -> Result<(), BuiltinError> {
        if !params.is_empty() {
            return Err(BuiltinError::new(
                "internal `input` must be `input(): String`",
                Some(name_span),
            ));
        }
        match return_type {
            Some(t) if type_name_named(t) == Some("String") => Ok(()),
            _ => Err(BuiltinError::new(
                "internal `input` must return `String`",
                Some(name_span),
            )),
        }
    }

    fn eval(&self, args: Vec<Value>, span: Span) -> Result<Value, BuiltinError> {
        if !args.is_empty() {
            return Err(BuiltinError::new("input takes no arguments", Some(span)));
        }
        let mut line = String::new();
        std::io::stdin()
            .read_line(&mut line)
            .map_err(|e| BuiltinError::new(format!("I/O: {e}"), Some(span)))?;
        if line.ends_with('\n') {
            line.pop();
            if line.ends_with('\r') {
                line.pop();
            }
        }
        Ok(Value::String(line))
    }
}

