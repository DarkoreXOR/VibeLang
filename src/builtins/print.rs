use crate::ast::{Param, TypeExpr};
use crate::error::Span;
use crate::value::Value;

use super::{type_name_named, BuiltinError, BuiltinImpl};

pub struct PrintBuiltin;

impl BuiltinImpl for PrintBuiltin {
    fn name(&self) -> &'static str {
        "print"
    }

    fn validate_decl(
        &self,
        params: &[Param],
        return_type: &Option<TypeExpr>,
        name_span: Span,
    ) -> Result<(), BuiltinError> {
        if params.len() != 1 || type_name_named(&params[0].ty) != Some("String") {
            return Err(BuiltinError::new(
                "internal `print` must be `print(s: String);`",
                Some(name_span),
            ));
        }
        if return_type.is_some() {
            return Err(BuiltinError::new(
                "internal `print` must have no return type",
                Some(name_span),
            ));
        }
        Ok(())
    }

    fn eval(&self, args: Vec<Value>, span: Span) -> Result<Value, BuiltinError> {
        if args.len() != 1 {
            return Err(BuiltinError::new("print needs 1 argument", Some(span)));
        }
        let Value::String(st) = args.into_iter().next().unwrap() else {
            return Err(BuiltinError::new("print expects String", Some(span)));
        };
        use std::io::Write;
        let mut stdout = std::io::stdout().lock();
        stdout
            .write_all(st.as_bytes())
            .map_err(|e| BuiltinError::new(format!("I/O: {e}"), Some(span)))?;
        stdout
            .write_all(b"\n")
            .map_err(|e| BuiltinError::new(format!("I/O: {e}"), Some(span)))?;
        Ok(Value::Unit)
    }
}

