use crate::ast::{Param, TypeExpr};
use crate::builtins::{BuiltinError, BuiltinImpl};
use crate::error::Span;
use crate::value::Value;

/// `internal async func fetch(request: Request): Task<Response>`
pub struct FetchBuiltin;

fn is_named(te: &TypeExpr, name: &str) -> bool {
    matches!(te, TypeExpr::Named(n) if n == name)
}

impl BuiltinImpl for FetchBuiltin {
    fn name(&self) -> &'static str {
        "fetch"
    }

    fn validate_decl(
        &self,
        params: &[Param],
        return_type: &Option<TypeExpr>,
        name_span: Span,
    ) -> Result<(), BuiltinError> {
        if params.len() != 1 {
            return Err(BuiltinError::new(
                "internal `fetch` must take one `request: Request` parameter",
                Some(name_span),
            ));
        }
        if params[0].is_params || params[0].is_wildcard || !is_named(&params[0].ty, "Request") {
            return Err(BuiltinError::new(
                "internal `fetch` must be declared as `fetch(request: Request): Task<Response>`",
                Some(name_span),
            ));
        }

        let Some(rt) = return_type else {
            return Err(BuiltinError::new(
                "internal `fetch` must return `Task<Response>`",
                Some(name_span),
            ));
        };
        match rt {
            TypeExpr::EnumApp { name, args } if name == "Task" && args.len() == 1 => {
                if is_named(&args[0], "Response") {
                    Ok(())
                } else {
                    Err(BuiltinError::new(
                        "internal `fetch` must return `Task<Response>`",
                        Some(name_span),
                    ))
                }
            }
            _ => Err(BuiltinError::new(
                "internal `fetch` must return `Task<Response>`",
                Some(name_span),
            )),
        }
    }

    fn eval(&self, _args: Vec<Value>, span: Span) -> Result<Value, BuiltinError> {
        Err(BuiltinError::new(
            "internal async `fetch` is VM-scheduled and cannot be called synchronously",
            Some(span),
        ))
    }
}

