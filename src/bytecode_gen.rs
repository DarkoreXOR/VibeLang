//! AST -> bytecode generator.

use std::collections::{HashMap, HashSet};

use crate::ast::{AstNode, BinaryOp, CallArg, GenericParam, LambdaBody, Pattern, PatternElem, TypeExpr};
use crate::bytecode::{FunctionBytecode, Instr, ProgramBytecode};
use crate::error::Span;
use crate::type_key::{format_ty_key, match_type_keys, type_expr_to_ty_key, TyKey};

#[derive(Debug)]
pub struct CompileError {
    pub message: String,
    pub span: Option<Span>,
}

impl CompileError {
    fn new(message: impl Into<String>, span: Option<Span>) -> Self {
        Self {
            message: message.into(),
            span,
        }
    }

    pub fn format_with_file(&self, path: &str) -> String {
        match self.span {
            Some(s) => format!("{}:{}:{}: bytecode error: {}", path, s.line, s.column, self.message),
            None => format!("{}: bytecode error: {}", path, self.message),
        }
    }
}

fn validate_internal_func_decl(
    builtins: &crate::builtins::BuiltinRegistry,
    name: &str,
    params: &[crate::ast::Param],
    return_type: &Option<TypeExpr>,
    name_span: Span,
    is_async: bool,
) -> Result<(), CompileError> {
    builtins
        .validate_internal_func_decl(name, params, return_type, name_span, is_async)
        .map_err(|e| CompileError::new(e.message, e.span))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct LabelId(usize);

struct Labeler {
    next: usize,
    positions: Vec<Option<usize>>,
    // label_id -> list of instruction indices to patch
    unresolved: HashMap<usize, Vec<usize>>,
}

impl Labeler {
    fn new() -> Self {
        Self {
            next: 0,
            positions: Vec::new(),
            unresolved: HashMap::new(),
        }
    }

    fn new_label(&mut self) -> LabelId {
        let id = self.next;
        self.next += 1;
        self.positions.push(None);
        LabelId(id)
    }

    fn mark(&mut self, label: LabelId, code_len: usize) {
        let pos = code_len;
        self.positions[label.0] = Some(pos);
    }

    fn add_patch(&mut self, label: LabelId, patch_at_instr: usize) {
        self.unresolved
            .entry(label.0)
            .or_default()
            .push(patch_at_instr);
    }

    fn patch_all(&self, code: &mut [Instr]) {
        for (label_id, instr_idxs) in &self.unresolved {
            let Some(pos) = self.positions.get(*label_id).and_then(|x| *x) else {
                continue;
            };
            for idx in instr_idxs {
                match &mut code[*idx] {
                    Instr::Jump { target }
                    | Instr::JumpIfFalse { target }
                    | Instr::JumpIfTrue { target } => {
                        *target = pos;
                    }
                    _ => {}
                }
            }
        }
    }
}

#[derive(Clone, Copy)]
struct VarSlot {
    kind: VarSlotKind,
    slot: u32,
}

#[derive(Clone, Copy)]
enum VarSlotKind {
    Local,
    Global,
}

#[derive(Clone, Copy)]
struct LoopLabels {
    break_target: LabelId,
    continue_target: LabelId,
}

#[derive(Clone)]
struct CallParamSpec {
    name: String,
    is_params: bool,
    default_value: Option<AstNode>,
}

struct FnGen<'a> {
    globals: &'a HashMap<String, u32>,
    call_specs: &'a HashMap<String, Vec<CallParamSpec>>,
    call_returns: &'a HashMap<String, Option<TypeExpr>>,
    enum_names: &'a HashSet<String>,
    alias_enum_targets: &'a HashMap<String, String>,
    unit_struct_names: &'a HashSet<String>,
    /// `method` -> callees for generic array extensions, most specific first (matches semantic pass).
    generic_array_ext: &'a HashMap<String, Vec<String>>,
    /// `method` -> callees for generic enum extensions (e.g. `Result<T,E>::is_ok`).
    generic_enum_ext: &'a HashMap<String, Vec<String>>,
    /// `method` -> callees for generic struct-receiver extensions (e.g. `T::m`).
    generic_struct_ext: &'a HashMap<String, Vec<String>>,
    /// `callee` -> (`self` type template, function type parameters) for generic extensions.
    extension_match_info: &'a HashMap<String, (TyKey, Vec<String>)>,
    /// User/internal functions that return `Task` without eager execution at call sites.
    async_callees: &'a HashSet<String>,
    /// AST name for internal async builtins -> VM deferred callee symbol (`asyncSleep` -> `sleep`).
    internal_async_vm_callee: &'a HashMap<String, String>,
    scopes: Vec<HashMap<String, u32>>,
    local_type_keys: Vec<HashMap<String, TyKey>>,
    next_local_slot: u32,

    code: Vec<Instr>,
    labeler: Labeler,

    loop_stack: Vec<LoopLabels>,
    generated_functions: Vec<FunctionBytecode>,
    lambda_counter: usize,
    current_fn_name: String,
}

impl<'a> FnGen<'a> {
    fn new(
        globals: &'a HashMap<String, u32>,
        call_specs: &'a HashMap<String, Vec<CallParamSpec>>,
        call_returns: &'a HashMap<String, Option<TypeExpr>>,
        enum_names: &'a HashSet<String>,
        alias_enum_targets: &'a HashMap<String, String>,
        unit_struct_names: &'a HashSet<String>,
        generic_array_ext: &'a HashMap<String, Vec<String>>,
        generic_enum_ext: &'a HashMap<String, Vec<String>>,
        generic_struct_ext: &'a HashMap<String, Vec<String>>,
        extension_match_info: &'a HashMap<String, (TyKey, Vec<String>)>,
        async_callees: &'a HashSet<String>,
        internal_async_vm_callee: &'a HashMap<String, String>,
    ) -> Self {
        Self {
            globals,
            call_specs,
            call_returns,
            enum_names,
            alias_enum_targets,
            unit_struct_names,
            generic_array_ext,
            generic_enum_ext,
            generic_struct_ext,
            extension_match_info,
            async_callees,
            internal_async_vm_callee,
            scopes: Vec::new(),
            local_type_keys: Vec::new(),
            next_local_slot: 0,
            code: Vec::new(),
            labeler: Labeler::new(),
            loop_stack: Vec::new(),
            generated_functions: Vec::new(),
            lambda_counter: 0,
            current_fn_name: String::new(),
        }
    }

    fn emit_function_invoke(&mut self, callee: &str, argc: usize, span: Span) {
        if self.async_callees.contains(callee) {
            let vm_target = self
                .internal_async_vm_callee
                .get(callee)
                .map(|s| s.as_str())
                .unwrap_or(callee);
            self.emit(Instr::MakeDeferredTask {
                func: vm_target.to_string(),
                argc,
                span,
            });
        } else {
            self.emit(Instr::Call {
                callee: callee.to_string(),
                argc,
                span,
            });
        }
    }

    fn push_scope(&mut self) {
        self.scopes.push(HashMap::new());
        self.local_type_keys.push(HashMap::new());
    }

    fn pop_scope(&mut self) {
        self.scopes.pop();
        self.local_type_keys.pop();
    }

    fn alloc_slot(&mut self) -> u32 {
        let s = self.next_local_slot;
        self.next_local_slot += 1;
        s
    }

    fn resolve_call_arguments(
        &self,
        callee: &str,
        arguments: &[CallArg],
        span: Span,
    ) -> Result<Vec<AstNode>, CompileError> {
        let Some(spec) = self.call_specs.get(callee) else {
            let mut out = Vec::with_capacity(arguments.len());
            for arg in arguments {
                match arg {
                    CallArg::Positional(expr) => out.push(expr.clone()),
                    CallArg::Named { name, name_span, .. } => {
                        return Err(CompileError::new(
                            format!("unknown named argument `{name}` for `{callee}`"),
                            Some(*name_span),
                        ));
                    }
                }
            }
            return Ok(out);
        };

        let params_index = spec.iter().position(|p| p.is_params);
        let fixed_count = params_index.unwrap_or(spec.len());
        let mut ordered: Vec<Option<AstNode>> = vec![None; spec.len()];
        let mut saw_named = false;
        let mut positional_idx = 0usize;
        let mut packed_params: Vec<AstNode> = Vec::new();
        let mut params_explicit_by_name = false;
        for arg in arguments {
            match arg {
                CallArg::Positional(expr) => {
                    if saw_named {
                        return Err(CompileError::new(
                            "positional arguments cannot follow named arguments",
                            Some(span),
                        ));
                    }
                    if positional_idx >= fixed_count {
                        if params_index.is_some() {
                            packed_params.push(expr.clone());
                            continue;
                        }
                        return Err(CompileError::new(
                            format!(
                                "function `{callee}` expects {} argument(s), found at least {}",
                                spec.len(),
                                positional_idx + 1
                            ),
                            Some(span),
                        ));
                    }
                    ordered[positional_idx] = Some(expr.clone());
                    positional_idx += 1;
                }
                CallArg::Named {
                    name,
                    name_span,
                    value,
                } => {
                    saw_named = true;
                    let Some(i) = spec.iter().position(|p| p.name == *name) else {
                        return Err(CompileError::new(
                            format!("unknown named argument `{name}` for `{callee}`"),
                            Some(*name_span),
                        ));
                    };
                    if ordered[i].is_some() {
                        return Err(CompileError::new(
                            format!("duplicate argument `{name}`"),
                            Some(*name_span),
                        ));
                    }
                    if Some(i) == params_index {
                        params_explicit_by_name = true;
                    }
                    ordered[i] = Some(value.clone());
                }
            }
        }

        if params_explicit_by_name && !packed_params.is_empty() {
            return Err(CompileError::new(
                "cannot use both packed positional arguments and explicit named `params` argument",
                Some(span),
            ));
        }

        let mut out = Vec::with_capacity(spec.len());
        for (i, entry) in ordered.iter().enumerate() {
            if Some(i) == params_index && !params_explicit_by_name {
                out.push(AstNode::ArrayLiteral {
                    elements: packed_params.clone(),
                    span,
                });
                continue;
            }
            if let Some(expr) = entry {
                out.push(expr.clone());
            } else if let Some(def) = spec[i].default_value.as_ref() {
                out.push(def.clone());
            } else {
                return Err(CompileError::new(
                    format!("missing required argument `{}` to `{callee}`", spec[i].name),
                    Some(span),
                ));
            }
        }
        Ok(out)
    }

