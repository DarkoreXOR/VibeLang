//! Internal-function (builtin) validation + dispatch.
//!
//! Builtins are registered into a `BuiltinRegistry`. This keeps VM dispatch
//! logic centralized and is extendable for plugins later.

use std::collections::HashMap;
use std::sync::OnceLock;

use crate::ast::{Param, TypeExpr};
use crate::error::Span;
use crate::value::Value;

mod concat;
mod input;
mod clone;
mod itos;
mod int_array_len;
mod print_any;
mod print;
mod print_int;
mod print_gen;
mod rand_int;
mod rand_bigint;
mod stoi;
mod sleep;

use concat::ConcatBuiltin;
use input::InputBuiltin;
use itos::ItosBuiltin;
use int_array_len::IntArrayLenBuiltin;
use print_any::PrintAnyBuiltin;
use print::PrintBuiltin;
use print_int::PrintIntBuiltin;
use clone::CloneBuiltin;
use print_gen::PrintGenBuiltin;
use rand_int::RandIntBuiltin;
use rand_bigint::RandBigIntBuiltin;
use stoi::StoiBuiltin;
use sleep::SleepBuiltin;

#[derive(Debug)]
pub struct BuiltinError {
    pub message: String,
    pub span: Option<Span>,
}

impl BuiltinError {
    pub fn new(message: impl Into<String>, span: Option<Span>) -> Self {
        Self {
            message: message.into(),
            span,
        }
    }
}

pub trait BuiltinImpl: Send + Sync {
    fn name(&self) -> &'static str;

    fn validate_decl(
        &self,
        params: &[Param],
        return_type: &Option<TypeExpr>,
        name_span: Span,
    ) -> Result<(), BuiltinError>;

    fn eval(&self, args: Vec<Value>, span: Span) -> Result<Value, BuiltinError>;
}

pub struct BuiltinRegistry {
    builtins: HashMap<&'static str, Box<dyn BuiltinImpl>>,
}

impl BuiltinRegistry {
    pub fn new() -> Self {
        Self {
            builtins: HashMap::new(),
        }
    }

    pub fn register<B: BuiltinImpl + 'static>(&mut self, builtin: B) {
        let name = builtin.name();
        self.builtins.insert(name, Box::new(builtin));
    }

    pub fn get(&self, name: &str) -> Option<&dyn BuiltinImpl> {
        self.builtins.get(name).map(|b| b.as_ref())
    }

    pub fn validate_internal_func_decl(
        &self,
        name: &str,
        params: &[Param],
        return_type: &Option<TypeExpr>,
        name_span: Span,
        is_async: bool,
    ) -> Result<(), BuiltinError> {
        if is_async && name != "sleep" {
            return Err(BuiltinError::new(
                format!("internal `async func` is only supported for `sleep` (found `{name}`)"),
                Some(name_span),
            ));
        }
        if name == "sleep" && !is_async {
            return Err(BuiltinError::new(
                "internal `sleep` must be declared `internal async func sleep(...): Task;`",
                Some(name_span),
            ));
        }
        let Some(b) = self.get(name) else {
            return Err(BuiltinError::new(
                format!("unsupported internal function `{name}` (no builtin registered)"),
                Some(name_span),
            ));
        };
        b.validate_decl(params, return_type, name_span)
    }

    pub fn eval_builtin(
        &self,
        name: &str,
        args: Vec<Value>,
        span: Span,
    ) -> Result<Value, BuiltinError> {
        let Some(b) = self.get(name) else {
            return Err(BuiltinError::new(
                format!("unsupported builtin `{name}`"),
                Some(span),
            ));
        };
        b.eval(args, span)
    }
}

pub(super) fn type_name_named(te: &TypeExpr) -> Option<&str> {
    match te {
        TypeExpr::Named(n) => Some(n.as_str()),
        _ => None,
    }
}

fn default_registry_impl() -> BuiltinRegistry {
    let mut reg = BuiltinRegistry::new();
    reg.register(PrintBuiltin);
    reg.register(PrintIntBuiltin);
    reg.register(ItosBuiltin);
    reg.register(IntArrayLenBuiltin);
    reg.register(PrintAnyBuiltin);
    reg.register(CloneBuiltin);
    reg.register(PrintGenBuiltin);
    reg.register(ConcatBuiltin);
    reg.register(InputBuiltin);
    reg.register(StoiBuiltin);
    reg.register(RandIntBuiltin);
    reg.register(RandBigIntBuiltin);
    reg.register(SleepBuiltin);
    reg
}

static DEFAULT_REGISTRY: OnceLock<BuiltinRegistry> = OnceLock::new();

pub fn default_registry_ref() -> &'static BuiltinRegistry {
    DEFAULT_REGISTRY.get_or_init(default_registry_impl)
}

