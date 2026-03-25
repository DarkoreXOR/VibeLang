use crate::ast::{Param, TypeExpr};
use crate::builtins::{BuiltinError, BuiltinImpl};
use crate::error::Span;
use crate::value::Value;

/// `internal async func create_completed_task_async<T>(value: T): Task<T>`
///
/// In the async runtime scheduler this callee is treated as a completed task
/// handle that becomes available immediately.
pub struct CreateCompletedTaskBuiltin;

impl BuiltinImpl for CreateCompletedTaskBuiltin {
    fn name(&self) -> &'static str {
        "create_completed_task_async"
    }

    fn validate_decl(
        &self,
        params: &[Param],
        return_type: &Option<TypeExpr>,
        name_span: Span,
    ) -> Result<(), BuiltinError> {
        if params.len() != 1 {
            return Err(BuiltinError::new(
                "internal `create_completed_task_async` must take one `value` parameter",
                Some(name_span),
            ));
        }
        if params[0].is_params || params[0].is_wildcard {
            return Err(BuiltinError::new(
                "internal `create_completed_task_async` must take a single value parameter",
                Some(name_span),
            ));
        }

        let Some(rt) = return_type else {
            return Err(BuiltinError::new(
                "internal `create_completed_task_async` must return `Task`",
                Some(name_span),
            ));
        };

        match rt {
            TypeExpr::Named(n) if n == "Task" => Ok(()),
            TypeExpr::EnumApp { name, .. } if name == "Task" => Ok(()),
            _ => Err(BuiltinError::new(
                "internal `create_completed_task_async` must return `Task` or `Task<...>`",
                Some(name_span),
            )),
        }
    }

    fn eval(&self, args: Vec<Value>, span: Span) -> Result<Value, BuiltinError> {
        // This callee is async and should be used via VM task spawning, not
        // directly as a synchronous builtin call.
        let _ = args;
        Err(BuiltinError::new(
            "internal `create_completed_task_async` must be awaited (via task spawning)",
            Some(span),
        ))
    }
}