    fn infer_receiver_ty_key(&self, expr: &AstNode) -> Option<TyKey> {
        match expr {
            AstNode::IntegerLiteral { .. } => Some(TyKey::Ident("Int".to_string())),
            AstNode::FloatLiteral { .. } => Some(TyKey::Ident("Float".to_string())),
            AstNode::StringLiteral { .. } => Some(TyKey::Ident("String".to_string())),
            AstNode::BoolLiteral { .. } => Some(TyKey::Ident("Bool".to_string())),
            AstNode::UnitLiteral { .. } => Some(TyKey::Unit),
            AstNode::ArrayLiteral { elements, .. } => {
                if elements.is_empty() {
                    return None;
                }
                let elem = self.infer_receiver_ty_key(&elements[0])?;
                Some(TyKey::Array(Box::new(elem)))
            }
            AstNode::DictLiteral { entries, .. } => {
                if entries.is_empty() {
                    return None;
                }
                let (k0, v0) = &entries[0];
                let key_ty = self.infer_receiver_ty_key(k0)?;
                let val_ty = self.infer_receiver_ty_key(v0)?;
                Some(TyKey::EnumApp {
                    name: "Dict".to_string(),
                    args: vec![key_ty, val_ty],
                })
            }
            AstNode::Identifier { name, .. } => self
                .local_type_keys
                .iter()
                .rev()
                .find_map(|s| s.get(name).cloned())
                .or_else(|| {
                    if self.unit_struct_names.contains(name) {
                        Some(TyKey::Ident(name.clone()))
                    } else {
                        None
                    }
                }),
            AstNode::StructLiteral {
                name, type_args, ..
            } => {
                if type_args.is_empty() {
                    Some(TyKey::Ident(name.clone()))
                } else {
                    let mut parts = Vec::new();
                    for a in type_args {
                        parts.push(type_expr_to_ty_key(a)?);
                    }
                    Some(TyKey::Ident(format!(
                        "{}<{}>",
                        name,
                        parts.iter().map(format_ty_key).collect::<Vec<_>>().join(", ")
                    )))
                }
            }
            AstNode::Call { callee, .. } => self
                .call_returns
                .get(callee)
                .and_then(|t| t.as_ref())
                .and_then(|t| type_expr_to_ty_key(t)),
            AstNode::TypeValue { type_name, .. } => Some(TyKey::Ident(type_name.clone())),
            AstNode::TypeMethodCall {
                type_name, method, ..
            } => {
                let callee = format!("{type_name}::{method}");
                self.call_returns
                    .get(&callee)
                    .and_then(|t| t.as_ref())
                    .and_then(|t| type_expr_to_ty_key(t))
            }
            AstNode::MethodCall {
                receiver, method, ..
            } => {
                let recv_key = self.infer_receiver_ty_key(receiver.as_ref())?;
                let callee = format!("{}::{}", format_ty_key(&recv_key), method);
                self.call_returns
                    .get(&callee)
                    .and_then(|t| t.as_ref())
                    .and_then(|t| type_expr_to_ty_key(t))
            }
            AstNode::EnumVariantCtor {
                enum_name,
                type_args,
                variant: _,
                payloads,
                ..
            } => {
                let mut arg_keys = Vec::new();
                let mut payload_idx = 0usize;
                for ta in type_args {
                    match ta {
                        TypeExpr::Infer => {
                            let p = payloads.get(payload_idx)?;
                            arg_keys.push(self.infer_receiver_ty_key(p)?);
                            payload_idx += 1;
                        }
                        _ => {
                            arg_keys.push(type_expr_to_ty_key(ta)?);
                        }
                    }
                }
                Some(TyKey::EnumApp {
                    name: enum_name.clone(),
                    args: arg_keys,
                })
            }
            _ => None,
        }
    }

    fn declare_pattern(&mut self, pattern: &Pattern) -> Result<(), CompileError> {
        match pattern {
            Pattern::Wildcard { .. } => Ok(()),
            Pattern::IntLiteral { .. }
            | Pattern::StringLiteral { .. }
            | Pattern::BoolLiteral { .. } => Ok(()),
            Pattern::Binding { name, .. } => {
                let slot = self.alloc_slot();
                self.scopes
                    .last_mut()
                    .expect("scope stack")
                    .insert(name.clone(), slot);
                Ok(())
            }
            Pattern::Tuple { elements, .. } => {
                for e in elements {
                    match e {
                        PatternElem::Rest(_) => {}
                        PatternElem::Pattern(p) => self.declare_pattern(p)?,
                    }
                }
                Ok(())
            }
            Pattern::Array { elements, .. } => {
                for e in elements {
                    match e {
                        PatternElem::Rest(_) => {}
                        PatternElem::Pattern(p) => self.declare_pattern(p)?,
                    }
                }
                Ok(())
            }
            Pattern::Struct { fields, .. } => {
                for f in fields {
                    self.declare_pattern(&f.pattern)?;
                }
                Ok(())
            }
            Pattern::EnumVariant { payloads, .. } => {
                for p in payloads {
                    self.declare_pattern(p)?;
                }
                Ok(())
            }
        }
    }

    fn resolve_var(&self, name: &str) -> Option<VarSlot> {
        for m in self.scopes.iter().rev() {
            if let Some(&slot) = m.get(name) {
                return Some(VarSlot {
                    kind: VarSlotKind::Local,
                    slot,
                });
            }
        }
        self.globals.get(name).map(|slot| VarSlot {
            kind: VarSlotKind::Global,
            slot: *slot,
        })
    }

