use std::thread;
use std::time::Duration;

use crate::ast::{Param, TypeExpr};
use crate::builtins::{type_name_named, BuiltinError, BuiltinImpl};
use crate::error::Span;
use crate::value::{TaskInner, Value};

/// `internal async func sleep(milliseconds: Int): Task` — blocks the thread for `ms` ms, returns a completed `Task<()>`.
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
        thread::sleep(Duration::from_millis(ms.min(u64::from(u32::MAX))));
        Ok(Value::Task(TaskInner::completed(Value::Unit)))
    }
}
