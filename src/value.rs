//! Shared runtime value representation for VM execution.
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use num_bigint::BigInt;

use crate::async_runtime::TaskId;

#[derive(Debug)]
pub struct StructInstance {
    pub name: String,
    pub is_unit: bool,
    pub fields: HashMap<String, Value>,
}

#[derive(Debug, Clone)]
pub enum Value {
    Int(BigInt),
    String(String),
    Bool(bool),
    Unit,
    Tuple(Vec<Value>),
    Array(Vec<Value>),
    /// Reference-like user struct instance (shared identity).
    Struct(Rc<RefCell<StructInstance>>),
    /// Value-semantic enum value: `EnumName::Variant(payloads...)`.
    Enum {
        enum_name: String,
        variant: String,
        payloads: Vec<Value>,
    },
    Closure {
        callee: String,
        captures: Vec<Value>,
    },
    /// Async task handle (`Task<...>`).
    ///
    /// Actual execution state is stored in the VM's async runtime scheduler.
    Task(TaskId),
}

impl PartialEq for Value {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Value::Int(a), Value::Int(b)) => a == b,
            (Value::String(a), Value::String(b)) => a == b,
            (Value::Bool(a), Value::Bool(b)) => a == b,
            (Value::Unit, Value::Unit) => true,
            (Value::Tuple(a), Value::Tuple(b)) => a == b,
            (Value::Array(a), Value::Array(b)) => a == b,
            (Value::Struct(a), Value::Struct(b)) => Rc::ptr_eq(a, b),
            (
                Value::Enum {
                    enum_name: a_name,
                    variant: a_var,
                    payloads: a_payloads,
                },
                Value::Enum {
                    enum_name: b_name,
                    variant: b_var,
                    payloads: b_payloads,
                },
            ) => a_name == b_name && a_var == b_var && a_payloads == b_payloads,
            (
                Value::Closure {
                    callee: a_callee,
                    captures: a_caps,
                },
                Value::Closure {
                    callee: b_callee,
                    captures: b_caps,
                },
            ) => a_callee == b_callee && a_caps == b_caps,
            (Value::Task(a), Value::Task(b)) => a == b,
            _ => false,
        }
    }
}

impl Eq for Value {}

impl Value {
    /// Deep clone helper used by `clone<T>` builtin: all nested structs are duplicated.
    pub fn deep_clone(&self) -> Value {
        match self {
            Value::Int(i) => Value::Int(i.clone()),
            Value::String(s) => Value::String(s.clone()),
            Value::Bool(b) => Value::Bool(*b),
            Value::Unit => Value::Unit,
            Value::Tuple(parts) => Value::Tuple(parts.iter().map(|v| v.deep_clone()).collect()),
            Value::Array(parts) => Value::Array(parts.iter().map(|v| v.deep_clone()).collect()),
            Value::Struct(rc) => {
                let inst = rc.borrow();
                let mut fields = HashMap::new();
                for (k, v) in inst.fields.iter() {
                    fields.insert(k.clone(), v.deep_clone());
                }
                Value::Struct(Rc::new(RefCell::new(StructInstance {
                    name: inst.name.clone(),
                    is_unit: inst.is_unit,
                    fields,
                })))
            }
            Value::Enum {
                enum_name,
                variant,
                payloads,
            } => Value::Enum {
                enum_name: enum_name.clone(),
                variant: variant.clone(),
                payloads: payloads.iter().map(|v| v.deep_clone()).collect(),
            },
            Value::Closure { callee, captures } => Value::Closure {
                callee: callee.clone(),
                captures: captures.iter().map(|v| v.deep_clone()).collect(),
            },
            Value::Task(id) => Value::Task(*id),
        }
    }
}

