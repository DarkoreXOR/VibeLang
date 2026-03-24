//! Stack-based bytecode IR for Vibelang.

use crate::ast::{BinaryOp, UnaryOp};
use crate::error::Span;

#[derive(Debug, Clone)]
pub enum Instr {
    // Constants
    PushInt {
        value: String,
        radix: u32,
        span: Span,
    },
    PushString { value: String },
    PushBool { value: bool },
    PushUnit,

    // Tuples
    MakeTuple { count: usize },
    GetTupleField { index: u32, span: Span },
    /// Extract tuple[n - 1 - offset_from_end].
    GetTupleFieldFromEnd {
        offset_from_end: u32,
        span: Span,
    },

    // Arrays
    MakeArray { count: usize },
    /// array[index] with runtime negative/positive indexing + bounds checks.
    /// Stack: [..., array, index] -> pushes element.
    GetArrayIndex { span: Span },
    /// Extract array[len - 1 - offset_from_end].
    /// Stack: [..., array] -> pushes element.
    GetArrayIndexFromEnd {
        offset_from_end: u32,
        span: Span,
    },
    /// array[index] = value (immutable update).
    /// Stack: [..., array, index, value] -> pushes updated array.
    ArrayStore { span: Span },
    /// Runtime check used by array destructuring patterns.
    /// Pops array; errors if `len != expected`.
    AssertArrayLenEq { expected: usize, span: Span },

    // Structs (reference types)
    /// `Name { x: v1, y: v2, ..base? }`
    /// Stack:
    /// - without update: [..., v1, v2, ...] -> pushes struct value
    /// - with update:    [..., base, v1, v2, ...] -> pushes struct value
    MakeStructLiteral {
        name: String,
        is_unit_literal: bool,
        field_names: Vec<String>,
        has_update: bool,
        span: Span,
    },
    /// `base.field` — pops a struct value and pushes the field value.
    GetStructField { field: String, span: Span },
    /// `base.field = value` — pops (struct, value), mutates struct in place.
    StructFieldStore { field: String, span: Span },
    /// Pops one value and checks whether it's a struct with the exact concrete name.
    /// Pushes `true` on match, otherwise `false`.
    MatchStructName { name: String, span: Span },

    // Enums (value semantics)
    /// `EnumName::Variant(payloads...)`
    /// Stack: [..., payload1, payload2, ...] -> pushes enum value.
    MakeEnumVariant {
        enum_name: String,
        variant: String,
        payload_count: usize,
        span: Span,
    },

    /// `if let EnumName::Variant(p1, p2, ...) = value { ... }`
    ///
    /// Pops one enum value and checks its tag:
    /// - on success: pushes payload values in order, then pushes `true`
    /// - on failure: pushes only `false`
    ///
    /// Stack effect:
    /// - success: [..., payload1, payload2, ..., true]
    /// - failure: [..., false]
    MatchEnumVariant {
        enum_name: String,
        variant: String,
        payload_count: usize,
        span: Span,
    },

    /// Pops an enum value and unpacks its payloads.
    ///
    /// This is intended for contexts where the variant is expected
    /// to match (e.g. `let Enum::Variant(x) = ...` destructuring).
    /// On mismatch, the VM returns a runtime error.
    UnpackEnumVariant {
        enum_name: String,
        variant: String,
        payload_count: usize,
        span: Span,
    },

    // Operators
    BinOp { op: BinaryOp, span: Span },
    UnOp { op: UnaryOp, span: Span },

    // Stack management
    Pop,
    /// Runtime assertion on a Bool stack value.
    /// Stack: [..., bool] -> []
    AssertBool { span: Span },

    // Control flow
    Jump { target: usize },
    JumpIfFalse { target: usize },
    JumpIfTrue { target: usize },

    // Locals / globals
    LoadLocal { slot: u32, span: Span },
    StoreLocal { slot: u32, span: Span },
    LoadGlobal { slot: u32, span: Span },
    StoreGlobal { slot: u32, span: Span },

    // Calls / returns
    Call { callee: String, argc: usize, span: Span },
    /// Create a closure value from a function and captured locals.
    MakeClosure {
        callee: String,
        capture_locals: Vec<u32>,
        span: Span,
    },
    /// Call a closure/function value currently on stack.
    /// Stack before: [..., callee_value, arg1, arg2, ...]
    CallClosure { argc: usize, span: Span },
    Return { span: Span },

    /// Pop `argc` arguments (Callee arg order last-popped = first param), then push `Task::Deferred`.
    MakeDeferredTask {
        func: String,
        argc: usize,
        span: Span,
    },
    /// Pop a `Task` value; if deferred, run target function to completion (recursive await); push payload.
    AwaitTask { span: Span },
}

#[derive(Debug, Clone)]
pub struct FunctionBytecode {
    pub name: String,
    pub code: Vec<Instr>,
    pub local_count: u32,
    pub param_count: usize,
    /// Slot for each parameter (in order). `None` means wildcard parameter (`_`) which is not stored.
    pub param_slots: Vec<Option<u32>>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct ProgramBytecode {
    pub entry_main: String,
    pub init_code: Vec<Instr>,
    pub functions: Vec<FunctionBytecode>,
    pub function_map: std::collections::HashMap<String, usize>,
    /// Globals are laid out as an indexed array (slot -> Value).
    pub globals_count: u32,
}

