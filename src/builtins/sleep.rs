use crate::ast::{Param, TypeExpr};
use crate::builtins::{type_name_named, BuiltinError, BuiltinImpl};
use crate::error::Span;
use crate::value::Value;

/// `internal async func sleep(milliseconds: Int): Task`
///
/// In the VM scheduler this async callee is treated as a non-blocking timer.
/// This builtin's `eval` returns the payload value (`Unit`) and is not expected
/// to perform any blocking work.
pub struct SleepBuiltin;

impl BuiltinImpl for SleepBuiltin {
    fn name(&self) -> &'static str {
        "sleep"
    }

    fn validate_decl(
        &self,
        params: &[Param],
        return_type: &Option<TypeExpr>,
        name_span: Span,
    ) -> Result<(), BuiltinError> {
        if params.len() != 1 {
            return Err(BuiltinError::new(
                "internal `sleep` must be `sleep(milliseconds: Int): Task`",
                Some(name_span),
            ));
        }
        if params[0].is_params || params[0].is_wildcard {
            return Err(BuiltinError::new(
                "internal `sleep` must be `sleep(milliseconds: Int): Task`",
                Some(name_span),
            ));
        }
        if type_name_named(&params[0].ty) != Some("Int") {
            return Err(BuiltinError::new(
                "internal `sleep` must be `sleep(milliseconds: Int): Task`",
                Some(name_span),
            ));
        }
        let Some(rt) = return_type else {
            return Err(BuiltinError::new(
                "internal `sleep` must return `Task`",
                Some(name_span),
            ));
        };
        match rt {
            TypeExpr::Named(n) if n == "Task" => Ok(()),
            TypeExpr::EnumApp { name, .. } if name == "Task" => Ok(()),
            _ => Err(BuiltinError::new(
                "internal `sleep` must return `Task` or `Task<...>`",
                Some(name_span),
            )),
        }
    }

    fn eval(&self, args: Vec<Value>, span: Span) -> Result<Value, BuiltinError> {
        use num_traits::ToPrimitive;
        let ms = match args.into_iter().next() {
            Some(Value::Int(i)) => i.to_u64().unwrap_or(u64::MAX),
            _ => {
                return Err(BuiltinError::new(
                    "internal `sleep` expects an Int",
                    Some(span),
                ))
            }
        };
        // The VM handles the actual timer/wake-up. Here we just return the
        // payload for `Task<()>` so the async callee can complete.
        let _ = ms;
        Ok(Value::Unit)
    }
}
