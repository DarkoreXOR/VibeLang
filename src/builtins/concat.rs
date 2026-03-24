use crate::ast::{Param, TypeExpr};
use crate::error::Span;
use crate::value::Value;

use super::{type_name_named, BuiltinError, BuiltinImpl};

pub struct ConcatBuiltin;

impl BuiltinImpl for ConcatBuiltin {
    fn name(&self) -> &'static str {
        "concat"
    }

    fn validate_decl(
        &self,
        params: &[Param],
        return_type: &Option<TypeExpr>,
        name_span: Span,
    ) -> Result<(), BuiltinError> {
        if params.len() != 2
            || type_name_named(&params[0].ty) != Some("String")
            || type_name_named(&params[1].ty) != Some("String")
        {
            return Err(BuiltinError::new(
                "internal `concat` must be `concat(s1: String, s2: String): String`",
                Some(name_span),
            ));
        }
        match return_type {
            Some(t) if type_name_named(t) == Some("String") => Ok(()),
            _ => Err(BuiltinError::new(
                "internal `concat` must return `String`",
                Some(name_span),
            )),
        }
    }

    fn eval(&self, args: Vec<Value>, span: Span) -> Result<Value, BuiltinError> {
        if args.len() != 2 {
            return Err(BuiltinError::new("concat needs 2 arguments", Some(span)));
        }
        let a = match &args[0] {
            Value::String(s) => s.clone(),
            _ => return Err(BuiltinError::new("concat expects String", Some(span))),
        };
        let b = match &args[1] {
            Value::String(s) => s.clone(),
            _ => return Err(BuiltinError::new("concat expects String", Some(span))),
        };
        Ok(Value::String(format!("{a}{b}")))
    }
}