    fn extract_array_index_chain<'b>(
        &self,
        lhs: &'b AstNode,
    ) -> Option<(VarSlot, Vec<(&'b AstNode, Span)>)> {
        let mut indices_rev: Vec<(&AstNode, Span)> = Vec::new();
        let mut cur = lhs;
        loop {
            match cur {
                AstNode::ArrayIndex { base, index, span } => {
                    indices_rev.push((index.as_ref(), *span));
                    cur = base.as_ref();
                }
                AstNode::Identifier { name, .. } => {
                    let root = self.resolve_var(name)?;
                    let indices = indices_rev.into_iter().rev().collect::<Vec<_>>();
                    return Some((root, indices));
                }
                _ => return None,
            }
        }
    }

    fn extract_field_access_chain<'b>(
        &self,
        lhs: &'b AstNode,
    ) -> Option<(VarSlot, Vec<(&'b str, Span)>)> {
        let mut fields_rev: Vec<(&str, Span)> = Vec::new();
        let mut cur = lhs;
        loop {
            match cur {
                AstNode::FieldAccess { base, field, span } => {
                    fields_rev.push((field.as_str(), *span));
                    cur = base.as_ref();
                }
                AstNode::Identifier { name, .. } => {
                    let root = self.resolve_var(name)?;
                    let fields = fields_rev.into_iter().rev().collect::<Vec<_>>();
                    return Some((root, fields));
                }
                _ => return None,
            }
        }
    }

    fn emit(&mut self, instr: Instr) {
        self.code.push(instr);
    }

    fn emit_jump(&mut self, label: LabelId, mk: impl FnOnce(usize) -> Instr) {
        let at = self.code.len();
        self.emit(mk(0));
        self.labeler.add_patch(label, at);
    }

    fn compile_block(&mut self, stmts: &[AstNode]) -> Result<(), CompileError> {
        self.push_scope();
        for s in stmts {
            self.compile_stmt(s)?;
        }
        self.pop_scope();
        Ok(())
    }

    fn store_pattern_value(&mut self, pattern: &Pattern) -> Result<(), CompileError> {
        match pattern {
            Pattern::Wildcard { .. } => {
                self.emit(Instr::Pop);
                Ok(())
            }
            Pattern::IntLiteral {
                original,
                radix,
                span,
                ..
            } => {
                self.emit(Instr::PushInt {
                    value: original.clone(),
                    radix: *radix,
                    span: *span,
                });
                self.emit(Instr::BinOp {
                    op: BinaryOp::Eq,
                    span: *span,
                });
                self.emit(Instr::AssertBool { span: *span });
                Ok(())
            }
            Pattern::StringLiteral { value, span } => {
                self.emit(Instr::PushString { value: value.clone() });
                self.emit(Instr::BinOp {
                    op: BinaryOp::Eq,
                    span: *span,
                });
                self.emit(Instr::AssertBool { span: *span });
                Ok(())
            }
            Pattern::BoolLiteral { value, span } => {
                self.emit(Instr::PushBool { value: *value });
                self.emit(Instr::BinOp {
                    op: BinaryOp::Eq,
                    span: *span,
                });
                self.emit(Instr::AssertBool { span: *span });
                Ok(())
            }
            Pattern::Binding { name, name_span } => {
                let Some(v) = self.resolve_var(name) else {
                    return Err(CompileError::new(
                        format!("unknown variable `{name}`"),
                        Some(*name_span),
                    ));
                };
                match v.kind {
                    VarSlotKind::Local => self.emit(Instr::StoreLocal {
                        slot: v.slot,
                        span: *name_span,
                    }),
                    VarSlotKind::Global => self.emit(Instr::StoreGlobal {
                        slot: v.slot,
                        span: *name_span,
                    }),
                }
                Ok(())
            }
            Pattern::Tuple {
                elements,
                span: tuple_span,
            } => {
                // Preserve tuple on a temp local slot so we can extract multiple fields.
                let tmp = self.alloc_slot();
                self.emit(Instr::StoreLocal {
                    slot: tmp,
                    span: *tuple_span,
                });

                // Split pattern elements around `..` rest.
                let mut rest_idx: Option<usize> = None;
                for (i, e) in elements.iter().enumerate() {
                    if matches!(e, PatternElem::Rest(_)) {
                        rest_idx = Some(i);
                        break;
                    }
                }

                let (prefix, suffix, has_rest) = match rest_idx {
                    None => (elements.iter().collect::<Vec<_>>(), Vec::new(), false),
                    Some(i) => (
                        elements[0..i].iter().collect::<Vec<_>>(),
                        elements[i + 1..].iter().collect::<Vec<_>>(),
                        true,
                    ),
                };

                // Prefix fields are extracted by fixed indices from the start.
                for (i, elem) in prefix.iter().enumerate() {
                    match elem {
                        PatternElem::Rest(_) => {}
                        PatternElem::Pattern(p) => {
                            self.emit(Instr::LoadLocal {
                                slot: tmp,
                                span: *tuple_span,
                            });
                            self.emit(Instr::GetTupleField {
                                index: i as u32,
                                span: *tuple_span,
                            });
                            self.store_pattern_value(p)?;
                        }
                    }
                }

                // Suffix fields are extracted from the end (so we don't need tuple length).
                if has_rest {
                    let suffix_len = suffix.len();
                    for (j, elem) in suffix.iter().enumerate() {
                        match elem {
                            PatternElem::Rest(_) => {}
                            PatternElem::Pattern(p) => {
                                let offset_from_end = (suffix_len - 1 - j) as u32;
                                self.emit(Instr::LoadLocal {
                                    slot: tmp,
                                    span: *tuple_span,
                                });
                                self.emit(Instr::GetTupleFieldFromEnd {
                                    offset_from_end,
                                    span: *tuple_span,
                                });
                                self.store_pattern_value(p)?;
                            }
                        }
                    }
                }

                Ok(())
            }
            Pattern::Array {
                elements,
                span: array_span,
            } => {
                // Preserve array on a temp so we can extract multiple elements.
                let tmp = self.alloc_slot();
                self.emit(Instr::StoreLocal {
                    slot: tmp,
                    span: *array_span,
                });

                // Split around `..` (if any).
                let mut rest_idx: Option<usize> = None;
                for (i, e) in elements.iter().enumerate() {
                    if matches!(e, PatternElem::Rest(_)) {
                        rest_idx = Some(i);
                        break;
                    }
                }

                let (prefix, suffix, has_rest) = match rest_idx {
                    None => (elements.iter().collect::<Vec<_>>(), Vec::new(), false),
                    Some(i) => (
                        elements[0..i].iter().collect::<Vec<_>>(),
                        elements[i + 1..].iter().collect::<Vec<_>>(),
                        true,
                    ),
                };

                if !has_rest {
                    // No rest => pattern must match exact runtime length.
                    self.emit(Instr::LoadLocal {
                        slot: tmp,
                        span: *array_span,
                    });
                    self.emit(Instr::AssertArrayLenEq {
                        expected: elements.len(),
                        span: *array_span,
                    });
                }

                // Prefix elements.
                for (i, elem) in prefix.iter().enumerate() {
                    match elem {
                        PatternElem::Rest(_) => {}
                        PatternElem::Pattern(p) => {
                            self.emit(Instr::LoadLocal {
                                slot: tmp,
                                span: *array_span,
                            });
                            self.emit(Instr::PushInt {
                                value: i.to_string(),
                                radix: 10,
                                span: *array_span,
                            });
                            self.emit(Instr::GetArrayIndex { span: *array_span });
                            self.store_pattern_value(p)?;
                        }
                    }
                }

                // Suffix elements extracted from the end.
                if has_rest {
                    let suffix_len = suffix.len();
                    for (j, elem) in suffix.iter().enumerate() {
                        match elem {
                            PatternElem::Rest(_) => {}
                            PatternElem::Pattern(p) => {
                                let offset_from_end = (suffix_len - 1 - j) as u32;
                                self.emit(Instr::LoadLocal {
                                    slot: tmp,
                                    span: *array_span,
                                });
                                self.emit(Instr::GetArrayIndexFromEnd {
                                    offset_from_end,
                                    span: *array_span,
                                });
                                self.store_pattern_value(p)?;
                            }
                        }
                    }
                }

                Ok(())
            }
            Pattern::Struct {
                fields,
                rest: _,
                span: struct_span,
                ..
            } => {
                // Preserve struct on a temp local so we can extract fields multiple times.
                let tmp = self.alloc_slot();
                self.emit(Instr::StoreLocal {
                    slot: tmp,
                    span: *struct_span,
                });

                for f in fields {
                    self.emit(Instr::LoadLocal {
                        slot: tmp,
                        span: *struct_span,
                    });
                    self.emit(Instr::GetStructField {
                        field: f.name.clone(),
                        span: *struct_span,
                    });
                    self.store_pattern_value(&f.pattern)?;
                }

                Ok(())
            }
            Pattern::EnumVariant {
                enum_name,
                variant,
                payloads,
                span,
                ..
            } => {
                // Destructure an enum value into payloads (and fail at runtime if mismatched).
                self.emit(Instr::UnpackEnumVariant {
                    enum_name: enum_name.clone(),
                    variant: variant.clone(),
                    payload_count: payloads.len(),
                    span: *span,
                });

                // Bind by consuming payloads from the stack.
                for p in payloads.iter().rev() {
                    self.store_pattern_value(p)?;
                }
                Ok(())
            }
        }
    }

    /// Match a pattern against the value currently on top of the stack.
    ///
    /// On success: consumes the value and binds any `Pattern::Binding` occurrences.
    /// On mismatch: jumps to `fail_lbl` (no value is kept on the stack).
    fn match_and_bind_pattern(
        &mut self,
        pattern: &Pattern,
        fail_lbl: LabelId,
    ) -> Result<(), CompileError> {
        match pattern {
            Pattern::Wildcard { .. } => self.store_pattern_value(pattern),
            Pattern::Binding { .. } => self.store_pattern_value(pattern),

            Pattern::IntLiteral {
                original,
                radix,
                span,
                ..
            } => {
                self.emit(Instr::PushInt {
                    value: original.clone(),
                    radix: *radix,
                    span: *span,
                });
                self.emit(Instr::BinOp {
                    op: BinaryOp::Eq,
                    span: *span,
                });
                self.emit_jump(fail_lbl, |target| Instr::JumpIfFalse { target });
                Ok(())
            }
            Pattern::BoolLiteral { value, span } => {
                self.emit(Instr::PushBool { value: *value });
                self.emit(Instr::BinOp {
                    op: BinaryOp::Eq,
                    span: *span,
                });
                self.emit_jump(fail_lbl, |target| Instr::JumpIfFalse { target });
                Ok(())
            }
            Pattern::StringLiteral { value, span } => {
                self.emit(Instr::PushString { value: value.clone() });
                self.emit(Instr::BinOp {
                    op: BinaryOp::Eq,
                    span: *span,
                });
                self.emit_jump(fail_lbl, |target| Instr::JumpIfFalse { target });
                Ok(())
            }

            Pattern::Struct {
                name,
                type_args,
                fields,
                rest: _,
                span,
                ..
            } => {
                // Extract struct fields via a temp local to allow nested pattern checks.
                let tmp = self.alloc_slot();
                self.emit(Instr::StoreLocal { slot: tmp, span: *span });

                if fields.is_empty() {
                    let concrete_name = if type_args.is_empty() {
                        name.clone()
                    } else {
                        let mut parts = Vec::new();
                        for a in type_args {
                            let tk = type_expr_to_ty_key(a).ok_or_else(|| {
                                CompileError::new(
                                    "struct pattern type arguments must be concrete compile-time types",
                                    Some(*span),
                                )
                            })?;
                            parts.push(format_ty_key(&tk));
                        }
                        format!("{name}<{}>", parts.join(", "))
                    };
                    self.emit(Instr::LoadLocal { slot: tmp, span: *span });
                    self.emit(Instr::MatchStructName {
                        name: concrete_name,
                        span: *span,
                    });
                    self.emit_jump(fail_lbl, |target| Instr::JumpIfFalse { target });
                }

                for f in fields {
                    self.emit(Instr::LoadLocal { slot: tmp, span: *span });
                    self.emit(Instr::GetStructField {
                        field: f.name.clone(),
                        span: *span,
                    });
                    self.match_and_bind_pattern(&f.pattern, fail_lbl)?;
                }
                Ok(())
            }

            // Not implemented yet for `match` (Rust-like literal patterns exist, but this
            // VM implementation currently only routes failures via enum-tag checks
            // and struct-field checks).
            Pattern::Tuple { .. } | Pattern::Array { .. } | Pattern::EnumVariant { .. } => Err(
                CompileError::new("unsupported pattern kind in `match`", Some(self.span_of_pattern(pattern))),
            ),
        }
    }

    fn span_of_pattern(&self, pattern: &Pattern) -> Span {
        // Small helper for error spans in new code.
        match pattern {
            Pattern::Wildcard { span } => *span,
            Pattern::Binding { name_span, .. } => *name_span,
            Pattern::IntLiteral { span, .. } => *span,
            Pattern::StringLiteral { span, .. } => *span,
            Pattern::BoolLiteral { span, .. } => *span,
            Pattern::Tuple { span, .. } => *span,
            Pattern::Array { span, .. } => *span,
            Pattern::Struct { span, .. } => *span,
            Pattern::EnumVariant { span, .. } => *span,
        }
    }

    fn compile_expr(&mut self, expr: &AstNode) -> Result<(), CompileError> {
        match expr {
            AstNode::IntegerLiteral {
                original,
                radix,
                span,
                ..
            } => {
                self.emit(Instr::PushInt {
                    value: original.clone(),
                    radix: *radix,
                    span: *span,
                });
                Ok(())
            }
            AstNode::FloatLiteral { cleaned, span, .. } => {
                self.emit(Instr::PushFloat {
                    value: cleaned.clone(),
                    span: *span,
                });
                Ok(())
            }
            AstNode::StringLiteral { value, .. } => {
                self.emit(Instr::PushString {
                    value: value.clone(),
                });
                Ok(())
            }
            AstNode::BoolLiteral { value, .. } => {
                self.emit(Instr::PushBool { value: *value });
                Ok(())
            }
            AstNode::UnitLiteral { .. } => {
                self.emit(Instr::PushUnit);
                Ok(())
            }
            AstNode::Identifier { name, span } => {
                if let Some(v) = self.resolve_var(name) {
                    match v.kind {
                        VarSlotKind::Local => self.emit(Instr::LoadLocal { slot: v.slot, span: *span }),
                        VarSlotKind::Global => {
                            self.emit(Instr::LoadGlobal { slot: v.slot, span: *span })
                        }
                    }
                    Ok(())
                } else if self.call_specs.contains_key(name) {
                    self.emit(Instr::MakeClosure {
                        callee: name.clone(),
                        capture_locals: Vec::new(),
                        span: *span,
                    });
                    Ok(())
                } else if self.unit_struct_names.contains(name) {
                    self.emit(Instr::MakeStructLiteral {
                        name: name.clone(),
                        is_unit_literal: true,
                        field_names: Vec::new(),
                        has_update: false,
                        span: *span,
                    });
                    Ok(())
                } else {
                    Err(CompileError::new(
                        format!("unknown variable `{name}`"),
                        Some(*span),
                    ))
                }
            }
            AstNode::TypeValue { type_name, span } => {
                if let Some(base) = type_name.split_once('<').map(|(b, _)| b).or(Some(type_name.as_str()))
                {
                    if self.unit_struct_names.contains(base) {
                        self.emit(Instr::MakeStructLiteral {
                            name: type_name.clone(),
                            is_unit_literal: true,
                            field_names: Vec::new(),
                            has_update: false,
                            span: *span,
                        });
                        return Ok(());
                    }
                }
                Err(CompileError::new(
                    format!("`{type_name}` is not a valid runtime type value"),
                    Some(*span),
                ))
            }
            AstNode::TupleLiteral { elements, .. } => {
                for e in elements {
                    self.compile_expr(e)?;
                }
                self.emit(Instr::MakeTuple { count: elements.len() });
                Ok(())
            }
            AstNode::TupleField { base, index, span } => {
                self.compile_expr(base)?;
                self.emit(Instr::GetTupleField {
                    index: *index,
                    span: *span,
                });
                Ok(())
            }
            AstNode::ArrayLiteral { elements, .. } => {
                for e in elements {
                    self.compile_expr(e)?;
                }
                self.emit(Instr::MakeArray { count: elements.len() });
                Ok(())
            }
            AstNode::DictLiteral { entries, span } => {
                // Encode runtime dict as a struct with a single `entries` field:
                //   Dict { entries: [(key, value), ...] }
                // We store each entry as a 2-tuple and wrap the array in `entries`.
                for (k, v) in entries {
                    self.compile_expr(k)?;
                    self.compile_expr(v)?;
                    self.emit(Instr::MakeTuple { count: 2 });
                }
                self.emit(Instr::MakeArray { count: entries.len() });
                self.emit(Instr::MakeStructLiteral {
                    name: "Dict".to_string(),
                    is_unit_literal: false,
                    field_names: vec!["entries".to_string()],
                    has_update: false,
                    span: *span,
                });
                Ok(())
            }
            AstNode::ArrayIndex { base, index, span } => {
                self.compile_expr(base)?;
                self.compile_expr(index)?;
                self.emit(Instr::GetArrayIndex { span: *span });
                Ok(())
            }
            AstNode::StructLiteral {
                name,
                type_args,
                fields,
                update,
                span,
            } => {
                let mut field_names = Vec::with_capacity(fields.len());
                // Evaluate optional update base first (it's deeper on the stack).
                let has_update = if let Some(u) = update {
                    self.compile_expr(u.as_ref())?;
                    true
                } else {
                    false
                };

                for (fname, expr) in fields {
                    field_names.push(fname.clone());
                    self.compile_expr(expr)?;
                }

                let concrete_name = if type_args.is_empty() {
                    name.clone()
                } else {
                    let mut parts = Vec::new();
                    for a in type_args {
                        let tk = type_expr_to_ty_key(a).ok_or_else(|| {
                            CompileError::new(
                                "struct literal type arguments must be concrete compile-time types",
                                Some(*span),
                            )
                        })?;
                        parts.push(format_ty_key(&tk));
                    }
                    format!("{}<{}>", name, parts.join(", "))
                };
                self.emit(Instr::MakeStructLiteral {
                    name: concrete_name,
                    is_unit_literal: false,
                    field_names,
                    has_update,
                    span: *span,
                });
                Ok(())
            }
            AstNode::FieldAccess { base, field, span } => {
                self.compile_expr(base)?;
                self.emit(Instr::GetStructField {
                    field: field.clone(),
                    span: *span,
                });
                Ok(())
            }
            AstNode::EnumVariantCtor {
                enum_name,
                type_args: _,
                variant,
                payloads,
                span,
            } => {
                for p in payloads {
                    self.compile_expr(p)?;
                }
                self.emit(Instr::MakeEnumVariant {
                    enum_name: enum_name.clone(),
                    variant: variant.clone(),
                    payload_count: payloads.len(),
                    span: *span,
                });
                Ok(())
            }
            AstNode::UnaryOp { op, operand, span } => {
                self.compile_expr(operand)?;
                self.emit(Instr::UnOp { op: *op, span: *span });
                Ok(())
            }
            AstNode::BinaryOp {
                left,
                op,
                right,
                span,
            } => {
                match op {
                    BinaryOp::And => {
                        // Short-circuit:
                        // left && right
                        // if left false -> push false, else evaluate right.
                        let false_lbl = self.labeler.new_label();
                        let end_lbl = self.labeler.new_label();

                        self.compile_expr(left)?;
                        self.emit_jump(false_lbl, |target| Instr::JumpIfFalse { target });

                        self.compile_expr(right)?;
                        self.emit_jump(end_lbl, |target| Instr::Jump { target });

                        self.labeler.mark(false_lbl, self.code.len());
                        self.emit(Instr::PushBool { value: false });

                        self.labeler.mark(end_lbl, self.code.len());
                        Ok(())
                    }
                    BinaryOp::Or => {
                        let true_lbl = self.labeler.new_label();
                        let end_lbl = self.labeler.new_label();

                        self.compile_expr(left)?;
                        self.emit_jump(true_lbl, |target| Instr::JumpIfTrue { target });

                        self.compile_expr(right)?;
                        self.emit_jump(end_lbl, |target| Instr::Jump { target });

                        self.labeler.mark(true_lbl, self.code.len());
                        self.emit(Instr::PushBool { value: true });

                        self.labeler.mark(end_lbl, self.code.len());
                        Ok(())
                    }
                    other => {
                        self.compile_expr(left)?;
                        self.compile_expr(right)?;
                        self.emit(Instr::BinOp { op: *other, span: *span });
                        Ok(())
                    }
                }
            }
            AstNode::Await { expr, span } => {
                // Special-case `await` on identifiers:
                // After awaiting, overwrite the original variable slot with the payload so the
                // program can later use the identifier as its completed value (matches the
                // expectations in `examples/example30.vc`).
                if let AstNode::Identifier { name, .. } = expr.as_ref() {
                    if let Some(slot) = self.resolve_var(name) {
                        match slot.kind {
                            VarSlotKind::Local => {
                                self.emit(Instr::LoadLocal { slot: slot.slot, span: *span });
                            }
                            VarSlotKind::Global => {
                                self.emit(Instr::LoadGlobal { slot: slot.slot, span: *span });
                            }
                        }
                        self.emit(Instr::AwaitTask { span: *span });
                        match slot.kind {
                            VarSlotKind::Local => {
                                self.emit(Instr::StoreLocal { slot: slot.slot, span: *span });
                                self.emit(Instr::LoadLocal { slot: slot.slot, span: *span });
                            }
                            VarSlotKind::Global => {
                                self.emit(Instr::StoreGlobal { slot: slot.slot, span: *span });
                                self.emit(Instr::LoadGlobal { slot: slot.slot, span: *span });
                            }
                        }
                        return Ok(());
                    }
                }

                self.compile_expr(expr.as_ref())?;
                self.emit(Instr::AwaitTask { span: *span });
                Ok(())
            }
            AstNode::Call {
                callee,
                type_args: _,
                arguments,
                span,
            } => {
                if self.resolve_var(callee).is_some() {
                    self.compile_expr(&AstNode::Identifier {
                        name: callee.clone(),
                        span: *span,
                    })?;
                    for a in arguments {
                        match a {
                            CallArg::Positional(v) => self.compile_expr(v)?,
                            CallArg::Named { value, .. } => self.compile_expr(value)?,
                        }
                    }
                    self.emit(Instr::CallClosure {
                        argc: arguments.len(),
                        span: *span,
                    });
                    return Ok(());
                }
                let resolved = self.resolve_call_arguments(callee, arguments, *span)?;
                for a in resolved {
                    self.compile_expr(&a)?;
                }
                let argc = self
                    .call_specs
                    .get(callee)
                    .map(|p| p.len())
                    .unwrap_or(arguments.len());
                self.emit_function_invoke(callee, argc, *span);
                Ok(())
            }
            AstNode::Invoke {
                callee,
                arguments,
                span,
            } => {
                self.compile_expr(callee.as_ref())?;
                for a in arguments {
                    match a {
                        CallArg::Positional(v) => self.compile_expr(v)?,
                        CallArg::Named { value, .. } => self.compile_expr(value)?,
                    }
                }
                self.emit(Instr::CallClosure {
                    argc: arguments.len(),
                    span: *span,
                });
                Ok(())
            }
            AstNode::Lambda { params, body, span } => {
                fn collect_used_names(node: &AstNode, out: &mut std::collections::HashSet<String>) {
                    match node {
                        AstNode::Identifier { name, .. } => {
                            out.insert(name.clone());
                        }
                        AstNode::UnaryOp { operand, .. } => collect_used_names(operand, out),
                        AstNode::BinaryOp { left, right, .. } => {
                            collect_used_names(left, out);
                            collect_used_names(right, out);
                        }
                        AstNode::Call { arguments, .. } | AstNode::Invoke { arguments, .. } => {
                            for a in arguments {
                                match a {
                                    CallArg::Positional(v) => collect_used_names(v, out),
                                    CallArg::Named { value, .. } => collect_used_names(value, out),
                                }
                            }
                        }
                        AstNode::MethodCall { receiver, arguments, .. } => {
                            collect_used_names(receiver, out);
                            for a in arguments {
                                match a {
                                    CallArg::Positional(v) => collect_used_names(v, out),
                                    CallArg::Named { value, .. } => collect_used_names(value, out),
                                }
                            }
                        }
                        AstNode::Return { value, .. } => {
                            if let Some(v) = value {
                                collect_used_names(v, out);
                            }
                        }
                        AstNode::Block { body, .. } => {
                            for s in body {
                                collect_used_names(s, out);
                            }
                        }
                        AstNode::Let { initializer, .. } => {
                            if let Some(v) = initializer {
                                collect_used_names(v, out);
                            }
                        }
                        AstNode::Assign { value, .. } => collect_used_names(value, out),
                        AstNode::AssignExpr { lhs, rhs, .. }
                        | AstNode::CompoundAssign { lhs, rhs, .. } => {
                            collect_used_names(lhs, out);
                            collect_used_names(rhs, out);
                        }
                        AstNode::While { condition, body, .. } => {
                            collect_used_names(condition, out);
                            for s in body {
                                collect_used_names(s, out);
                            }
                        }
                        AstNode::Await { expr, .. } => collect_used_names(expr, out),
                        _ => {}
                    }
                }
                let mut used_names = std::collections::HashSet::new();
                match body.as_ref() {
                    LambdaBody::Expr(e) => collect_used_names(e, &mut used_names),
                    LambdaBody::Block(items) => {
                        for s in items {
                            collect_used_names(s, &mut used_names);
                        }
                    }
                }
                for p in params {
                    used_names.remove(&p.name);
                }
                let mut capture_bindings: Vec<(String, u32)> = self
                    .scopes
                    .iter()
                    .flat_map(|s| s.iter().map(|(k, v)| (k.clone(), *v)))
                    .filter(|(k, _)| used_names.contains(k))
                    .collect();
                capture_bindings.sort_by(|a, b| a.1.cmp(&b.1).then(a.0.cmp(&b.0)));
                capture_bindings.dedup_by(|a, b| a.1 == b.1);
                let capture_locals: Vec<u32> = capture_bindings.iter().map(|(_, s)| *s).collect();

                let lambda_name = format!("{}$lambda${}", self.current_fn_name, self.lambda_counter);
                self.lambda_counter += 1;
                let mut child = FnGen::new(
                    self.globals,
                    self.call_specs,
                    self.call_returns,
                    self.enum_names,
                    self.alias_enum_targets,
                    self.unit_struct_names,
                    self.generic_array_ext,
                    self.generic_enum_ext,
                    self.generic_struct_ext,
                    self.extension_match_info,
                    self.async_callees,
                    self.internal_async_vm_callee,
                );
                child.current_fn_name = lambda_name.clone();
                child.push_scope();
                let mut param_slots: Vec<Option<u32>> = Vec::new();
                for (cap_name, _cap_slot) in &capture_bindings {
                    let slot = child.alloc_slot();
                    child
                        .scopes
                        .last_mut()
                        .expect("scope stack")
                        .insert(cap_name.clone(), slot);
                    param_slots.push(Some(slot));
                }
                for p in params {
                    let slot = child.alloc_slot();
                    child
                        .scopes
                        .last_mut()
                        .expect("scope stack")
                        .insert(p.name.clone(), slot);
                    param_slots.push(Some(slot));
                }
                match body.as_ref() {
                    LambdaBody::Expr(e) => {
                        child.compile_expr(e)?;
                        child.emit(Instr::Return { span: *span });
                    }
                    LambdaBody::Block(items) => {
                        for s in items {
                            child.compile_stmt(s)?;
                        }
                        child.emit(Instr::PushUnit);
                        child.emit(Instr::Return { span: *span });
                    }
                }
                child.labeler.patch_all(&mut child.code);
                self.generated_functions.push(FunctionBytecode {
                    name: lambda_name.clone(),
                    code: child.code,
                    local_count: child.next_local_slot,
                    param_count: capture_locals.len() + params.len(),
                    param_slots,
                    span: *span,
                });
                self.generated_functions.extend(child.generated_functions);
                self.emit(Instr::MakeClosure {
                    callee: lambda_name,
                    capture_locals,
                    span: *span,
                });
                Ok(())
            }
            AstNode::MethodCall {
                receiver,
                method,
                arguments,
                span,
            } => {
                let Some(inferred_tk) = self.infer_receiver_ty_key(receiver.as_ref()) else {
                    return Err(CompileError::new(
                        format!(
                            "cannot resolve receiver type for method call `.{method}` at compile time"
                        ),
                        Some(*span),
                    ));
                };
                let recv_key = format_ty_key(&inferred_tk);
                let mut callee = format!("{recv_key}::{method}");
                if !self.call_specs.contains_key(&callee) {
                    if matches!(inferred_tk, TyKey::Array(_)) {
                        if let Some(candidates) = self.generic_array_ext.get(method) {
                            for g in candidates {
                                if let Some((tmpl, tps)) = self.extension_match_info.get(g) {
                                    if match_type_keys(tmpl, &inferred_tk, tps) {
                                        callee = g.clone();
                                        break;
                                    }
                                }
                            }
                        }
                    } else if matches!(inferred_tk, TyKey::EnumApp { .. }) {
                        if let Some(candidates) = self.generic_enum_ext.get(method) {
                            for g in candidates {
                                if let Some((tmpl, tps)) = self.extension_match_info.get(g) {
                                    if match_type_keys(tmpl, &inferred_tk, tps) {
                                        callee = g.clone();
                                        break;
                                    }
                                }
                            }
                        }
                    } else if matches!(inferred_tk, TyKey::Ident(_)) {
                        if let Some(candidates) = self.generic_struct_ext.get(method) {
                            for g in candidates {
                                if let Some((tmpl, tps)) = self.extension_match_info.get(g) {
                                    if match_type_keys(tmpl, &inferred_tk, tps) {
                                        callee = g.clone();
                                        break;
                                    }
                                }
                            }
                        }
                    }
                }
                let mut lowered = Vec::with_capacity(arguments.len() + 1);
                lowered.push(CallArg::Positional((**receiver).clone()));
                lowered.extend(arguments.iter().cloned());
                // Convenience for unit structs: `None.foo()` -> `None::foo()` when static.
                let treat_as_static_unit = matches!(receiver.as_ref(), AstNode::Identifier { name, .. } if self.unit_struct_names.contains(name))
                    && self
                        .call_specs
                        .get(&callee)
                        .is_some_and(|params| params.is_empty());
                let resolved = if treat_as_static_unit {
                    self.resolve_call_arguments(&callee, arguments, *span)?
                } else {
                    self.resolve_call_arguments(&callee, &lowered, *span)?
                };
                for a in resolved {
                    self.compile_expr(&a)?;
                }
                let argc = self
                    .call_specs
                    .get(&callee)
                    .map(|p| p.len())
                    .unwrap_or(if treat_as_static_unit {
                        arguments.len()
                    } else {
                        arguments.len() + 1
                    });
                self.emit_function_invoke(&callee, argc, *span);
                Ok(())
            }
            AstNode::TypeMethodCall {
                type_name,
                method,
                arguments,
                span,
            } => {
                let mut callee = format!("{type_name}::{method}");
                if !self.call_specs.contains_key(&callee) {
                    if type_name.starts_with('[') && type_name.ends_with(']') {
                        if let Ok(te) = crate::parser::Parser::parse_type_expr_from_source(type_name) {
                            if let Some(tk) = type_expr_to_ty_key(&te) {
                                if let Some(candidates) = self.generic_array_ext.get(method) {
                                    for g in candidates {
                                        if let Some((tmpl, tps)) = self.extension_match_info.get(g) {
                                            if match_type_keys(tmpl, &tk, tps) {
                                                callee = g.clone();
                                                break;
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    } else if let Ok(te) = crate::parser::Parser::parse_type_expr_from_source(type_name) {
                        if let Some(tk) = type_expr_to_ty_key(&te) {
                            if matches!(tk, TyKey::EnumApp { .. }) {
                                if let Some(candidates) = self.generic_enum_ext.get(method) {
                                    for g in candidates {
                                        if let Some((tmpl, tps)) = self.extension_match_info.get(g) {
                                            if match_type_keys(tmpl, &tk, tps) {
                                                callee = g.clone();
                                                break;
                                            }
                                        }
                                    }
                                }
                            }
                            if matches!(tk, TyKey::Ident(_)) {
                                if let Some(candidates) = self.generic_struct_ext.get(method) {
                                    for g in candidates {
                                        if let Some((tmpl, tps)) = self.extension_match_info.get(g) {
                                            if match_type_keys(tmpl, &tk, tps) {
                                                callee = g.clone();
                                                break;
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                if self.call_specs.contains_key(&callee) {
                    let resolved = self.resolve_call_arguments(&callee, arguments, *span)?;
                    for a in resolved {
                        self.compile_expr(&a)?;
                    }
                    let argc = self
                        .call_specs
                        .get(&callee)
                        .map(|p| p.len())
                        .unwrap_or(arguments.len());
                    self.emit_function_invoke(&callee, argc, *span);
                    return Ok(());
                }
                let enum_target = if self.enum_names.contains(type_name) {
                    Some(type_name.clone())
                } else {
                    self.alias_enum_targets.get(type_name).cloned()
                };
                if let Some(enum_target) = enum_target {
                    let mut payloads: Vec<AstNode> = Vec::with_capacity(arguments.len());
                    for a in arguments {
                        match a {
                            CallArg::Positional(v) => payloads.push(v.clone()),
                            CallArg::Named { .. } => {
                                return Err(CompileError::new(
                                    "enum variant constructor does not support named arguments",
                                    Some(*span),
                                ));
                            }
                        }
                    }
                    for p in &payloads {
                        self.compile_expr(p)?;
                    }
                    self.emit(Instr::MakeEnumVariant {
                        enum_name: enum_target,
                        variant: method.clone(),
                        payload_count: payloads.len(),
                        span: *span,
                    });
                    return Ok(());
                }
                Err(CompileError::new(
                    format!("unknown static method `{type_name}::{method}`"),
                    Some(*span),
                ))
            }
            AstNode::Block { body, .. } => {
                self.compile_block(body)?;
                self.emit(Instr::PushUnit);
                Ok(())
            }
            AstNode::Match { scrutinee, arms, span } => {
                let end_lbl = self.labeler.new_label();

                // Evaluate scrutinee once.
                self.compile_expr(scrutinee.as_ref())?;
                let tmp = self.alloc_slot();
                self.emit(Instr::StoreLocal {
                    slot: tmp,
                    span: *span,
                });

                for arm in arms {
                    let next_arm_lbl = self.labeler.new_label();
                    // Try patterns in this arm (`pat1 | pat2`).
                    for (alt_idx, pat) in arm.patterns.iter().enumerate() {
                        let is_last_alt = alt_idx + 1 == arm.patterns.len();
                        let fail_lbl = if is_last_alt {
                            next_arm_lbl
                        } else {
                            self.labeler.new_label()
                        };

                        self.push_scope();
                        self.declare_pattern(pat)?;

                        match pat {
                            Pattern::EnumVariant {
                                enum_name,
                                variant,
                                payloads,
                                ..
                            } => {
                                // Load scrutinee and attempt enum variant match.
                                self.emit(Instr::LoadLocal {
                                    slot: tmp,
                                    span: *span,
                                });
                                self.emit(Instr::MatchEnumVariant {
                                    enum_name: enum_name.clone(),
                                    variant: variant.clone(),
                                    payload_count: payloads.len(),
                                    span: *span,
                                });
                                self.emit_jump(fail_lbl, |target| Instr::JumpIfFalse { target });

                                // Stack: payload1, payload2, ... (bool already popped by JumpIfFalse)
                                for p in payloads.iter().rev() {
                                    self.match_and_bind_pattern(p, fail_lbl)?;
                                }
                            }
                            _ => {
                                self.emit(Instr::LoadLocal {
                                    slot: tmp,
                                    span: *span,
                                });
                                self.match_and_bind_pattern(pat, fail_lbl)?;
                            }
                        }

                        // Optional guard.
                        if let Some(g) = arm.guard.as_ref() {
                            self.compile_expr(g.as_ref())?;
                            self.emit_jump(fail_lbl, |target| Instr::JumpIfFalse { target });
                        }

                        // Arm body value.
                        self.compile_expr(arm.body.as_ref())?;
                        self.pop_scope();
                        self.emit_jump(end_lbl, |target| Instr::Jump { target });
                        self.labeler.mark(fail_lbl, self.code.len());
                    }
                }

                // If all arms fail (e.g. due to guards), evaluate to `()`.
                self.emit(Instr::PushUnit);
                self.labeler.mark(end_lbl, self.code.len());
                Ok(())
            }
            _ => Err(CompileError::new(
                "unsupported expression node",
                None,
            )),
        }
    }

    fn compile_stmt(&mut self, stmt: &AstNode) -> Result<(), CompileError> {
        match stmt {
            AstNode::SingleLineComment(_) | AstNode::MultiLineComment(_) => Ok(()),
            AstNode::Block { body, .. } => self.compile_block(body),
            AstNode::Let {
                pattern,
                initializer,
                ..
            } => {
                // Introduce bindings first so initializer can be checked for definite assignment.
                self.declare_pattern(pattern)?;
                match initializer {
                    None => Ok(()),
                    Some(init) => {
                        self.compile_expr(init.as_ref())?;
                        self.store_pattern_value(pattern)?;
                        if let Pattern::Binding { name, .. } = pattern {
                            if let Some(k) = self.infer_receiver_ty_key(init.as_ref()) {
                                if let Some(scope) = self.local_type_keys.last_mut() {
                                    scope.insert(name.clone(), k);
                                }
                            }
                        }
                        Ok(())
                    }
                }
            }
            AstNode::Assign { pattern, value, .. } => {
                self.compile_expr(value.as_ref())?;
                self.store_pattern_value(pattern)?;
                Ok(())
            }
            AstNode::AssignExpr { lhs, rhs, span } => {
                // Semantics allow only identifier lvalues for now.
                self.compile_expr(rhs.as_ref())?;
                match lhs.as_ref() {
                    AstNode::Identifier { name, .. } => {
                        let Some(v) = self.resolve_var(name) else {
                            return Err(CompileError::new(
                                format!("unknown variable `{name}`"),
                                Some(*span),
                            ));
                        };
                        match v.kind {
                            VarSlotKind::Local => self.emit(Instr::StoreLocal {
                                slot: v.slot,
                                span: *span,
                            }),
                            VarSlotKind::Global => self.emit(Instr::StoreGlobal {
                                slot: v.slot,
                                span: *span,
                            }),
                        }
                        Ok(())
                    }
                    AstNode::ArrayIndex { .. } => {
                        let Some((root, segs)) = self.extract_array_index_chain(lhs.as_ref())
                        else {
                            return Err(CompileError::new(
                                "array assignment requires a chain rooted at a simple variable name",
                                Some(*span),
                            ));
                        };

                        let depth = segs.len();
                        if depth == 0 {
                            return Err(CompileError::new(
                                "internal error: empty array index chain",
                                Some(*span),
                            ));
                        }

                        // Save RHS in a temp slot so we can rebuild arrays bottom-up.
                        let rhs_slot = self.alloc_slot();
                        self.emit(Instr::StoreLocal {
                            slot: rhs_slot,
                            span: *span,
                        });

                        // array_slots[i] = the array being indexed at depth i
                        let mut array_slots: Vec<u32> = Vec::with_capacity(depth);
                        // index_slots[i] = computed index for depth i
                        let mut index_slots: Vec<u32> = Vec::with_capacity(depth);

                        // Load root array value into array_slots[0].
                        let root_tmp = self.alloc_slot();
                        array_slots.push(root_tmp);
                        match root.kind {
                            VarSlotKind::Local => self.emit(Instr::LoadLocal {
                                slot: root.slot,
                                span: *span,
                            }),
                            VarSlotKind::Global => self.emit(Instr::LoadGlobal {
                                slot: root.slot,
                                span: *span,
                            }),
                        }
                        self.emit(Instr::StoreLocal {
                            slot: root_tmp,
                            span: *span,
                        });

                        // Evaluate indices and compute intermediate arrays.
                        for i in 0..depth {
                            let idx_expr = segs[i].0;
                            let idx_span = segs[i].1;
                            let idx_slot = self.alloc_slot();
                            index_slots.push(idx_slot);

                            self.compile_expr(idx_expr)?;
                            self.emit(Instr::StoreLocal {
                                slot: idx_slot,
                                span: idx_span,
                            });

                            if i + 1 < depth {
                                // inner = array_slots[i][idx]
                                let span_for_get = idx_span;
                                self.emit(Instr::LoadLocal {
                                    slot: array_slots[i],
                                    span: *span,
                                });
                                self.emit(Instr::LoadLocal {
                                    slot: idx_slot,
                                    span: *span,
                                });
                                self.emit(Instr::GetArrayIndex {
                                    span: span_for_get,
                                });
                                let inner_slot = self.alloc_slot();
                                array_slots.push(inner_slot);
                                self.emit(Instr::StoreLocal {
                                    slot: inner_slot,
                                    span: *span,
                                });
                            }
                        }

                        let updated_slot = self.alloc_slot();

                        // updated = array_slots[depth - 1][index_slots[depth - 1]] = rhs
                        let last_i = depth - 1;
                        self.emit(Instr::LoadLocal {
                            slot: array_slots[last_i],
                            span: *span,
                        });
                        self.emit(Instr::LoadLocal {
                            slot: index_slots[last_i],
                            span: *span,
                        });
                        self.emit(Instr::LoadLocal {
                            slot: rhs_slot,
                            span: *span,
                        });
                        self.emit(Instr::ArrayStore {
                            span: segs[last_i].1,
                        });
                        self.emit(Instr::StoreLocal {
                            slot: updated_slot,
                            span: *span,
                        });

                        // Propagate updated inner arrays back outward.
                        for i in (0..depth - 1).rev() {
                            self.emit(Instr::LoadLocal {
                                slot: array_slots[i],
                                span: *span,
                            });
                            self.emit(Instr::LoadLocal {
                                slot: index_slots[i],
                                span: *span,
                            });
                            self.emit(Instr::LoadLocal {
                                slot: updated_slot,
                                span: *span,
                            });
                            self.emit(Instr::ArrayStore { span: segs[i].1 });
                            self.emit(Instr::StoreLocal {
                                slot: updated_slot,
                                span: *span,
                            });
                        }

                        // Store updated root array back.
                        self.emit(Instr::LoadLocal {
                            slot: updated_slot,
                            span: *span,
                        });
                        match root.kind {
                            VarSlotKind::Local => self.emit(Instr::StoreLocal {
                                slot: root.slot,
                                span: *span,
                            }),
                            VarSlotKind::Global => self.emit(Instr::StoreGlobal {
                                slot: root.slot,
                                span: *span,
                            }),
                        }

                        Ok(())
                    }
                    AstNode::FieldAccess { .. } => {
                        let Some((root, segs)) =
                            self.extract_field_access_chain(lhs.as_ref())
                        else {
                            return Err(CompileError::new(
                                "field assignment requires a chain rooted at a simple variable name",
                                Some(*span),
                            ));
                        };

                        if segs.is_empty() {
                            return Err(CompileError::new(
                                "internal error: empty field access chain",
                                Some(*span),
                            ));
                        }

                        // Save RHS in a temp slot.
                        let rhs_slot = self.alloc_slot();
                        self.emit(Instr::StoreLocal {
                            slot: rhs_slot,
                            span: *span,
                        });

                        // Load root struct instance.
                        match root.kind {
                            VarSlotKind::Local => self.emit(Instr::LoadLocal {
                                slot: root.slot,
                                span: *span,
                            }),
                            VarSlotKind::Global => self.emit(Instr::LoadGlobal {
                                slot: root.slot,
                                span: *span,
                            }),
                        }

                        // Navigate through intermediate fields.
                        for (field, fspan) in &segs[..segs.len() - 1] {
                            self.emit(Instr::GetStructField {
                                field: field.to_string(),
                                span: *fspan,
                            });
                        }

                        let (last_field, last_span) = segs[segs.len() - 1];
                        // Push RHS and store into the last field.
                        self.emit(Instr::LoadLocal {
                            slot: rhs_slot,
                            span: *span,
                        });
                        self.emit(Instr::StructFieldStore {
                            field: last_field.to_string(),
                            span: last_span,
                        });
                        Ok(())
                    }
                    _ => Err(CompileError::new("unsupported assignment target", Some(*span))),
                }
            }
            AstNode::CompoundAssign { lhs, op, rhs, span } => {
                let lhs_slot = match lhs.as_ref() {
                    AstNode::Identifier { name, .. } => self.resolve_var(name),
                    _ => None,
                };
                let Some(lhs_slot) = lhs_slot else {
                    return Err(CompileError::new("compound assignment target not found", Some(*span)));
                };
                // Stack: [lhs, rhs] then apply op
                match lhs_slot.kind {
                    VarSlotKind::Local => self.emit(Instr::LoadLocal { slot: lhs_slot.slot, span: *span }),
                    VarSlotKind::Global => self.emit(Instr::LoadGlobal { slot: lhs_slot.slot, span: *span }),
                }
                self.compile_expr(rhs.as_ref())?;

                let bin_op = match op {
                    crate::ast::CompoundOp::Add => BinaryOp::Add,
                    crate::ast::CompoundOp::Sub => BinaryOp::Sub,
                    crate::ast::CompoundOp::Mul => BinaryOp::Mul,
                    crate::ast::CompoundOp::Div => BinaryOp::Div,
                    crate::ast::CompoundOp::Mod => BinaryOp::Mod,
                    crate::ast::CompoundOp::BitAnd => BinaryOp::BitAnd,
                    crate::ast::CompoundOp::BitXor => BinaryOp::BitXor,
                    crate::ast::CompoundOp::BitOr => BinaryOp::BitOr,
                    crate::ast::CompoundOp::ShiftLeft => BinaryOp::ShiftLeft,
                    crate::ast::CompoundOp::ShiftRight => BinaryOp::ShiftRight,
                };

                self.emit(Instr::BinOp { op: bin_op, span: *span });
                // Store result back
                match lhs_slot.kind {
                    VarSlotKind::Local => self.emit(Instr::StoreLocal { slot: lhs_slot.slot, span: *span }),
                    VarSlotKind::Global => self.emit(Instr::StoreGlobal { slot: lhs_slot.slot, span: *span }),
                }
                Ok(())
            }
            AstNode::Call { .. }
            | AstNode::Invoke { .. }
            | AstNode::Await { .. }
            | AstNode::MethodCall { .. }
            | AstNode::TypeMethodCall { .. } => {
                // Call as a statement: discard return value.
                self.compile_expr(stmt)?;
                self.emit(Instr::Pop);
                Ok(())
            }
            AstNode::Match { .. } => {
                // Match as a statement: discard resulting value.
                self.compile_expr(stmt)?;
                self.emit(Instr::Pop);
                Ok(())
            }
            AstNode::Return { value, span } => {
                match value {
                    Some(v) => self.compile_expr(v.as_ref())?,
                    None => self.emit(Instr::PushUnit),
                }
                self.emit(Instr::Return { span: *span });
                Ok(())
            }
            AstNode::If {
                condition,
                then_body,
                else_body,
                ..
            } => {
                let else_lbl = self.labeler.new_label();
                let end_lbl = self.labeler.new_label();

                self.compile_expr(condition.as_ref())?;
                if else_body.is_some() {
                    self.emit_jump(else_lbl, |target| Instr::JumpIfFalse { target });
                } else {
                    self.emit_jump(end_lbl, |target| Instr::JumpIfFalse { target });
                }

                for s in then_body {
                    self.compile_stmt(s)?;
                }

                if let Some(_) = else_body {
                    self.emit_jump(end_lbl, |target| Instr::Jump { target });
                    self.labeler.mark(else_lbl, self.code.len());
                    for s in else_body.as_ref().unwrap() {
                        self.compile_stmt(s)?;
                    }
                } else {
                    self.labeler.mark(end_lbl, self.code.len());
                }
                self.labeler.mark(end_lbl, self.code.len());
                Ok(())
            }
            AstNode::IfLet {
                pattern,
                value,
                then_body,
                else_body,
                span,
            } => {
                let else_lbl = self.labeler.new_label();
                let end_lbl = self.labeler.new_label();

                self.compile_expr(value.as_ref())?;

                let has_else = else_body.is_some();

                match pattern {
                    Pattern::EnumVariant {
                        enum_name,
                        variant,
                        payloads,
                        ..
                    } => {
                        // Runtime tag check. On success the stack becomes:
                        //   [..., payload1, ..., payloadN, true]
                        // On failure the stack becomes: [..., false]
                        self.emit(Instr::MatchEnumVariant {
                            enum_name: enum_name.clone(),
                            variant: variant.clone(),
                            payload_count: payloads.len(),
                            span: *span,
                        });
                        if has_else {
                            self.emit_jump(else_lbl, |target| {
                                Instr::JumpIfFalse { target }
                            });
                        } else {
                            self.emit_jump(end_lbl, |target| {
                                Instr::JumpIfFalse { target }
                            });
                        }

                        // Then branch: bind pattern variables from payloads.
                        self.push_scope();
                        self.declare_pattern(pattern)?;
                        // Payload values are already on the stack (the enum value was consumed by
                        // `MatchEnumVariant`, and the bool was consumed by `JumpIfFalse`).
                        for p in payloads.iter().rev() {
                            self.store_pattern_value(p)?;
                        }
                        for s in then_body {
                            self.compile_stmt(s)?;
                        }
                        self.pop_scope();

                        if has_else {
                            self.emit_jump(end_lbl, |target| Instr::Jump { target });
                            self.labeler.mark(else_lbl, self.code.len());
                            self.compile_block(else_body.as_ref().unwrap())?;
                            self.labeler.mark(end_lbl, self.code.len());
                        } else {
                            self.labeler.mark(end_lbl, self.code.len());
                        }
                    }
                    _ => {
                        // Non-enum patterns can still fail at runtime (e.g. typed unit-struct patterns).
                        self.push_scope();
                        self.declare_pattern(pattern)?;
                        if has_else {
                            self.match_and_bind_pattern(pattern, else_lbl)?;
                        } else {
                            self.match_and_bind_pattern(pattern, end_lbl)?;
                        }
                        for s in then_body {
                            self.compile_stmt(s)?;
                        }
                        self.pop_scope();

                        if has_else {
                            self.emit_jump(end_lbl, |target| Instr::Jump { target });
                            self.labeler.mark(else_lbl, self.code.len());
                            self.compile_block(else_body.as_ref().unwrap())?;
                        }
                        self.labeler.mark(end_lbl, self.code.len());
                    }
                }

                Ok(())
            }
            AstNode::While {
                condition,
                body,
                ..
            } => {
                let start_lbl = self.labeler.new_label();
                let end_lbl = self.labeler.new_label();

                self.labeler.mark(start_lbl, self.code.len());

                self.compile_expr(condition.as_ref())?;
                self.emit_jump(end_lbl, |target| Instr::JumpIfFalse { target });

                self.loop_stack.push(LoopLabels {
                    break_target: end_lbl,
                    continue_target: start_lbl,
                });

                for s in body {
                    self.compile_stmt(s)?;
                }

                self.loop_stack.pop();
                self.emit_jump(start_lbl, |target| Instr::Jump { target });
                self.labeler.mark(end_lbl, self.code.len());

                Ok(())
            }
            AstNode::Break { .. } => {
                let Some(top) = self.loop_stack.last().copied() else {
                    return Err(CompileError::new(
                        "`break` outside loop",
                        None,
                    ));
                };
                self.emit_jump(top.break_target, |target| Instr::Jump { target });
                Ok(())
            }
            AstNode::Continue { .. } => {
                let Some(top) = self.loop_stack.last().copied() else {
                    return Err(CompileError::new(
                        "`continue` outside loop",
                        None,
                    ));
                };
                self.emit_jump(top.continue_target, |target| Instr::Jump { target });
                Ok(())
            }
            other => Err(CompileError::new(
                format!("unsupported statement node: {other:?}"),
                None,
            )),
        }
    }
}

/// Compile a semantically valid `AstNode::Program` into stack bytecode using the default builtin registry.
pub fn compile_program(ast: &AstNode) -> Result<ProgramBytecode, CompileError> {
    let mono = crate::monomorphize::monomorphize_program(ast);
    compile_program_with_builtins(&mono, crate::builtins::default_registry_ref())
}

/// Like [`compile_program`], but lets the caller provide the builtin registry.
pub fn compile_program_with_builtins(
    ast: &AstNode,
    builtins: &crate::builtins::BuiltinRegistry,
) -> Result<ProgramBytecode, CompileError> {
    let AstNode::Program(items) = ast else {
        return Err(CompileError::new("bytecode generator expects a program root", None));
    };

    let mut globals: HashMap<String, u32> = HashMap::new();
    let mut call_specs: HashMap<String, Vec<CallParamSpec>> = HashMap::new();
    let mut call_returns: HashMap<String, Option<TypeExpr>> = HashMap::new();
    let mut enum_names: HashSet<String> = HashSet::new();
    let mut alias_enum_targets: HashMap<String, String> = HashMap::new();
    let mut unit_struct_names: HashSet<String> = HashSet::new();
    let mut init_order: Vec<&AstNode> = Vec::new();
    let mut generic_array_ext: HashMap<String, Vec<String>> = HashMap::new();
    let mut generic_enum_ext: HashMap<String, Vec<String>> = HashMap::new();
    let mut generic_struct_ext: HashMap<String, Vec<String>> = HashMap::new();
    let mut extension_match_info: HashMap<String, (TyKey, Vec<String>)> = HashMap::new();
    let mut async_callees: HashSet<String> = HashSet::new();
    let mut internal_async_vm_callee: HashMap<String, String> = HashMap::new();
    let mut struct_names: HashSet<String> = HashSet::new();

    // Validate internal built-ins and collect globals layout.
    for item in items {
        match item {
            AstNode::InternalFunction {
                name,
                type_params,
                params,
                return_type,
                name_span,
                is_async,
                ..
            } => {
                if *is_async {
                    let canon = builtins
                        .resolve_internal_async_callee(params, return_type, *name_span)
                        .map_err(|e| CompileError::new(e.message, e.span))?;
                    internal_async_vm_callee.insert(name.clone(), canon.to_string());
                    async_callees.insert(name.clone());
                } else {
                    validate_internal_func_decl(
                        builtins,
                        name,
                        params,
                        return_type,
                        *name_span,
                        false,
                    )?;
                }
                call_specs.insert(
                    name.clone(),
                    params
                        .iter()
                        .map(|p| CallParamSpec {
                            name: p.name.clone(),
                            is_params: p.is_params,
                            default_value: None,
                        })
                        .collect(),
                );
                call_returns.insert(name.clone(), return_type.clone());

                // Generic extension dispatch metadata:
                // internal functions can also be declared as extension receivers, and method-call
                // lowering needs those candidates for non-monomorphized call sites.
                if let Some((_, method_name)) = name.split_once("::") {
                    if let Some(first_param) = params.first() {
                        if first_param.name == "self" {
                            let recv_ty = &first_param.ty;
                            let is_generic_recv = match recv_ty {
                                TypeExpr::EnumApp { args, .. } => {
                                    args.iter().all(|a| matches!(a, TypeExpr::TypeParam(_)))
                                }
                                _ => false,
                            };
                            if is_generic_recv {
                                if let Some(tk) = type_expr_to_ty_key(recv_ty) {
                                    extension_match_info.insert(
                                        name.clone(),
                                        (tk, GenericParam::names(type_params)),
                                    );
                                }
                                let entry =
                                    generic_enum_ext.entry(method_name.to_string()).or_default();
                                if !entry.contains(name) {
                                    entry.insert(0, name.clone());
                                }
                            }
                            // If the receiver base is a concrete struct name, also register for
                            // the `TyKey::Ident` dispatch path.
                            if let TypeExpr::EnumApp { name: recv_base, .. } = recv_ty {
                                if struct_names.contains(recv_base) {
                                    let entry = generic_struct_ext
                                        .entry(method_name.to_string())
                                        .or_default();
                                    if !entry.contains(name) {
                                        entry.push(name.clone());
                                    }
                                }
                            }
                        }
                    }
                }
            }
            AstNode::Function {
                name,
                type_params,
                params,
                return_type,
                extension_receiver,
                is_async,
                ..
            } => {
                if *is_async {
                    async_callees.insert(name.clone());
                }
                if let Some(ext) = extension_receiver {
                    if let TypeExpr::Array(inner) = &ext.ty {
                        let is_generic = match inner.as_ref() {
                            TypeExpr::TypeParam(_) => true,
                            TypeExpr::EnumApp { args, .. } => {
                                args.iter().all(|a| matches!(a, TypeExpr::TypeParam(_)))
                            }
                            _ => false,
                        };
                        if is_generic {
                            if let Some(tk) = type_expr_to_ty_key(&ext.ty) {
                                extension_match_info.insert(
                                    name.clone(),
                                    (tk, GenericParam::names(type_params)),
                                );
                            }
                            let entry = generic_array_ext
                                .entry(ext.method_name.clone())
                                .or_default();
                            if !entry.contains(name) {
                                if matches!(inner.as_ref(), TypeExpr::EnumApp { .. }) {
                                    entry.insert(0, name.clone());
                                } else {
                                    entry.push(name.clone());
                                }
                            }
                        }
                    } else if let TypeExpr::EnumApp { args, .. } = &ext.ty {
                        let is_generic = args.iter().all(|a| matches!(a, TypeExpr::TypeParam(_)));
                        if is_generic {
                            if let Some(tk) = type_expr_to_ty_key(&ext.ty) {
                                extension_match_info.insert(
                                    name.clone(),
                                    (tk, GenericParam::names(type_params)),
                                );
                            }
                            let entry = generic_enum_ext.entry(ext.method_name.clone()).or_default();
                            if !entry.contains(name) {
                                entry.insert(0, name.clone());
                            }
                            if let TypeExpr::EnumApp { name: recv_name, .. } = &ext.ty {
                                if struct_names.contains(recv_name) {
                                    let entry = generic_struct_ext
                                        .entry(ext.method_name.clone())
                                        .or_default();
                                    if !entry.contains(name) {
                                        entry.push(name.clone());
                                    }
                                }
                            }
                        }
                    } else if let TypeExpr::Named(n) = &ext.ty {
                        if type_params.iter().any(|tp| tp.name == *n) {
                            if let Some(tk) = type_expr_to_ty_key(&ext.ty) {
                                extension_match_info.insert(
                                    name.clone(),
                                    (tk, GenericParam::names(type_params)),
                                );
                            }
                            let entry = generic_struct_ext
                                .entry(ext.method_name.clone())
                                .or_default();
                            if !entry.contains(name) {
                                entry.push(name.clone());
                            }
                        }
                    }
                }
                call_specs.insert(
                    name.clone(),
                    params
                        .iter()
                        .map(|p| CallParamSpec {
                            name: p.name.clone(),
                            is_params: p.is_params,
                            default_value: p.default_value.as_deref().cloned(),
                        })
                        .collect(),
                );
                call_returns.insert(name.clone(), return_type.clone());
            }
            AstNode::StructDef { name, is_unit, .. } => {
                struct_names.insert(name.clone());
                if *is_unit {
                    unit_struct_names.insert(name.clone());
                }
            }
            AstNode::EnumDef { name, .. } => {
                enum_names.insert(name.clone());
            }
            AstNode::TypeAlias { name, target, .. } => {
                match target {
                    TypeExpr::EnumApp { name: enum_name, .. } | TypeExpr::Named(enum_name) => {
                        alias_enum_targets.insert(name.clone(), enum_name.clone());
                    }
                    _ => {}
                }
            }
            AstNode::Let { pattern, initializer, .. } => {
                // Root patterns are limited by semantic checker (no tuple patterns).
                if initializer.is_none() {
                    // Semantic checker should have caught this earlier.
                    return Err(CompileError::new("global `let` requires initializer", None));
                }
                if let Pattern::Binding { name, .. } = pattern {
                    if !globals.contains_key(name) {
                        let slot = globals.len() as u32;
                        globals.insert(name.clone(), slot);
                    }
                    init_order.push(item);
                } else if matches!(pattern, Pattern::Wildcard { .. }) {
                    init_order.push(item);
                } else {
                    return Err(CompileError::new(
                        "tuple patterns are not allowed at the top level",
                        None,
                    ));
                }
            }
            _ => {}
        }
    }

    // Ensure we have a main function.
    let entry_main = {
        let mut found = None;
        for item in items {
            if let AstNode::Function { name, .. } = item {
                if name == "main" {
                    found = Some(name.clone());
                }
            }
        }
        found.ok_or_else(|| {
            CompileError::new(
                "program must define `func main() { ... }` or `async func main() { ... }`",
                None,
            )
        })?
    };

    // Compile globals init instructions (executed before main).
    let mut init_gen = FnGen::new(
        &globals,
        &call_specs,
        &call_returns,
        &enum_names,
        &alias_enum_targets,
        &unit_struct_names,
        &generic_array_ext,
        &generic_enum_ext,
        &generic_struct_ext,
        &extension_match_info,
        &async_callees,
        &internal_async_vm_callee,
    );
    init_gen.current_fn_name = "__init__".to_string();
    // Empty scope stack is fine: globals are resolved via `globals` map.
    for item in &init_order {
        if let AstNode::Let {
            pattern,
            initializer,
            ..
        } = item
        {
            let init = initializer.as_deref().unwrap();
            init_gen.compile_expr(init)?;
            match pattern {
                Pattern::Wildcard { .. } => {
                    init_gen.emit(Instr::Pop);
                }
                Pattern::Binding { name, .. } => {
                    let slot = *globals
                        .get(name)
                        .ok_or_else(|| CompileError::new(format!("unknown global `{name}`"), None))?;
                    init_gen.emit(Instr::StoreGlobal {
                        slot,
                        span: Span::new(1, 1, 1),
                    });
                }
                Pattern::IntLiteral { .. }
                | Pattern::StringLiteral { .. }
                | Pattern::BoolLiteral { .. } => {
                    return Err(CompileError::new(
                        "literal patterns are not allowed at the top level",
                        None,
                    ));
                }
                Pattern::Tuple { .. } => {
                    return Err(CompileError::new(
                        "tuple patterns are not allowed at the top level",
                        None,
                    ));
                }
                Pattern::Array { .. } => {
                    return Err(CompileError::new(
                        "array patterns are not allowed at the top level",
                        None,
                    ));
                }
                Pattern::Struct { .. } => {
                    return Err(CompileError::new(
                        "struct patterns are not allowed at the top level",
                        None,
                    ));
                }
                Pattern::EnumVariant { .. } => {
                    return Err(CompileError::new(
                        "enum patterns are not allowed at the top level",
                        None,
                    ));
                }
            }
        }
    }
    init_gen.labeler.patch_all(&mut init_gen.code);

    // Compile user functions.
    let mut functions: Vec<FunctionBytecode> = Vec::new();
    for item in items {
        if let AstNode::Function {
            name,
            params,
            body,
            name_span,
            closing_span,
            ..
        } = item
        {
            // Internal functions are already validated; do not compile them as user functions.
            let mut fn_gen = FnGen::new(
                &globals,
                &call_specs,
                &call_returns,
                &enum_names,
                &alias_enum_targets,
                &unit_struct_names,
                &generic_array_ext,
                &generic_enum_ext,
                &generic_struct_ext,
                &extension_match_info,
                &async_callees,
                &internal_async_vm_callee,
            );
            fn_gen.current_fn_name = name.clone();
            fn_gen.push_scope();
            let mut real_param_slots: Vec<Option<u32>> = Vec::with_capacity(params.len());
            for p in params {
                if p.is_wildcard {
                    real_param_slots.push(None);
                } else {
                    let slot = fn_gen.alloc_slot();
                    fn_gen
                        .scopes
                        .last_mut()
                        .expect("scope stack")
                        .insert(p.name.clone(), slot);
                    real_param_slots.push(Some(slot));
                }
            }

            // Compile statements into instruction stream.
            for s in body {
                fn_gen.compile_stmt(s)?;
            }

            // Fallthrough -> return Unit.
            fn_gen.emit(Instr::PushUnit);
            fn_gen.emit(Instr::Return { span: *closing_span });
            fn_gen.labeler.patch_all(&mut fn_gen.code);

            functions.push(FunctionBytecode {
                name: name.clone(),
                code: fn_gen.code,
                local_count: fn_gen.next_local_slot,
                param_count: params.len(),
                param_slots: real_param_slots,
                span: *name_span,
            });
            functions.extend(fn_gen.generated_functions);
        }
    }

    let mut function_map = HashMap::new();
    for (idx, f) in functions.iter().enumerate() {
        function_map.insert(f.name.clone(), idx);
    }

    Ok(ProgramBytecode {
        entry_main,
        init_code: init_gen.code,
        functions,
        function_map,
        globals_count: globals.len() as u32,
    })
}

