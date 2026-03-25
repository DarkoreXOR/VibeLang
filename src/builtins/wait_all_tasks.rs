use crate::ast::{Param, TypeExpr};
use crate::builtins::{BuiltinError, BuiltinImpl};
use crate::error::Span;
use crate::value::Value;

/// `internal async func wait_all_tasks_async<T>(tasks: [Task<T>]): Task`
///
/// In the new cooperative scheduler, the VM will not rely on this builtin's
/// `eval` to implement concurrency. It may be used for signature validation
/// and should return the payload value (`Unit`) if invoked.
pub struct WaitAllTasksBuiltin;

fn task_element_type_ok(ty: &TypeExpr) -> bool {
    matches!(ty, TypeExpr::Named(n) if n == "Task")
        || matches!(ty, TypeExpr::EnumApp { name, .. } if name == "Task")
}

impl BuiltinImpl for WaitAllTasksBuiltin {
    fn name(&self) -> &'static str {
        "wait_all_tasks_async"
    }

    fn validate_decl(
        &self,
        params: &[Param],
        return_type: &Option<TypeExpr>,
        name_span: Span,
    ) -> Result<(), BuiltinError> {
        if params.len() != 1 {
            return Err(BuiltinError::new(
                "internal `wait_all_tasks_async` must take one `tasks` array parameter",
                Some(name_span),
            ));
        }
        if params[0].is_params || params[0].is_wildcard {
            return Err(BuiltinError::new(
                "internal `wait_all_tasks_async` must take `tasks: [Task<...>]`",
                Some(name_span),
            ));
        }
        let ok_elem = match &params[0].ty {
            TypeExpr::Array(inner) => task_element_type_ok(inner.as_ref()),
            _ => false,
        };
        if !ok_elem {
            return Err(BuiltinError::new(
                "internal `wait_all_tasks_async` must take `tasks: [Task<...>]`",
                Some(name_span),
            ));
        }
        let Some(rt) = return_type else {
            return Err(BuiltinError::new(
                "internal `wait_all_tasks_async` must return `Task`",
                Some(name_span),
            ));
        };
        match rt {
            TypeExpr::Named(n) if n == "Task" => Ok(()),
            TypeExpr::EnumApp { name, .. } if name == "Task" => Ok(()),
            _ => Err(BuiltinError::new(
                "internal `wait_all_tasks_async` must return `Task` or `Task<...>`",
                Some(name_span),
            )),
        }
    }

    fn eval(&self, args: Vec<Value>, span: Span) -> Result<Value, BuiltinError> {
        let _tasks = match args.into_iter().next() {
            Some(Value::Array(elements)) => elements,
            _ => {
                return Err(BuiltinError::new(
                    "`wait_all_tasks_async` expects an array of tasks",
                    Some(span),
                ))
            }
        };

        // If invoked, treat it as already complete.
        // (VM special-cases this callee to ensure correct async behavior.)
        Ok(Value::Unit)
    }
}
