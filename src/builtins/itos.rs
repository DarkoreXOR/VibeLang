use crate::ast::{Param, TypeExpr};
use crate::error::Span;
use crate::value::Value;

use super::{type_name_named, BuiltinError, BuiltinImpl};

pub struct ItosBuiltin;

impl BuiltinImpl for ItosBuiltin {
    fn name(&self) -> &'static str {
        "itos"
    }

    fn validate_decl(
        &self,
        params: &[Param],
        return_type: &Option<TypeExpr>,
        name_span: Span,
    ) -> Result<(), BuiltinError> {
        if params.len() != 1 || type_name_named(&params[0].ty) != Some("Int") {
            return Err(BuiltinError::new(
                "internal `itos` must be `itos(s: Int): String`",
                Some(name_span),
            ));
        }
        match return_type {
            Some(t) if type_name_named(t) == Some("String") => Ok(()),
            _ => Err(BuiltinError::new(
                "internal `itos` must return `String`",
                Some(name_span),
            )),
        }
    }

    fn eval(&self, args: Vec<Value>, span: Span) -> Result<Value, BuiltinError> {
        if args.len() != 1 {
            return Err(BuiltinError::new("itos needs 1 argument", Some(span)));
        }
        let Value::Int(i) = args.into_iter().next().unwrap() else {
            return Err(BuiltinError::new("itos expects Int", Some(span)));
        };
        Ok(Value::String(i.to_string()))
    }
}

