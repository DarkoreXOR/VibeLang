use crate::ast::{Param, TypeExpr};
use crate::error::Span;
use crate::value::Value;
use num_bigint::{BigInt, Sign};
use num_traits::{ToPrimitive, Zero};

use super::{type_name_named, BuiltinError, BuiltinImpl};

pub struct RandBigIntBuiltin;

impl BuiltinImpl for RandBigIntBuiltin {
    fn name(&self) -> &'static str {
        "rand_bigint"
    }

    fn validate_decl(
        &self,
        params: &[Param],
        return_type: &Option<TypeExpr>,
        name_span: Span,
    ) -> Result<(), BuiltinError> {
        if params.len() != 1 || type_name_named(&params[0].ty) != Some("Int") {
            return Err(BuiltinError::new(
                "internal `rand_bigint` must be `rand_bigint(bits: Int): Int`",
                Some(name_span),
            ));
        }
        match return_type {
            Some(t) if type_name_named(t) == Some("Int") => Ok(()),
            _ => Err(BuiltinError::new(
                "internal `rand_bigint` must return `Int`",
                Some(name_span),
            )),
        }
    }

    fn eval(&self, args: Vec<Value>, span: Span) -> Result<Value, BuiltinError> {
        if args.len() != 1 {
            return Err(BuiltinError::new("rand_bigint needs 1 argument", Some(span)));
        }
        let Value::Int(bits) = args.into_iter().next().unwrap() else {
            return Err(BuiltinError::new("rand_bigint expects Int", Some(span)));
        };
        if bits <= BigInt::zero() {
            return Err(BuiltinError::new(
                "rand_bigint expects `bits > 0`",
                Some(span),
            ));
        }

        // Safety cap to avoid pathological allocations.
        let bits_u32 = bits.to_u32().ok_or_else(|| {
            BuiltinError::new("rand_bigint bits out of supported range", Some(span))
        })?;
        if bits_u32 > 1_000_000 {
            return Err(BuiltinError::new(
                "rand_bigint bits too large (max 1000000)",
                Some(span),
            ));
        }

        let byte_len = (bits_u32 as usize).div_ceil(8);
        let mut bytes = vec![0u8; byte_len];
        for b in &mut bytes {
            *b = rand::random::<u8>();
        }

        // Mask high bits so generated value has at most `bits`.
        let extra = (8 - (bits_u32 % 8)) % 8;
        if extra > 0 {
            bytes[0] &= 0xFF >> extra;
        }

        // Random sign for non-zero values.
        let mut out = BigInt::from_bytes_be(Sign::Plus, &bytes);
        if !out.is_zero() && rand::random::<bool>() {
            out = -out;
        }
        Ok(Value::Int(out))
    }
}

