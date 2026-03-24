use crate::ast::{Param, TypeExpr};
use crate::error::Span;
use crate::value::Value;
use num_bigint::BigInt;

use super::{type_name_named, BuiltinError, BuiltinImpl};

pub struct StoiBuiltin;

impl BuiltinImpl for StoiBuiltin {
    fn name(&self) -> &'static str {
        "stoi"
    }

    fn validate_decl(
        &self,
        params: &[Param],
        return_type: &Option<TypeExpr>,
        name_span: Span,
    ) -> Result<(), BuiltinError> {
        if params.len() != 1 || type_name_named(&params[0].ty) != Some("String") {
            return Err(BuiltinError::new(
                "internal `stoi` must be `stoi(s: String): Int`",
                Some(name_span),
            ));
        }
        match return_type {
            Some(t) if type_name_named(t) == Some("Int") => Ok(()),
            _ => Err(BuiltinError::new(
                "internal `stoi` must return `Int`",
                Some(name_span),
            )),
        }
    }

    fn eval(&self, args: Vec<Value>, span: Span) -> Result<Value, BuiltinError> {
        if args.len() != 1 {
            return Err(BuiltinError::new("stoi needs 1 argument", Some(span)));
        }
        let Value::String(s) = args.into_iter().next().unwrap() else {
            return Err(BuiltinError::new("stoi expects String", Some(span)));
        };
        let parsed = s.trim().parse::<BigInt>().map_err(|_| {
            BuiltinError::new(
                "stoi expects a valid Int-formatted string",
                Some(span),
            )
        })?;
        Ok(Value::Int(parsed))
    }
}

