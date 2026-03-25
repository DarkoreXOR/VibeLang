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
mod wait_all_tasks;
mod create_completed_task;
mod dict;

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
use wait_all_tasks::WaitAllTasksBuiltin;
use create_completed_task::CreateCompletedTaskBuiltin;
use dict::{DictContainsBuiltin, DictGetBuiltin, DictInsertBuiltin, DictRemoveBuiltin};

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

// Async task driving is handled by the VM's async runtime scheduler.

fn canonical_builtin_name<'a>(name: &'a str) -> &'a str {
    match name {
        "sleep_async" => "sleep",
        x => x,
    }
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
        let key = canonical_builtin_name(name);
        self.builtins.get(key).map(|b| b.as_ref())
    }

    pub fn resolve_internal_async_callee(
        &self,
        params: &[Param],
        return_type: &Option<TypeExpr>,
        name_span: Span,
    ) -> Result<&'static str, BuiltinError> {
        if let Some(b) = self.builtins.get("sleep") {
            if b.validate_decl(params, return_type, name_span).is_ok() {
                return Ok("sleep");
            }
        }
        if let Some(b) = self.builtins.get("wait_all_tasks_async") {
            if b.validate_decl(params, return_type, name_span).is_ok() {
                return Ok("wait_all_tasks_async");
            }
        }
        if let Some(b) = self.builtins.get("create_completed_task_async") {
            if b.validate_decl(params, return_type, name_span).is_ok() {
                return Ok("create_completed_task_async");
            }
        }
        Err(BuiltinError::new(
            "internal `async func` must match built-in `sleep` or `wait_all_tasks_async` signatures",
            Some(name_span),
        ))
    }

    pub fn validate_internal_func_decl(
        &self,
        name: &str,
        params: &[Param],
        return_type: &Option<TypeExpr>,
        name_span: Span,
        is_async: bool,
    ) -> Result<(), BuiltinError> {
        if is_async {
            self.resolve_internal_async_callee(params, return_type, name_span)?;
            return Ok(());
        }
        if name == "sleep" && !is_async {
            return Err(BuiltinError::new(
                "internal `sleep` must be declared `internal async func sleep(...): Task;`",
                Some(name_span),
            ));
        }
        let key = canonical_builtin_name(name);
        let Some(b) = self.builtins.get(key) else {
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
        let key = canonical_builtin_name(name);
        let Some(b) = self.builtins.get(key) else {
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
    reg.register(WaitAllTasksBuiltin);
    reg.register(CreateCompletedTaskBuiltin);
    reg.register(DictContainsBuiltin);
    reg.register(DictGetBuiltin);
    reg.register(DictInsertBuiltin);
    reg.register(DictRemoveBuiltin);
    reg
}

static DEFAULT_REGISTRY: OnceLock<BuiltinRegistry> = OnceLock::new();

pub fn default_registry_ref() -> &'static BuiltinRegistry {
    DEFAULT_REGISTRY.get_or_init(default_registry_impl)
}

