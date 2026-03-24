//! Stack VM for executing Vibelang bytecode.

use crate::ast::{BinaryOp, UnaryOp};
use crate::bytecode::{Instr, ProgramBytecode};
use crate::error::Span;
use crate::value::{StructInstance, TaskInner, Value};
use num_bigint::{BigInt, Sign};
use num_traits::{Signed, ToPrimitive, Zero};
use std::cell::RefCell;
use std::rc::Rc;

#[derive(Debug)]
pub struct VmError {
    pub message: String,
    pub span: Option<Span>,
}

impl VmError {
    fn new(message: impl Into<String>, span: Option<Span>) -> Self {
        Self {
            message: message.into(),
            span,
        }
    }

    pub fn format_with_file(&self, path: &str) -> String {
        match self.span {
            Some(s) => format!("{}:{}:{}: vm error: {}", path, s.line, s.column, self.message),
            None => format!("{}: vm error: {}", path, self.message),
        }
    }
}

impl std::fmt::Display for VmError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.span {
            Some(s) => write!(f, "{}:{}: {}", s.line, s.column, self.message),
            None => write!(f, "{}", self.message),
        }
    }
}

impl std::error::Error for VmError {}

#[derive(Debug, Clone, Copy)]
enum FrameKind {
    Init,
    User { func_index: usize },
}

#[derive(Debug, Clone)]
struct Frame {
    kind: FrameKind,
    ip: usize,
    locals: Vec<Option<Value>>,
    stack_base: usize,
}

pub fn run_program(bytecode: &ProgramBytecode) -> Result<(), VmError> {
    run_program_with_builtins(bytecode, crate::builtins::default_registry_ref())
}

