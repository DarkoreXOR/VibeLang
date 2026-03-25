use crate::ast::{Param, TypeExpr};
use crate::builtins::{BuiltinError, BuiltinImpl};
use crate::error::Span;
use crate::value::Value;

/// `print_gen<T>(value: T);`
///
/// Generic builtin currently has no compile-time constraints; it just prints
/// whatever runtime value it receives.
pub struct PrintGenBuiltin;

impl PrintGenBuiltin {
    fn fmt_value(v: &Value) -> String {
        match v {
            Value::Int(i) => format!("{i}"),
            Value::Float(f) => f.to_string(),
            Value::String(s) => s.clone(),
            Value::Bool(b) => format!("{b}"),
            Value::Unit => "()".to_string(),
            Value::Tuple(parts) => {
                let inner: Vec<String> = parts.iter().map(Self::fmt_value).collect();
                format!("({})", inner.join(", "))
            }
            Value::Array(parts) => {
                let inner: Vec<String> = parts.iter().map(Self::fmt_value).collect();
                format!("[{}]", inner.join(", "))
            }
            Value::Struct(rc) => {
                let inst = rc.borrow();
                if inst.is_unit {
                    return inst.name.clone();
                }
                let mut parts: Vec<String> = inst
                    .fields
                    .iter()
                    .map(|(k, v)| format!("{k}: {}", Self::fmt_value(v)))
                    .collect();
                parts.sort();
                format!("{}{{{}}}", inst.name, parts.join(", "))
            }
            Value::Enum {
                enum_name,
                variant,
                payloads,
            } => {
                if payloads.is_empty() {
                    format!("{}::{}", enum_name, variant)
                } else {
                    let inner: Vec<String> =
                        payloads.iter().map(|v| Self::fmt_value(v)).collect();
                    format!("{}::{}({})", enum_name, variant, inner.join(", "))
                }
            }
            Value::Closure { callee, .. } => format!("<closure:{callee}>"),
            Value::Task(_) => "<task>".to_string(),
        }
    }
}

impl BuiltinImpl for PrintGenBuiltin {
    fn name(&self) -> &'static str {
        "print_gen"
    }

    fn validate_decl(
        &self,
        params: &[Param],
        return_type: &Option<TypeExpr>,
        name_span: Span,
    ) -> Result<(), BuiltinError> {
        // Generic type parameter(s) are validated by the semantic checker.
        if params.len() != 1 {
            return Err(BuiltinError::new(
                "`print_gen` must have exactly 1 parameter",
                Some(name_span),
            ));
        }
        if return_type.is_some() {
            return Err(BuiltinError::new(
                "internal `print_gen` must have no return type",
                Some(name_span),
            ));
        }
        Ok(())
    }

    fn eval(&self, args: Vec<Value>, span: Span) -> Result<Value, BuiltinError> {
        if args.len() != 1 {
            return Err(BuiltinError::new(
                "print_gen needs 1 argument",
                Some(span),
            ));
        }
        let v = args.into_iter().next().unwrap();
        println!("{}", Self::fmt_value(&v));
        Ok(Value::Unit)
    }
}

