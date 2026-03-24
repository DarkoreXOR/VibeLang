use crate::ast::{Param, TypeExpr};
use crate::error::Span;
use crate::value::Value;
use num_traits::{ToPrimitive, Zero};

use super::{type_name_named, BuiltinError, BuiltinImpl};

pub struct RandIntBuiltin;

impl BuiltinImpl for RandIntBuiltin {
    fn name(&self) -> &'static str {
        "rand_int"
    }

    fn validate_decl(
        &self,
        params: &[Param],
        return_type: &Option<TypeExpr>,
        name_span: Span,
    ) -> Result<(), BuiltinError> {
        if params.len() != 1 || type_name_named(&params[0].ty) != Some("Int") {
            return Err(BuiltinError::new(
                "internal `rand_int` must be `rand_int(to: Int): Int`",
                Some(name_span),
            ));
        }
        match return_type {
            Some(t) if type_name_named(t) == Some("Int") => Ok(()),
            _ => Err(BuiltinError::new(
                "internal `rand_int` must return `Int`",
                Some(name_span),
            )),
        }
    }

    fn eval(&self, args: Vec<Value>, span: Span) -> Result<Value, BuiltinError> {
        if args.len() != 1 {
            return Err(BuiltinError::new("rand_int needs 1 argument", Some(span)));
        }
        let Value::Int(to) = args.into_iter().next().unwrap() else {
            return Err(BuiltinError::new("rand_int expects Int", Some(span)));
        };
        if to <= num_bigint::BigInt::zero() {
            return Err(BuiltinError::new("rand_int expects `to > 0`", Some(span)));
        }
        let to_u64 = to.to_u64().ok_or_else(|| {
            BuiltinError::new("rand_int bound is too large", Some(span))
        })?;
        let r = rand::random::<u64>() % to_u64;
        Ok(Value::Int(r.into()))
    }
}