pub fn run_program_with_builtins(
    bytecode: &ProgramBytecode,
    builtins: &crate::builtins::BuiltinRegistry,
) -> Result<(), VmError> {
    let mut globals: Vec<Option<Value>> = vec![None; bytecode.globals_count as usize];
    let mut operand_stack: Vec<Value> = Vec::new();

    let mut frames: Vec<Frame> = Vec::new();
    frames.push(Frame {
        kind: FrameKind::Init,
        ip: 0,
        locals: Vec::new(),
        stack_base: 0,
    });

    let main_idx = bytecode
        .function_map
        .get(&bytecode.entry_main)
        .copied()
        .ok_or_else(|| VmError::new("missing `main` function in bytecode", None))?;

    loop {
        let Some(frame) = frames.last_mut() else {
            break;
        };

        let code_len = match frame.kind {
            FrameKind::Init => bytecode.init_code.len(),
            FrameKind::User { func_index } => bytecode.functions[func_index].code.len(),
        };

        if frame.ip >= code_len {
            // Fell off the end of a frame.
            let ended = frames.pop().expect("just checked");
            match ended.kind {
                FrameKind::Init => {
                    // Run main next.
                    frames.push(Frame {
                        kind: FrameKind::User { func_index: main_idx },
                        ip: 0,
                        locals: vec![None; bytecode.functions[main_idx].local_count as usize],
                        stack_base: operand_stack.len(),
                    });
                }
                FrameKind::User { .. } => {
                    // Implicit `return;` -> `()`.
                    let ret = Value::Unit;
                    if !frames.is_empty() {
                        operand_stack.truncate(ended.stack_base);
                        operand_stack.push(ret);
                    } else {
                        operand_stack.truncate(ended.stack_base);
                    }
                }
            }
            continue;
        }

        // Fetch instruction (clone to avoid borrow conflicts).
        let instr = match frame.kind {
            FrameKind::Init => bytecode.init_code[frame.ip].clone(),
            FrameKind::User { func_index } => bytecode.functions[func_index].code[frame.ip].clone(),
        };
        frame.ip += 1;

        match instr {
            Instr::PushInt { value, radix, span } => {
                let digits = if radix == 10 {
                    value.replace('_', "")
                } else if value.len() >= 2
                    && (value.starts_with("0x")
                        || value.starts_with("0X")
                        || value.starts_with("0o")
                        || value.starts_with("0O")
                        || value.starts_with("0b")
                        || value.starts_with("0B"))
                {
                    value[2..].replace('_', "")
                } else {
                    value.replace('_', "")
                };
                let i = BigInt::parse_bytes(digits.as_bytes(), radix)
                    .ok_or_else(|| VmError::new("invalid integer literal", Some(span)))?;
                operand_stack.push(Value::Int(i));
            }
            Instr::PushString { value } => operand_stack.push(Value::String(value)),
            Instr::PushBool { value } => operand_stack.push(Value::Bool(value)),
            Instr::PushUnit => operand_stack.push(Value::Unit),

            Instr::MakeTuple { count } => {
                let mut parts = Vec::with_capacity(count);
                for _ in 0..count {
                    parts.push(
                        operand_stack
                            .pop()
                            .ok_or_else(|| VmError::new("stack underflow", None))?,
                    );
                }
                parts.reverse();
                operand_stack.push(Value::Tuple(parts));
            }
            Instr::GetTupleField { index, span } => {
                let v = operand_stack.pop().ok_or_else(|| {
                    VmError::new("stack underflow on tuple field access", Some(span))
                })?;
                let Value::Tuple(parts) = v else {
                    return Err(VmError::new(
                        "tuple field access on non-tuple",
                        Some(span),
                    ));
                };
                let i = index as usize;
                let out = parts
                    .get(i)
                    .cloned()
                    .ok_or_else(|| VmError::new("tuple index out of range", Some(span)))?;
                operand_stack.push(out);
            }
            Instr::GetTupleFieldFromEnd {
                offset_from_end,
                span,
            } => {
                let v = operand_stack.pop().ok_or_else(|| {
                    VmError::new("stack underflow on tuple field access", Some(span))
                })?;
                let Value::Tuple(parts) = v else {
                    return Err(VmError::new(
                        "tuple field access on non-tuple",
                        Some(span),
                    ));
                };
                let offset = offset_from_end as usize;
                if offset >= parts.len() {
                    return Err(VmError::new(
                        "tuple index out of range",
                        Some(span),
                    ));
                }
                let i = parts.len() - 1 - offset;
                operand_stack.push(parts[i].clone());
            }

            Instr::MakeArray { count } => {
                let mut parts = Vec::with_capacity(count);
                for _ in 0..count {
                    parts.push(
                        operand_stack
                            .pop()
                            .ok_or_else(|| VmError::new("stack underflow", None))?,
                    );
                }
                parts.reverse();
                operand_stack.push(Value::Array(parts));
            }
            Instr::GetArrayIndex { span } => {
                let value_idx = operand_stack
                    .pop()
                    .ok_or_else(|| VmError::new("stack underflow on array index", Some(span)))?;
                let array_val = operand_stack
                    .pop()
                    .ok_or_else(|| VmError::new("stack underflow on array index", Some(span)))?;
                let idx = as_int(value_idx, Some(span))?;

                let Value::Array(parts) = array_val else {
                    return Err(VmError::new("array indexing on non-array", Some(span)));
                };
                let len = BigInt::from(parts.len());
                let mapped = if idx.sign() == Sign::Minus { &len + &idx } else { idx.clone() };
                if mapped.sign() == Sign::Minus || mapped >= len {
                    return Err(VmError::new(
                        format!(
                            "array index out of range: index {} for length {}",
                            idx,
                            parts.len()
                        ),
                        Some(span),
                    ));
                }
                let mapped_usize = mapped
                    .to_usize()
                    .ok_or_else(|| VmError::new("array index out of range", Some(span)))?;
                operand_stack.push(parts[mapped_usize].clone());
            }
            Instr::GetArrayIndexFromEnd {
                offset_from_end,
                span,
            } => {
                let array_val = operand_stack.pop().ok_or_else(|| {
                    VmError::new("stack underflow on array index-from-end", Some(span))
                })?;
                let Value::Array(parts) = array_val else {
                    return Err(VmError::new(
                        "array indexing-from-end on non-array",
                        Some(span),
                    ));
                };
                let offset = offset_from_end as usize;
                if offset >= parts.len() {
                    return Err(VmError::new(
                        "array index out of range",
                        Some(span),
                    ));
                }
                let i = parts.len() - 1 - offset;
                operand_stack.push(parts[i].clone());
            }
            Instr::ArrayStore { span } => {
                let value = operand_stack
                    .pop()
                    .ok_or_else(|| VmError::new("stack underflow on array store", Some(span)))?;
                let value_idx = operand_stack
                    .pop()
                    .ok_or_else(|| VmError::new("stack underflow on array store", Some(span)))?;
                let array_val = operand_stack
                    .pop()
                    .ok_or_else(|| VmError::new("stack underflow on array store", Some(span)))?;

                let idx = as_int(value_idx, Some(span))?;
                let Value::Array(mut parts) = array_val else {
                    return Err(VmError::new("array store on non-array", Some(span)));
                };

                let len = BigInt::from(parts.len());
                let mapped = if idx.sign() == Sign::Minus { &len + &idx } else { idx.clone() };
                if mapped.sign() == Sign::Minus || mapped >= len {
                    return Err(VmError::new(
                        format!(
                            "array index out of range: index {} for length {}",
                            idx,
                            parts.len()
                        ),
                        Some(span),
                    ));
                }

                let mapped_usize = mapped
                    .to_usize()
                    .ok_or_else(|| VmError::new("array index out of range", Some(span)))?;
                parts[mapped_usize] = value;
                operand_stack.push(Value::Array(parts));
            }
            Instr::AssertArrayLenEq { expected, span } => {
                let array_val = operand_stack.pop().ok_or_else(|| {
                    VmError::new("stack underflow on AssertArrayLenEq", Some(span))
                })?;
                let Value::Array(parts) = array_val else {
                    return Err(VmError::new("AssertArrayLenEq on non-array", Some(span)));
                };
                if parts.len() != expected {
                    return Err(VmError::new(
                        format!(
                            "array pattern length mismatch: expected {}, found {}",
                            expected,
                            parts.len()
                        ),
                        Some(span),
                    ));
                }
            }

            Instr::MakeStructLiteral {
                name,
                is_unit_literal,
                field_names,
                has_update,
                span,
            } => {
                let field_count = field_names.len();
                let mut field_values: Vec<Value> = Vec::with_capacity(field_count);
                for _ in 0..field_count {
                    field_values.push(
                        operand_stack
                            .pop()
                            .ok_or_else(|| VmError::new("stack underflow", Some(span)))?,
                    );
                }
                field_values.reverse();

                let mut map: std::collections::HashMap<String, Value> =
                    std::collections::HashMap::new();
                if has_update {
                    let base = operand_stack.pop().ok_or_else(|| {
                        VmError::new("stack underflow on struct update base", Some(span))
                    })?;
                    let Value::Struct(rc) = base else {
                        return Err(VmError::new(
                            "struct update base requires a struct value",
                            Some(span),
                        ));
                    };
                    map = rc.borrow().fields.clone();
                }

                for (fname, v) in field_names.into_iter().zip(field_values.into_iter()) {
                    map.insert(fname, v);
                }

                operand_stack.push(Value::Struct(Rc::new(RefCell::new(StructInstance {
                    name,
                    is_unit: is_unit_literal,
                    fields: map,
                }))));
            }

            Instr::GetStructField { field, span } => {
                let v = operand_stack.pop().ok_or_else(|| {
                    VmError::new("stack underflow on GetStructField", Some(span))
                })?;
                let Value::Struct(rc) = v else {
                    return Err(VmError::new(
                        "GetStructField on non-struct value",
                        Some(span),
                    ));
                };
                let inst = rc.borrow();
                let out = inst.fields.get(&field).cloned().ok_or_else(|| {
                    VmError::new(format!("unknown field `{field}` in struct"), Some(span))
                })?;
                operand_stack.push(out);
            }

            Instr::StructFieldStore { field, span } => {
                let value = operand_stack.pop().ok_or_else(|| {
                    VmError::new("stack underflow on StructFieldStore(value)", Some(span))
                })?;
                let v = operand_stack.pop().ok_or_else(|| {
                    VmError::new("stack underflow on StructFieldStore(struct)", Some(span))
                })?;
                let Value::Struct(rc) = v else {
                    return Err(VmError::new(
                        "StructFieldStore on non-struct value",
                        Some(span),
                    ));
                };
                rc.borrow_mut().fields.insert(field, value);
            }
            Instr::MatchStructName { name, span } => {
                let v = operand_stack.pop().ok_or_else(|| {
                    VmError::new("stack underflow on MatchStructName(value)", Some(span))
                })?;
                if let Value::Struct(rc) = v {
                    let got = rc.borrow();
                    operand_stack.push(Value::Bool(got.name == name));
                } else {
                    operand_stack.push(Value::Bool(false));
                }
            }
            Instr::MakeEnumVariant {
                enum_name,
                variant,
                payload_count,
                span,
            } => {
                let mut payloads = Vec::with_capacity(payload_count);
                for _ in 0..payload_count {
                    payloads.push(operand_stack.pop().ok_or_else(|| {
                        VmError::new(
                            "stack underflow on MakeEnumVariant(payload)",
                            Some(span),
                        )
                    })?);
                }
                payloads.reverse();
                operand_stack.push(Value::Enum {
                    enum_name,
                    variant,
                    payloads,
                });
            }
            Instr::MatchEnumVariant {
                enum_name,
                variant,
                payload_count,
                span,
            } => {
                let v = operand_stack.pop().ok_or_else(|| {
                    VmError::new("stack underflow on MatchEnumVariant(value)", Some(span))
                })?;
                if let Value::Enum {
                    enum_name: got_enum,
                    variant: got_variant,
                    payloads: got_payloads,
                } = v
                {
                    let ok = got_enum == enum_name
                        && got_variant == variant
                        && got_payloads.len() == payload_count;
                    if ok {
                        // Push payloads in order, then the match result bool.
                        for p in got_payloads {
                            operand_stack.push(p);
                        }
                        operand_stack.push(Value::Bool(true));
                    } else {
                        operand_stack.push(Value::Bool(false));
                    }
                } else {
                    operand_stack.push(Value::Bool(false));
                }
            }
            Instr::UnpackEnumVariant {
                enum_name,
                variant,
                payload_count,
                span,
            } => {
                let v = operand_stack.pop().ok_or_else(|| {
                    VmError::new(
                        "stack underflow on UnpackEnumVariant(value)",
                        Some(span),
                    )
                })?;

                let Value::Enum {
                    enum_name: got_enum,
                    variant: got_variant,
                    payloads,
                } = v
                else {
                    return Err(VmError::new(
                        "UnpackEnumVariant on non-enum value",
                        Some(span),
                    ));
                };

                if got_enum != enum_name || got_variant != variant || payloads.len() != payload_count
                {
                    return Err(VmError::new(
                        format!(
                            "enum variant mismatch: expected `{enum_name}::{variant}` with {} payload(s)",
                            payload_count
                        ),
                        Some(span),
                    ));
                }

                for p in payloads {
                    operand_stack.push(p);
                }
            }

            Instr::BinOp { op, span } => {
                let r = operand_stack
                    .pop()
                    .ok_or_else(|| VmError::new("stack underflow on binary op", Some(span)))?;
                let l = operand_stack
                    .pop()
                    .ok_or_else(|| VmError::new("stack underflow on binary op", Some(span)))?;
                operand_stack.push(eval_binop(l, op, r, span)?);
            }
            Instr::UnOp { op, span } => {
                let v = operand_stack
                    .pop()
                    .ok_or_else(|| VmError::new("stack underflow on unary op", Some(span)))?;
                operand_stack.push(eval_unop(v, op, span)?);
            }
            Instr::Pop => {
                let _ = operand_stack.pop();
            }
            Instr::AssertBool { span } => {
                let v = operand_stack
                    .pop()
                    .ok_or_else(|| VmError::new("stack underflow on AssertBool", None))?;
                let b = as_bool(v, Some(span))?;
                if !b {
                    return Err(VmError::new("pattern literal mismatch", Some(span)));
                }
            }

            Instr::Jump { target } => {
                frames.last_mut().unwrap().ip = target;
            }
            Instr::JumpIfFalse { target } => {
                let v = operand_stack
                    .pop()
                    .ok_or_else(|| VmError::new("stack underflow on JumpIfFalse", None))?;
                let b = as_bool(v, None)?;
                if !b {
                    frames.last_mut().unwrap().ip = target;
                }
            }
            Instr::JumpIfTrue { target } => {
                let v = operand_stack
                    .pop()
                    .ok_or_else(|| VmError::new("stack underflow on JumpIfTrue", None))?;
                let b = as_bool(v, None)?;
                if b {
                    frames.last_mut().unwrap().ip = target;
                }
            }

            Instr::LoadLocal { slot, span } => {
                let frame = frames.last().unwrap();
                let got = frame
                    .locals
                    .get(slot as usize)
                    .ok_or_else(|| VmError::new("invalid local slot", Some(span)))?;
                let Some(v) = got.clone() else {
                    return Err(VmError::new("uninitialized local", Some(span)));
                };
                operand_stack.push(v);
            }
            Instr::StoreLocal { slot, span } => {
                let v = operand_stack
                    .pop()
                    .ok_or_else(|| VmError::new("stack underflow on StoreLocal", Some(span)))?;
                let frame = frames.last_mut().unwrap();
                let slot_ref = frame.locals.get_mut(slot as usize).ok_or_else(|| {
                    VmError::new("invalid local slot", Some(span))
                })?;
                *slot_ref = Some(v);
            }
            Instr::LoadGlobal { slot, span } => {
                let got = globals.get(slot as usize).ok_or_else(|| {
                    VmError::new("invalid global slot", Some(span))
                })?;
                let Some(v) = got.clone() else {
                    return Err(VmError::new("uninitialized global", Some(span)));
                };
                operand_stack.push(v);
            }
            Instr::StoreGlobal { slot, span } => {
                let v = operand_stack
                    .pop()
                    .ok_or_else(|| VmError::new("stack underflow on StoreGlobal", Some(span)))?;
                let slot_ref = globals.get_mut(slot as usize).ok_or_else(|| {
                    VmError::new("invalid global slot", Some(span))
                })?;
                *slot_ref = Some(v);
            }

            Instr::Call { callee, argc, span } => {
                let mut args = Vec::with_capacity(argc);
                for _ in 0..argc {
                    args.push(
                        operand_stack
                            .pop()
                            .ok_or_else(|| VmError::new("stack underflow on call", Some(span)))?,
                    );
                }
                args.reverse();

                // Builtins (shared dispatch)
                if builtins.get(&callee).is_some() {
                    let v = builtins
                        .eval_builtin(&callee, args, span)
                        .map_err(|e| VmError::new(e.message, e.span))?;
                    operand_stack.push(v);
                    continue;
                }

                // User function call
                let func_index = bytecode
                    .function_map
                    .get(&callee)
                    .copied()
                    .ok_or_else(|| VmError::new(format!("unknown function `{callee}`"), Some(span)))?;

                let func = &bytecode.functions[func_index];
                let mut locals = vec![None; func.local_count as usize];
                if args.len() != func.param_count {
                    return Err(VmError::new(
                        format!(
                            "wrong arity: expected {} args, got {}",
                            func.param_count,
                            args.len()
                        ),
                        Some(span),
                    ));
                }
                for (i, slot_opt) in func.param_slots.iter().enumerate() {
                    if let Some(slot) = slot_opt {
                        locals[*slot as usize] = Some(args[i].clone());
                    }
                }

                let stack_base = operand_stack.len();
                frames.push(Frame {
                    kind: FrameKind::User { func_index },
                    ip: 0,
                    locals,
                    stack_base,
                });
            }
            Instr::MakeClosure {
                callee,
                capture_locals,
                span,
            } => {
                let frame = frames
                    .last()
                    .ok_or_else(|| VmError::new("missing frame for closure", Some(span)))?;
                let mut captures = Vec::with_capacity(capture_locals.len());
                for slot in capture_locals {
                    let v = frame
                        .locals
                        .get(slot as usize)
                        .ok_or_else(|| VmError::new("invalid captured local slot", Some(span)))?
                        .clone()
                        .ok_or_else(|| VmError::new("captured local is uninitialized", Some(span)))?;
                    captures.push(v);
                }
                operand_stack.push(Value::Closure { callee, captures });
            }
            Instr::CallClosure { argc, span } => {
                let mut args = Vec::with_capacity(argc);
                for _ in 0..argc {
                    args.push(
                        operand_stack.pop().ok_or_else(|| {
                            VmError::new("stack underflow on closure call args", Some(span))
                        })?,
                    );
                }
                args.reverse();
                let callee_v = operand_stack
                    .pop()
                    .ok_or_else(|| VmError::new("stack underflow on closure callee", Some(span)))?;
                let (callee, mut captures) = match callee_v {
                    Value::Closure { callee, captures } => (callee, captures),
                    _ => {
                        return Err(VmError::new(
                            "attempted to call non-closure value",
                            Some(span),
                        ))
                    }
                };
                captures.extend(args);
                let func_index = bytecode
                    .function_map
                    .get(&callee)
                    .copied()
                    .ok_or_else(|| VmError::new(format!("unknown function `{callee}`"), Some(span)))?;
                let func = &bytecode.functions[func_index];
                if captures.len() != func.param_count {
                    return Err(VmError::new(
                        format!(
                            "wrong arity: expected {} args, got {}",
                            func.param_count,
                            captures.len()
                        ),
                        Some(span),
                    ));
                }
                let mut locals = vec![None; func.local_count as usize];
                for (i, slot_opt) in func.param_slots.iter().enumerate() {
                    if let Some(slot) = slot_opt {
                        locals[*slot as usize] = Some(captures[i].clone());
                    }
                }
                let stack_base = operand_stack.len();
                frames.push(Frame {
                    kind: FrameKind::User { func_index },
                    ip: 0,
                    locals,
                    stack_base,
                });
            }

            Instr::MakeDeferredTask { func, argc, span } => {
                let mut args = Vec::with_capacity(argc);
                for _ in 0..argc {
                    args.push(
                        operand_stack
                            .pop()
                            .ok_or_else(|| VmError::new("stack underflow on MakeDeferredTask", Some(span)))?,
                    );
                }
                args.reverse();
                operand_stack.push(Value::Task(TaskInner::Deferred { func, args }));
            }
            Instr::AwaitTask { span } => {
                let v = operand_stack
                    .pop()
                    .ok_or_else(|| VmError::new("stack underflow on await", Some(span)))?;
                match v {
                    Value::Task(TaskInner::Completed(payload)) => {
                        operand_stack.push(*payload);
                    }
                    Value::Task(TaskInner::Deferred {
                        func: callee,
                        args,
                    }) => {
                        if builtins.get(&callee).is_some() {
                            let mut cur = builtins
                                .eval_builtin(&callee, args, span)
                                .map_err(|e| VmError::new(e.message, e.span))?;
                            loop {
                                match cur {
                                    Value::Task(TaskInner::Completed(p)) => {
                                        operand_stack.push(*p);
                                        break;
                                    }
                                    Value::Task(TaskInner::Deferred {
                                        func: c2,
                                        args: a2,
                                    }) => {
                                        if builtins.get(&c2).is_some() {
                                            cur = builtins
                                                .eval_builtin(&c2, a2, span)
                                                .map_err(|e| VmError::new(e.message, e.span))?;
                                            continue;
                                        }
                                        let func_index = bytecode
                                            .function_map
                                            .get(&c2)
                                            .copied()
                                            .ok_or_else(|| {
                                                VmError::new(
                                                    format!("unknown function `{c2}` in await"),
                                                    Some(span),
                                                )
                                            })?;
                                        let func = &bytecode.functions[func_index];
                                        let mut locals = vec![None; func.local_count as usize];
                                        if a2.len() != func.param_count {
                                            return Err(VmError::new(
                                                format!(
                                                    "wrong arity in deferred await: expected {}, got {}",
                                                    func.param_count,
                                                    a2.len()
                                                ),
                                                Some(span),
                                            ));
                                        }
                                        for (i, slot_opt) in func.param_slots.iter().enumerate() {
                                            if let Some(slot) = slot_opt {
                                                locals[*slot as usize] = Some(a2[i].clone());
                                            }
                                        }
                                        let stack_base = operand_stack.len();
                                        frames.push(Frame {
                                            kind: FrameKind::User { func_index },
                                            ip: 0,
                                            locals,
                                            stack_base,
                                        });
                                        break;
                                    }
                                    other => {
                                        operand_stack.push(other);
                                        break;
                                    }
                                }
                            }
                        } else {
                            let func_index = bytecode
                                .function_map
                                .get(&callee)
                                .copied()
                                .ok_or_else(|| {
                                    VmError::new(
                                        format!("unknown function `{callee}` in await"),
                                        Some(span),
                                    )
                                })?;
                            let func = &bytecode.functions[func_index];
                            let mut locals = vec![None; func.local_count as usize];
                            if args.len() != func.param_count {
                                return Err(VmError::new(
                                    format!(
                                        "wrong arity in deferred await: expected {}, got {}",
                                        func.param_count,
                                        args.len()
                                    ),
                                    Some(span),
                                ));
                            }
                            for (i, slot_opt) in func.param_slots.iter().enumerate() {
                                if let Some(slot) = slot_opt {
                                    locals[*slot as usize] = Some(args[i].clone());
                                }
                            }
                            let stack_base = operand_stack.len();
                            frames.push(Frame {
                                kind: FrameKind::User { func_index },
                                ip: 0,
                                locals,
                                stack_base,
                            });
                        }
                    }
                    _ => {
                        return Err(VmError::new(
                            "`await` expects a `Task` value",
                            Some(span),
                        ));
                    }
                }
            }

            Instr::Return { .. } => {
                let ret = operand_stack
                    .pop()
                    .ok_or_else(|| VmError::new("stack underflow on return", None))?;

                let ended = frames.pop().expect("must have a frame");
                let base = ended.stack_base;
                operand_stack.truncate(base);

                if frames.is_empty() {
                    break;
                }
                operand_stack.push(ret);
            }
        }
    }

    Ok(())
}

