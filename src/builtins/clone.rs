use crate::ast::{Param, TypeExpr};
use crate::builtins::{BuiltinError, BuiltinImpl};
use crate::error::Span;
use crate::value::Value;

/// `clone<T>(value: T): T;` deep clones recursively.
pub struct CloneBuiltin;

impl CloneBuiltin {
    fn validate_same_type(param_ty: &TypeExpr, return_ty: &Option<TypeExpr>) -> bool {
        match return_ty {
            Some(rt) => rt == param_ty,
            None => false,
        }
    }
}

impl BuiltinImpl for CloneBuiltin {
    fn name(&self) -> &'static str {
        "clone"
    }

    fn validate_decl(
        &self,
        params: &[Param],
        return_type: &Option<TypeExpr>,
        name_span: Span,
    ) -> Result<(), BuiltinError> {
        if params.len() != 1 {
            return Err(BuiltinError::new(
                "`clone` must have exactly 1 parameter",
                Some(name_span),
            ));
        }
        if !Self::validate_same_type(&params[0].ty, return_type) {
            return Err(BuiltinError::new(
                "internal `clone` must be `clone<T>(value: T): T;`",
                Some(name_span),
            ));
        }
        Ok(())
    }

    fn eval(&self, args: Vec<Value>, span: Span) -> Result<Value, BuiltinError> {
        if args.len() != 1 {
            return Err(BuiltinError::new("clone needs 1 argument", Some(span)));
        }
        let v = args.into_iter().next().unwrap();
        Ok(v.deep_clone())
    }
}

