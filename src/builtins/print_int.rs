use crate::ast::{Param, TypeExpr};
use crate::error::Span;
use crate::value::Value;

use super::{type_name_named, BuiltinError, BuiltinImpl};

pub struct PrintIntBuiltin;

impl BuiltinImpl for PrintIntBuiltin {
    fn name(&self) -> &'static str {
        "print_int"
    }

    fn validate_decl(
        &self,
        params: &[Param],
        return_type: &Option<TypeExpr>,
        name_span: Span,
    ) -> Result<(), BuiltinError> {
        if params.len() != 1 || type_name_named(&params[0].ty) != Some("Int") {
            return Err(BuiltinError::new(
                "internal `print_int` must be `print_int(v: Int);`",
                Some(name_span),
            ));
        }
        if return_type.is_some() {
            return Err(BuiltinError::new(
                "internal `print_int` must have no return type",
                Some(name_span),
            ));
        }
        Ok(())
    }

    fn eval(&self, args: Vec<Value>, span: Span) -> Result<Value, BuiltinError> {
        if args.len() != 1 {
            return Err(BuiltinError::new("print_int needs 1 argument", Some(span)));
        }
        let Value::Int(i) = args.into_iter().next().unwrap() else {
            return Err(BuiltinError::new("print_int expects Int", Some(span)));
        };
        println!("{i}");
        Ok(Value::Unit)
    }
}