fn as_bool(v: Value, span: Option<Span>) -> Result<bool, VmError> {
    match v {
        Value::Bool(b) => Ok(b),
        _ => Err(VmError::new("expected Bool", span)),
    }
}

fn as_int(v: Value, span: Option<Span>) -> Result<BigInt, VmError> {
    match v {
        Value::Int(i) => Ok(i),
        _ => Err(VmError::new("expected Int", span)),
    }
}

fn eval_unop(v: Value, op: UnaryOp, span: Span) -> Result<Value, VmError> {
    match op {
        UnaryOp::Not => Ok(Value::Bool(!as_bool(v, Some(span))?)),
        UnaryOp::Plus => Ok(Value::Int(as_int(v, Some(span))?)),
        UnaryOp::Minus => Ok(Value::Int(-as_int(v, Some(span))?)),
        UnaryOp::BitNot => Ok(Value::Int(!as_int(v, Some(span))?)),
    }
}

fn eval_binop(l: Value, op: BinaryOp, r: Value, span: Span) -> Result<Value, VmError> {
    match op {
        BinaryOp::Add => match (l, r) {
            (Value::Int(a), Value::Int(b)) => Ok(Value::Int(a + b)),
            (Value::String(a), Value::String(b)) => Ok(Value::String(format!("{a}{b}"))),
            _ => Err(VmError::new("`+` expects Int+Int or String+String", Some(span))),
        },
        BinaryOp::Sub => Ok(Value::Int(as_int(l, Some(span))? - as_int(r, Some(span))?)),
        BinaryOp::Mul => Ok(Value::Int(as_int(l, Some(span))? * as_int(r, Some(span))?)),
        BinaryOp::Div => {
            let a = as_int(l, Some(span))?;
            let b = as_int(r, Some(span))?;
            if b.is_zero() {
                return Err(VmError::new("division by zero", Some(span)));
            }
            Ok(Value::Int(a / b))
        }
        BinaryOp::Mod => {
            let a = as_int(l, Some(span))?;
            let b = as_int(r, Some(span))?;
            if b.is_zero() {
                return Err(VmError::new("modulo by zero", Some(span)));
            }
            // Divisor-sign remainder semantics (Python-style).
            let mut rem = a % &b;
            if !rem.is_zero() && rem.signum() != b.signum() {
                rem += b;
            }
            Ok(Value::Int(rem))
        }
        BinaryOp::BitAnd => Ok(Value::Int(as_int(l, Some(span))? & as_int(r, Some(span))?)),
        BinaryOp::BitXor => Ok(Value::Int(as_int(l, Some(span))? ^ as_int(r, Some(span))?)),
        BinaryOp::BitOr => Ok(Value::Int(as_int(l, Some(span))? | as_int(r, Some(span))?)),
        BinaryOp::ShiftLeft => {
            let a = as_int(l, Some(span))?;
            let b = as_int(r, Some(span))?;
            if b.sign() == Sign::Minus {
                return Err(VmError::new("shift amount must be non-negative", Some(span)));
            }
            let ub = b
                .to_u32()
                .ok_or_else(|| VmError::new("shift amount out of range", Some(span)))?;
            if ub >= 64 {
                return Err(VmError::new("shift amount out of range", Some(span)));
            }
            Ok(Value::Int(a << ub))
        }
        BinaryOp::ShiftRight => {
            let a = as_int(l, Some(span))?;
            let b = as_int(r, Some(span))?;
            if b.sign() == Sign::Minus {
                return Err(VmError::new("shift amount must be non-negative", Some(span)));
            }
            let ub = b
                .to_u32()
                .ok_or_else(|| VmError::new("shift amount out of range", Some(span)))?;
            if ub >= 64 {
                return Err(VmError::new("shift amount out of range", Some(span)));
            }
            Ok(Value::Int(a >> ub))
        }
        BinaryOp::Eq | BinaryOp::Ne => Ok(Value::Bool(if op == BinaryOp::Eq {
            l == r
        } else {
            l != r
        })),
        BinaryOp::Lt | BinaryOp::Gt | BinaryOp::Le | BinaryOp::Ge => {
            let a = as_int(l, Some(span))?;
            let b = as_int(r, Some(span))?;
            Ok(Value::Bool(match op {
                BinaryOp::Lt => a < b,
                BinaryOp::Gt => a > b,
                BinaryOp::Le => a <= b,
                BinaryOp::Ge => a >= b,
                _ => unreachable!(),
            }))
        }
        BinaryOp::And | BinaryOp::Or => Err(VmError::new(
            "internal error: short-circuit handled by codegen",
            Some(span),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bytecode_gen::compile_program;
    use crate::lexer::Lexer;
    use crate::parser::Parser;
    use crate::semantic::check_program;

    fn run_both(src: &str) -> Result<(), String> {
        let mut lexer = Lexer::new(src);
        let tokens = lexer.tokenize().map_err(|e| e.to_string())?;
        let mut parser = Parser::new(tokens);
        let ast = parser.parse().map_err(|e| e.to_string())?;
        let errs = check_program(&ast);
        assert!(errs.is_empty(), "{:?}", errs);

        // VM
        let bytecode = compile_program(&ast).map_err(|e| format!("{e:?}"))?;
        run_program(&bytecode).map_err(|e| e.to_string())
    }

    fn run_vm_err_contains(src: &str, needle: &str) {
        let mut lexer = Lexer::new(src);
        let tokens = lexer.tokenize().unwrap();
        let mut parser = Parser::new(tokens);
        let ast = parser.parse().unwrap();
        let errs = check_program(&ast);
        assert!(errs.is_empty(), "{:?}", errs);

        let bytecode = compile_program(&ast).unwrap();
        let err = run_program(&bytecode).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains(needle),
            "expected `{needle}` in `{msg}`"
        );
    }

    #[test]
    fn vm_async_sleep_zero_ms() {
        run_both(
            r#"internal struct Task<T = ()>;
               internal async func sleep(ms: Int): Task;
               async func main(): Task {
                   await sleep(0);
               }"#,
        )
        .unwrap();
    }

    #[test]
    fn vm_async_user_deferred_and_await() {
        run_both(
            r#"internal struct Task<T = ()>;
               async func wrap(x: Int): Task<Int> {
                   return x;
               }
               async func main(): Task {
                   let t = wrap(7);
                   let y = await t;
                   let _: Int = y;
               }"#,
        )
        .unwrap();
    }

    #[test]
    fn vm_arithmetic_main() {
        run_both(r#"func main() { let _: Int = 1 + 2 * 3; }"#).unwrap();
    }

    #[test]
    fn vm_let_uninit_then_assign() {
        run_both(
            r#"func main() {
                let a: Int;
                a = 5;
                let _: Int = a;
            }"#,
        )
        .unwrap();
    }

    #[test]
    fn vm_if_else_and_while_break_continue() {
        run_both(
            r#"func main() {
                let a: Int = 0;
                let i: Int = 0;
                while i < 10 {
                    if i == 3 { break; }
                    i += 1;
                    a += 1;
                    if a == 2 { continue; }
                }
            }"#,
        )
        .unwrap();
    }

    #[test]
    fn vm_while_break() {
        run_both(
            r#"func main() {
                while true { break; }
            }"#,
        )
        .unwrap();
    }

    #[test]
    fn vm_internal_input_decl_ok() {
        run_both(
            r#"internal func input(): String;
               internal func print(s: String);
               func main() { print("ok"); }"#,
        )
        .unwrap();
    }

    #[test]
    fn vm_internal_stoi_decl_ok_and_works() {
        run_both(
            r#"internal func stoi(s: String): Int;
               func main() {
                   let x: Int = stoi("7");
                   if x == 7 { let _: Int = 1; } else { let _: Int = 1 / 0; }
               }"#,
        )
        .unwrap();
    }

    #[test]
    fn vm_tuple_destructure_and_field_access() {
        run_both(
            r#"internal func print(s: String);
               internal func itos(n: Int): String;
               func main() {
                   let t = (10, 20);
                   let (a, b) = t;
                   print(itos(a + b));
                   print(itos(t.0));
               }"#,
        )
        .unwrap();
    }

    #[test]
    fn vm_struct_example12() {
        run_both(include_str!("../examples/example12.vc")).unwrap();
    }

    #[test]
    fn vm_arrow_example13() {
        run_both(include_str!("../examples/example13.vc")).unwrap();
    }

    #[test]
    fn vm_lambda_closure_capture_and_invoke() {
        let src = r#"
internal func print_gen<T>(t: T);
func make_adder(x: Int): (Int) => Int = (y) => x + y;
func main() {
    let add5 = make_adder(5);
    print_gen(add5(3));
}
"#;
        run_both(src).unwrap();
    }

    #[test]
    fn vm_enum_example14() {
        run_both(include_str!("../examples/example14.vc")).unwrap();
    }

    #[test]
    fn vm_enum_example15() {
        run_both(include_str!("../examples/example15.vc")).unwrap();
    }

    #[test]
    fn vm_example16_match_basic() {
        run_both(
            r#"internal func print_gen<T>(t: T);

               enum Option<T> {
                   None,
                   Some(T),
               }

               struct Point {
                   x: Int,
                   y: Int,
               }

               func get_point() = Option::Some(Point { x: 10, y: 20 });

               func main() {
                   match get_point() {
                       Option::Some(Point { x: 1, y: 2 }) => print_gen("1, 2"),
                       Option::Some(Point { x, y }) => print_gen("10, 20"),
                       Option::None => print_gen("None"),
                   }
               }"#,
        )
        .unwrap();
    }

    #[test]
    fn vm_example17_defaults_and_named_args() {
        run_both(include_str!("../examples/example17.vc")).unwrap();
    }

    #[test]
    fn vm_default_value_not_evaluated_when_argument_is_provided() {
        run_both(
            r#"func foo(a: Int = 1 / 0) {}
               func main() { foo(10); }"#,
        )
        .unwrap();
    }

    #[test]
    fn vm_default_value_is_evaluated_when_argument_is_omitted() {
        run_vm_err_contains(
            r#"func foo(a: Int = 1 / 0) {}
               func main() { foo(); }"#,
            "division by zero",
        );
    }

    #[test]
    fn vm_params_autopack_and_empty() {
        run_both(
            r#"internal func int_array_len(a: [Int]): Int;
               internal func print_int(v: Int);
               func sum(params numbers: [Int]): Int {
                   let i = 0;
                   let out = 0;
                   while int_array_len(numbers) > i {
                       out += numbers[i];
                       i += 1;
                   }
                   return out;
               }
               func main() {
                   print_int(sum());
                   print_int(sum(1, 2, 3, 4));
               }"#,
        )
        .unwrap();
    }

    #[test]
    fn vm_example19_params_sum_variant() {
        run_both(
            r#"internal func int_array_len(a: [Int]): Int;
               internal func print_gen<T>(t: T);
               internal func itos(s: Int): String;
               func sum(params numbers: [Int]): Int {
                   let idx = 0;
                   let result = 0;
                   while int_array_len(numbers) > idx {
                       result += numbers[idx];
                       idx += 1;
                   }
                   return result;
               }
               func main() {
                   print_gen(itos(sum(1, 2, 3, 4)));
               }"#,
        )
        .unwrap();
    }

    #[test]
    fn vm_example21_array_extension_methods() {
        run_both(
            r#"internal func print_gen<T>(t: T);
               func [Int]::len(self): Int {
                   return 123;
               }
               func [String]::len(self): Int {
                   return 456;
               }
               func [type T]::len<T>(self): Int {
                   return 789;
               }
               func main() {
                   print_gen([1, 2, 3, 4, 5].len());
                   print_gen(["Hello"].len());
                   print_gen([true].len());
               }"#,
        )
        .unwrap();
    }

    #[test]
    fn vm_example22_struct_extension_methods() {
        run_both(
            r#"internal func print_gen<T>(t: T);
               struct Void {}
               func Void::foo<T>(x: T): T {
                   return x;
               }
               func Void::bar<T>(self, x: T): T {
                   return x;
               }
               func main() {
                   print_gen(Void::foo(123));
                   print_gen(Void{}.bar(456));
               }"#,
        )
        .unwrap();
    }

    #[test]
    fn vm_generic_struct_literals_and_unit_generic() {
        run_both(
            r#"internal func print_gen<T>(t: T);
               struct Generic<T> { a: T, b: T }
               struct UnitGeneric<T>;
               func main() {
                   let g = Generic { a: 7, b: 8 };
                   let g2 = Generic<Int> { a: 9, b: 10 };
                   let u = UnitGeneric<Int>;
                   print_gen(g);
                   print_gen(g2);
                   print_gen(u);
               }"#,
        )
        .unwrap();
    }

    #[test]
    fn vm_unit_generic_struct_patterns_if_let_and_match() {
        run_both(
            r#"internal func print_gen<T>(t: T);
               struct UnitGeneric<T>;
               func main() {
                   let ug = UnitGeneric<Int>;
                   if let UnitGeneric<Int> = ug {
                       print_gen("ok");
                   } else {
                       print_gen("bad");
                   }
                   let out: Int = match ug {
                       UnitGeneric<Int> => 2,
                       _ => 3,
                   };
                   print_gen(out);
               }"#,
        )
        .unwrap();
    }


    #[test]
    fn vm_arrays_example9() {
        run_both(include_str!("../examples/example9.vc")).unwrap();
    }

    #[test]
    fn vm_any_example10() {
        run_both(include_str!("../examples/example10.vc")).unwrap();
    }

    #[test]
    fn vm_generics_example11() {
        run_both(include_str!("../examples/example11.vc")).unwrap();
    }

    #[test]
    fn vm_user_defined_generic_identity() {
        run_both(
            r#"internal func print_int(v: Int);
               func id<T>(x: T): T { return x; }
               func main() {
                   let a: Int = id(777);
                   print_int(a);
               }"#,
        )
        .unwrap();
    }

    #[test]
    fn vm_shift_operators_ok() {
        run_both(
            r#"internal func print_int(v: Int);
               func main() {
                   let a: Int = 1 << 3;
                   let b: Int = 16 >> 2;
                   print_int(a);
                   print_int(b);
                   let c: Int = 1;
                   c <<= 2;
                   print_int(c);
                   let d: Int = 8;
                   d >>= 1;
                   print_int(d);
               }"#,
        )
        .unwrap();
    }

    #[test]
    fn vm_shift_negative_amount_runtime_error() {
        run_vm_err_contains(r#"func main() { let _: Int = 1 << -1; }"#, "shift amount must be non-negative");
    }

    #[test]
    fn vm_shift_out_of_range_runtime_error() {
        run_vm_err_contains(r#"func main() { let _: Int = 1 << 64; }"#, "shift amount out of range");
    }

    #[test]
    fn vm_array_index_out_of_bounds_errors() {
        run_vm_err_contains(
            r#"internal func print_int(v: Int);
               func main() {
                   let a: [Int] = [1, 2, 3];
                   print_int(a[3]);
               }"#,
            "array index out of range",
        )
    }

    #[test]
    fn vm_array_pattern_length_mismatch_errors() {
        run_vm_err_contains(
            r#"internal func print_int(v: Int);
               func main() {
                   let a: [Int] = [1];
                   let [x, y] = a;
                   print_int(x);
               }"#,
            "array pattern length mismatch",
        )
    }

    #[test]
    fn vm_short_circuit() {
        run_both(
            r#"internal func print(s: String);
               func main() {
                   if false && true { print("bad_and"); }
                   if true || false { print("ok_or"); }
               }"#,
        )
        .unwrap();
    }

    #[test]
    fn vm_division_by_zero_runtime_error() {
        run_vm_err_contains(r#"func main() { let _: Int = 1 / 0; }"#, "division by zero");
    }

    #[test]
    fn vm_stoi_invalid_string() {
        run_vm_err_contains(
            r#"internal func stoi(s: String): Int;
               func main() { let _: Int = stoi("nope"); }"#,
            "stoi expects a valid Int-formatted string",
        );
    }

    #[test]
    fn vm_rand_int_bounds() {
        run_both(
            r#"internal func rand_int(to: Int): Int;
               func main() {
                   let r: Int = rand_int(2);
                   if r == 0 { }
                   else if r == 1 { }
                   else { let _: Int = 1 / 0; }
               }"#,
        )
        .unwrap();
    }

    #[test]
    fn vm_rand_int_zero_errors() {
        run_vm_err_contains(
            r#"internal func rand_int(to: Int): Int;
               func main() { let _: Int = rand_int(0); }"#,
            "rand_int expects `to > 0`",
        );
    }

    #[test]
    fn vm_rand_bigint_basic() {
        run_both(
            r#"internal func rand_bigint(bits: Int): Int;
               func main() {
                   let x: Int = rand_bigint(128);
                   let _ = x + 1;
               }"#,
        )
        .unwrap();
    }

    #[test]
    fn vm_rand_bigint_invalid_bits() {
        run_vm_err_contains(
            r#"internal func rand_bigint(bits: Int): Int;
               func main() { let _ = rand_bigint(0); }"#,
            "bits > 0",
        );
    }

    #[test]
    fn vm_bigint_unbounded_literal_arithmetic() {
        run_both(
            r#"func main() {
                   let a: Int = 999999999999999999999999999999999999999999999999999999999;
                   let b: Int = a + 1;
                   let _ = b;
               }"#,
        )
        .unwrap();
    }

    #[test]
    fn vm_modulo_divisor_sign_semantics() {
        run_both(
            r#"func main() {
                   let a: Int = 5 % -3;
                   let b: Int = -5 % -3;
                   let c: Int = -5 % 3;
                   let d: Int = 5 % 3;
                   if a != -1 { let _: Int = 1 / 0; }
                   if b != -2 { let _: Int = 1 / 0; }
                   if c != 1 { let _: Int = 1 / 0; }
                   if d != 2 { let _: Int = 1 / 0; }
               }"#,
        )
        .unwrap();
    }

    macro_rules! vm_match_nested_case {
        ($name:ident, $k:expr) => {
            #[test]
            fn $name() {
                run_both(
                    &format!(
                        r#"internal func print_int(v: Int);
                           enum Option<T> {{ None, Some(T) }}
                           struct Point3 {{ x: Int, y: Int, z: Int, }}
                           struct Inner {{ p: Point3, t: Point3, }}
                           struct Wrap {{ inner: Inner, ok: Bool, marker: Int, }}
                           func mk(i: Int): Option<Wrap> {{
                               if i > 0 {{
                                   return Option::Some(Wrap {{
                                       inner: Inner {{
                                           p: Point3 {{ x: i, y: i + 1, z: i + 2 }},
                                           t: Point3 {{ x: i + 3, y: i + 4, z: i + 5 }}
                                       }},
                                       ok: true,
                                       marker: i
                                   }});
                               }}
                               return Option::None;
                           }}
                           func main() {{
                               let out: Int = match mk(4) {{
                                   Option::Some(Wrap {{
                                       inner: Inner {{
                                           p: Point3 {{ x, y: _, z }},
                                           t: Point3 {{ x: tx, y: _, z: tz }}
                                       }},
                                       ok: _,
                                       marker: _
                                   }}) => x + z + tx + tz + {},
                                   Option::None => 0,
                               }};
                               print_int(out);
                           }}"#,
                        $k
                    ),
                )
                .unwrap();
            }
        };
    }

    vm_match_nested_case!(vm_match_edge_01, 1);
    vm_match_nested_case!(vm_match_edge_02, 2);
    vm_match_nested_case!(vm_match_edge_03, 3);
    vm_match_nested_case!(vm_match_edge_04, 4);
    vm_match_nested_case!(vm_match_edge_05, 5);
    vm_match_nested_case!(vm_match_edge_06, 6);
    vm_match_nested_case!(vm_match_edge_07, 7);
    vm_match_nested_case!(vm_match_edge_08, 8);
    vm_match_nested_case!(vm_match_edge_09, 9);
    vm_match_nested_case!(vm_match_edge_10, 10);

    macro_rules! vm_if_let_nested_case {
        ($name:ident, $k:expr) => {
            #[test]
            fn $name() {
                run_both(
                    &format!(
                        r#"internal func print_int(v: Int);
                           enum Option<T> {{ None, Some(T) }}
                           struct Point {{ x: Int, y: Int, }}
                           struct Wrap {{ p: Point, m: Int, }}
                           func mk(i: Int): Option<Wrap> {{
                               if i > 0 {{
                                   return Option::Some(Wrap {{ p: Point {{ x: i, y: i + 1 }}, m: i + 2 }});
                               }}
                               return Option::None;
                           }}
                           func main() {{
                               let out: Int = 0;
                               if let Option::Some(Wrap {{ p: Point {{ x, y: _ }}, m }}) = mk(3) {{
                                   out = x + m + {};
                               }} else {{
                                   out = 0;
                               }}
                               print_int(out);
                           }}"#,
                        $k
                    ),
                )
                .unwrap();
            }
        };
    }

    vm_if_let_nested_case!(vm_if_let_edge_01, 1);
    vm_if_let_nested_case!(vm_if_let_edge_02, 2);
    vm_if_let_nested_case!(vm_if_let_edge_03, 3);
    vm_if_let_nested_case!(vm_if_let_edge_04, 4);
    vm_if_let_nested_case!(vm_if_let_edge_05, 5);
}

