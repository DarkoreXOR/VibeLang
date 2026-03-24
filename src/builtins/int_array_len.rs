use crate::ast::{Param, TypeExpr};
use crate::builtins::{BuiltinError, BuiltinImpl};
use crate::error::Span;
use crate::value::Value;

pub struct IntArrayLenBuiltin;

impl BuiltinImpl for IntArrayLenBuiltin {
    fn name(&self) -> &'static str {
        "int_array_len"
    }

    fn validate_decl(
        &self,
        params: &[Param],
        return_type: &Option<TypeExpr>,
        name_span: Span,
    ) -> Result<(), BuiltinError> {
        if params.len() != 1 {
            return Err(BuiltinError::new(
                "int_array_len expects exactly one parameter",
                Some(name_span),
            ));
        }
        if !matches!(params[0].ty, TypeExpr::Array(_)) {
            return Err(BuiltinError::new(
                "int_array_len expects an array parameter",
                Some(name_span),
            ));
        }
        if let Some(rt) = return_type {
            if !matches!(rt, TypeExpr::Named(n) if n == "Int") {
                return Err(BuiltinError::new(
                    "int_array_len return type must be `Int`",
                    Some(name_span),
                ));
            }
        } else {
            return Err(BuiltinError::new(
                "int_array_len return type must be `Int`",
                Some(name_span),
            ));
        }
        Ok(())
    }

    fn eval(&self, args: Vec<Value>, span: Span) -> Result<Value, BuiltinError> {
        let a = args.into_iter().next().unwrap_or(Value::Unit);
        let len = match a {
            Value::Array(v) => v.len(),
            _ => {
                return Err(BuiltinError::new(
                    "int_array_len expects an array",
                    Some(span),
                ))
            }
        };
        Ok(Value::Int(len.into()))
    }
}

