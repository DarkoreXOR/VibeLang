//! Semantic analysis: symbols, types, tuples, patterns, definite assignment.

use std::collections::HashMap;

use crate::ast::{
    AstNode, BinaryOp, CallArg, CompoundOp, FunctionTypeParam, GenericParam, Param, Pattern,
    PatternElem, TypeExpr, UnaryOp,
};
use crate::error::{SemanticError, Span};
#[path = "semantic/infer.rs"]
mod infer;
#[path = "semantic/typecheck.rs"]
mod typecheck;

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Ty {
    Int,
    Float,
    String,
    Bool,
    Unit,
    Tuple(Vec<Ty>),
    Array(Box<Ty>),
    Any,
    /// Generic type parameter (e.g. `T` in `func f<T>(x: T)`).
    TypeParam(String),
    /// User-defined nominal struct type.
    Struct(String),
    /// User-defined enum type: `Option<Int>` / `Result<T, E>`.
    Enum {
        name: String,
        args: Vec<Ty>,
    },
    Function {
        params: Vec<Ty>,
        param_names: Vec<Option<String>>,
        param_has_default: Vec<bool>,
        ret: Box<Ty>,
    },
    InferVar(u32),
    /// Internal/async task carrier: `Task<T>`.
    Task(Box<Ty>),
}

#[derive(Clone, Debug)]
struct FuncSig {
    type_params: Vec<GenericParam>,
    params: Vec<Ty>,
    params_ast: Vec<Param>,
    ret: Option<Ty>,
    /// Distinguishes `async func` / `internal async func` for callers (see bytecode generator).
    #[allow(dead_code)]
    is_async: bool,
    /// For extension methods, the receiver type template (`Task<T>`) used to infer type args from
    /// `Type::method` calls. `None` for ordinary functions (first parameter is used instead).
    extension_receiver_ty: Option<Ty>,
}

#[derive(Debug)]
struct GlobalDecl {
    pattern: Pattern,
    type_annotation: Option<TypeExpr>,
    initializer: Option<Box<AstNode>>,
    span: Span,
}

#[derive(Clone, Debug)]
struct StructDef {
    type_params: Vec<GenericParam>,
    fields: HashMap<String, Ty>,
    is_unit: bool,
}

#[derive(Clone, Debug)]
struct EnumDef {
    type_params: Vec<GenericParam>,
    /// Variant name -> payload type templates (0+ payload types).
    variants: HashMap<String, Vec<TypeExpr>>,
}

#[derive(Clone, Debug)]
struct AliasDef {
    type_params: Vec<GenericParam>,
    target: TypeExpr,
}

#[derive(Clone)]
enum NameRes {
    Local(usize),
    Global(Ty),
}

struct BodyCtx<'a> {
    registry: &'a HashMap<String, FuncSig>,
    structs: &'a HashMap<String, StructDef>,
    enums: &'a HashMap<String, EnumDef>,
    aliases: &'a HashMap<String, AliasDef>,
    globals: &'a HashMap<String, Ty>,
    /// `method_name` -> callee keys for generic array extensions, e.g. `len` -> `["[T]::len", ...]`.
    generic_array_ext: &'a HashMap<String, Vec<String>>,
    /// `method_name` -> callee keys for generic enum extensions, e.g. `is_ok` -> `["Result<T, E>::is_ok"]`.
    generic_enum_ext: &'a HashMap<String, Vec<String>>,
    /// `method_name` -> callee keys for generic struct-receiver extensions, e.g. `m` -> `["T::m"]`.
    generic_struct_ext: &'a HashMap<String, Vec<String>>,
    scopes: Vec<HashMap<String, usize>>,
    bindings_ty: Vec<Option<Ty>>,
    assigned: Vec<bool>,
    loop_depth: usize,
    /// True while type-checking an `async func` / `internal async func` body.
    in_async: bool,
    infer_ctx: infer::InferCtx,
}

impl<'a> BodyCtx<'a> {
    fn new(
        registry: &'a HashMap<String, FuncSig>,
        structs: &'a HashMap<String, StructDef>,
        enums: &'a HashMap<String, EnumDef>,
        aliases: &'a HashMap<String, AliasDef>,
        globals: &'a HashMap<String, Ty>,
        generic_array_ext: &'a HashMap<String, Vec<String>>,
        generic_enum_ext: &'a HashMap<String, Vec<String>>,
        generic_struct_ext: &'a HashMap<String, Vec<String>>,
    ) -> Self {
        Self {
            registry,
            structs,
            enums,
            aliases,
            globals,
            generic_array_ext,
            generic_enum_ext,
            generic_struct_ext,
            scopes: Vec::new(),
            bindings_ty: Vec::new(),
            assigned: Vec::new(),
            loop_depth: 0,
            in_async: false,
            infer_ctx: infer::InferCtx::default(),
        }
    }

    fn push_scope(&mut self) {
        self.scopes.push(HashMap::new());
    }

    fn pop_scope(&mut self) {
        self.scopes.pop();
    }

    fn lookup(&self, name: &str) -> Option<NameRes> {
        for map in self.scopes.iter().rev() {
            if let Some(&id) = map.get(name) {
                return Some(NameRes::Local(id));
            }
        }
        self.globals
            .get(name)
            .map(|ty| NameRes::Global(ty.clone()))
    }

    fn declare_param(&mut self, p: &Param, ty: Ty, errors: &mut Vec<SemanticError>) {
        if p.is_wildcard {
            return;
        }
        let map = self.scopes.last_mut().expect("scope stack");
        if map.contains_key(&p.name) {
            errors.push(SemanticError::new(
                format!("duplicate parameter `{}`", p.name),
                p.name_span,
            ));
            return;
        }
        let id = self.bindings_ty.len();
        self.bindings_ty.push(Some(ty));
        self.assigned.push(true);
        map.insert(p.name.clone(), id);
    }

    fn declare_binding(
        &mut self,
        name: &str,
        name_span: Span,
        ann_ty: Option<Ty>,
        init: Option<&AstNode>,
        errors: &mut Vec<SemanticError>,
    ) {
        let expected_for_check = ann_ty.clone();
        let map = self.scopes.last_mut().expect("scope stack");
        if map.contains_key(name) {
            errors.push(SemanticError::new(
                format!("redefinition of `{name}` in the same scope"),
                name_span,
            ));
            return;
        }
        let id = self.bindings_ty.len();
        let stored_ty = ann_ty.or_else(|| Some(self.infer_ctx.fresh_var()));
        self.bindings_ty.push(stored_ty);
        self.assigned.push(false);
        map.insert(name.to_string(), id);

        if let Some(init_expr) = init {
            let got = if let Some(ref expected_ty) = expected_for_check {
                check_expr_expected(init_expr, self, errors, Some(expected_ty))
            } else {
                check_expr(init_expr, self, errors)
            };
            if let Some(got) = got {
                if unify_binding(self, id, got, expr_span(init_expr), errors) {
                    self.assigned[id] = true;
                }
            }
        }
    }

    /// Leaf binding from tuple destructure: known type, definitely assigned.
    fn declare_binding_destructure(
        &mut self,
        name: &str,
        name_span: Span,
        ty: Ty,
        errors: &mut Vec<SemanticError>,
    ) {
        let map = self.scopes.last_mut().expect("scope stack");
        if map.contains_key(name) {
            errors.push(SemanticError::new(
                format!("redefinition of `{name}` in the same scope"),
                name_span,
            ));
            return;
        }
        let id = self.bindings_ty.len();
        self.bindings_ty.push(Some(ty));
        self.assigned.push(true);
        map.insert(name.to_string(), id);
    }
}

/// Like [`check_expr`], but can use an expected type to contextualize empty array literals.
/// This is needed for array expressions where all elements are `[]` but the surrounding
/// `let` annotation provides the element type.
fn check_expr_expected(
    expr: &AstNode,
    ctx: &mut BodyCtx<'_>,
    errors: &mut Vec<SemanticError>,
    expected: Option<&Ty>,
) -> Option<Ty> {
    if let AstNode::Lambda { params, body, span: _ } = expr {
        if let Some(Ty::Function {
            params: exp_params,
            param_names,
            param_has_default,
            ret,
        }) = expected
        {
            if exp_params.len() != params.len() {
                errors.push(SemanticError::new(
                    format!(
                        "lambda parameter count mismatch: expected {}, found {}",
                        exp_params.len(),
                        params.len()
                    ),
                    expr_span(expr),
                ));
                return Some(Ty::Function {
                    params: vec![Ty::Any; params.len()],
                    param_names: vec![None; params.len()],
                    param_has_default: vec![false; params.len()],
                    ret: Box::new(Ty::Any),
                });
            }
            ctx.push_scope();
            for (p, pty) in params.iter().zip(exp_params.iter()) {
                ctx.declare_binding_destructure(&p.name, p.name_span, pty.clone(), errors);
            }
            match body.as_ref() {
                crate::ast::LambdaBody::Expr(e) => {
                    let got = check_expr_expected(e, ctx, errors, Some(ret.as_ref()));
                    if let Some(got) = got {
                        if ret.as_ref() != &Ty::Any {
                            let _ = infer::unify_types(
                                ret.as_ref(),
                                &got,
                                &mut ctx.infer_ctx,
                                errors,
                                expr_span(e),
                            );
                        }
                    }
                }
                crate::ast::LambdaBody::Block(items) => {
                    let mut reachable = true;
                    for s in items {
                        match check_statement(s, Some(ret.as_ref()), ctx, reachable, errors) {
                            StmtFlow::Next(r) => reachable = r,
                            _ => {}
                        }
                    }
                }
            }
            ctx.pop_scope();
            return Some(Ty::Function {
                params: exp_params.clone(),
                param_names: param_names.clone(),
                param_has_default: param_has_default.clone(),
                ret: ret.clone(),
            });
        }
    }
    if let AstNode::ArrayLiteral { elements, .. } = expr {
        if let Some(expected_ty) = expected {
            let Ty::Array(expected_elem_box) = expected_ty else {
                return check_expr(expr, ctx, errors);
            };
            let expected_elem_ty: &Ty = expected_elem_box.as_ref();
            for e in elements {
                let got = check_expr_expected(e, ctx, errors, Some(expected_elem_ty))?;
                let got_resolved = infer::resolve_ty(&got, &ctx.infer_ctx);
                let exp_resolved = infer::resolve_ty(expected_elem_ty, &ctx.infer_ctx);
                if got_resolved != exp_resolved {
                    errors.push(SemanticError::new(
                        format!(
                            "array literal element type mismatch: expected `{}`, found `{}`",
                            ty_name(&exp_resolved),
                            ty_name(&got_resolved)
                        ),
                        expr_span(e),
                    ));
                    return None;
                }
            }
            return Some(expected_ty.clone());
        }
    }

    // Dict literal contextual typing:
    // `let d: Dict<K, V> = { "k": v, ... }` uses the annotation to validate each entry.
    if let AstNode::DictLiteral { entries, .. } = expr {
        if let Some(Ty::Struct(struct_name)) = expected {
            if struct_base_name(struct_name) == "Dict" {
                let span = expr_span(expr);

                let te = crate::parser::Parser::parse_type_expr_from_source(struct_name);
                let Ok(te) = te else {
                    return check_expr(expr, ctx, errors);
                };

                let (key_te, value_te) = match te {
                    TypeExpr::EnumApp { mut args, .. } if args.len() == 2 => {
                        // Clone out args so we don't borrow across the match.
                        let v0 = args.remove(0);
                        let v1 = args.remove(0);
                        (v0, v1)
                    }
                    _ => {
                        errors.push(SemanticError::new(
                            "dict type annotation must be `Dict<K, V>`".to_string(),
                            span,
                        ));
                        return None;
                    }
                };

                let Some(expected_key_ty) = ty_from_type_expr(
                    &key_te,
                    span,
                    errors,
                    &[],
                    &ctx.structs,
                    &ctx.enums,
                    ctx.aliases,
                ) else {
                    return None;
                };
                let Some(expected_val_ty) = ty_from_type_expr(
                    &value_te,
                    span,
                    errors,
                    &[],
                    &ctx.structs,
                    &ctx.enums,
                    ctx.aliases,
                ) else {
                    return None;
                };

                for (k, v) in entries {
                    let _ = check_expr_expected(k, ctx, errors, Some(&expected_key_ty));
                    let _ = check_expr_expected(v, ctx, errors, Some(&expected_val_ty));
                }

                return expected.cloned();
            }
        }
    }

    // Enum constructor contextual typing (needed for `Option::None`-like variants).
    if let AstNode::EnumVariantCtor {
        enum_name,
        type_args,
        variant,
        payloads,
        ..
    } = expr
    {
        if let Some(Ty::Enum {
            name: exp_enum_name,
            args: exp_args,
        }) = expected
        {
            if exp_enum_name == enum_name {
                let Some(def) = ctx.enums.get(enum_name) else {
                    return None;
                };
                let Some(payload_templates) = def.variants.get(variant) else {
                    errors.push(SemanticError::new(
                        format!("unknown variant `{}` for enum `{}`", variant, enum_name),
                        Span::new(1, 1, 1),
                    ));
                    return None;
                };

                if payload_templates.len() != payloads.len() {
                    errors.push(SemanticError::new(
                        format!(
                            "enum variant `{}` expects {} payload(s), found {}",
                            variant,
                            payload_templates.len(),
                            payloads.len()
                        ),
                        expr_span(expr),
                    ));
                    return None;
                }

                if def.type_params.len() != exp_args.len() {
                    errors.push(SemanticError::new(
                        format!(
                            "enum `{}` type argument count mismatch",
                            enum_name
                        ),
                        expr_span(expr),
                    ));
                    return None;
                }

                // If the constructor uses explicit type arguments (e.g. `Option<Int>::None`),
                // validate them against the expected type. `_` is resolved from the
                // expected type context.
                if !type_args.is_empty() {
                    if type_args.len() != def.type_params.len() {
                        errors.push(SemanticError::new(
                            format!(
                                "`{enum_name}` expects {} type argument(s) in constructor, found {}",
                                def.type_params.len(),
                                type_args.len()
                            ),
                            expr_span(expr),
                        ));
                        return None;
                    }
                    for (i, tpl) in type_args.iter().enumerate() {
                        match tpl {
                            TypeExpr::Infer => {}
                            other => {
                                let Some(got) = ty_from_type_expr(
                                    other,
                                    expr_span(expr),
                                    errors,
                                    &[],
                                    &ctx.structs,
                                    &ctx.enums,
                                    ctx.aliases,
                                ) else {
                                    return None;
                                };
                                if got != exp_args[i] {
                                    errors.push(SemanticError::new(
                                        format!(
                                            "enum type argument mismatch in constructor: expected `{}`, found `{}`",
                                            ty_name(&exp_args[i]),
                                            ty_name(&got)
                                        ),
                                        expr_span(expr),
                                    ));
                                    return None;
                                }
                            }
                        }
                    }
                }

                let substs: HashMap<String, Ty> = def
                    .type_params
                    .iter()
                    .zip(exp_args.iter())
                    .map(|(tp, ty)| (tp.name.clone(), ty.clone()))
                    .collect();

                // Validate payload types against the expected enum type.
                for (tpl, payload_expr) in payload_templates.iter().zip(payloads.iter()) {
                    let Some(tpl_ty) = ty_from_type_expr(
                        tpl,
                        expr_span(payload_expr),
                        errors,
                        &def.type_params,
                        &ctx.structs,
                        &ctx.enums,
                        ctx.aliases,
                    ) else {
                        return None;
                    };
                    let expected_payload_ty = infer::instantiate_ty(&tpl_ty, &substs);
                    let got_payload = check_expr_expected(
                        payload_expr,
                        ctx,
                        errors,
                        Some(&expected_payload_ty),
                    )?;
                    if got_payload != expected_payload_ty {
                        errors.push(SemanticError::new(
                            format!(
                                "enum variant `{}` payload type mismatch: expected `{}`, found `{}`",
                                variant,
                                ty_name(&expected_payload_ty),
                                ty_name(&got_payload)
                            ),
                            expr_span(payload_expr),
                        ));
                        return None;
                    }
                }

                return Some(Ty::Enum {
                    name: exp_enum_name.clone(),
                    args: exp_args.clone(),
                });
            }
        }
    }

    // Contextual typing for enum constructor syntax parsed as `TypeMethodCall`.
    if let AstNode::TypeMethodCall {
        type_name,
        method,
        arguments,
        span,
    } = expr
    {
        if let Some(Ty::Enum {
            name: exp_enum_name, ..
        }) = expected
        {
            if exp_enum_name == type_name {
                let payloads = arguments
                    .iter()
                    .map(|a| match a {
                        CallArg::Positional(v) => v.clone(),
                        CallArg::Named { value, .. } => value.clone(),
                    })
                    .collect::<Vec<_>>();
                let as_ctor = AstNode::EnumVariantCtor {
                    enum_name: type_name.clone(),
                    type_args: Vec::new(),
                    variant: method.clone(),
                    payloads,
                    span: *span,
                };
                return check_expr_expected(&as_ctor, ctx, errors, expected);
            }
        }
    }

    check_expr(expr, ctx, errors)
}

pub fn check_program(ast: &AstNode) -> Vec<SemanticError> {
    let mut errors = Vec::new();

    let AstNode::Program(items) = ast else {
        errors.push(SemanticError::new(
            "internal error: root must be a program",
            Span::new(1, 1, 1),
        ));
        return errors;
    };

    let mut registry: HashMap<String, FuncSig> = HashMap::new();
    // For instance method fallback: when `[Elem]::method` is missing, use generic `[T]::method` if registered.
    let mut generic_array_ext: HashMap<String, Vec<String>> = HashMap::new();
    let mut generic_enum_ext: HashMap<String, Vec<String>> = HashMap::new();
    let mut generic_struct_ext: HashMap<String, Vec<String>> = HashMap::new();
    let mut defined_names: HashMap<String, Span> = HashMap::new();
    let mut function_registry_name: HashMap<(String, usize, usize), String> = HashMap::new();
    let mut global_order: Vec<GlobalDecl> = Vec::new();

    // Collect struct declarations first so type expressions can reference them.
    let mut structs_ast: HashMap<String, (Vec<GenericParam>, Vec<crate::ast::StructFieldDecl>, bool)> =
        HashMap::new();
    for item in items {
        if let AstNode::StructDef {
            name,
            type_params,
            fields,
            is_unit,
            ..
        } = item
        {
            if structs_ast.contains_key(name) {
                // Name collisions among structs are reported during the final construction.
                continue;
            }
            structs_ast.insert(
                name.clone(),
                (type_params.clone(), fields.clone(), *is_unit),
            );
        }
    }

    // Collect enum declarations so type expressions can reference them.
    let mut enums_ast: HashMap<String, (Vec<GenericParam>, HashMap<String, Vec<TypeExpr>>)> =
        HashMap::new();
    for item in items {
        if let AstNode::EnumDef {
            name,
            type_params,
            variants,
            ..
        } = item
        {
            if enums_ast.contains_key(name) {
                continue;
            }
            let mut variant_map: HashMap<String, Vec<TypeExpr>> = HashMap::new();
            for v in variants {
                if variant_map.contains_key(&v.name) {
                    errors.push(SemanticError::new(
                        format!("duplicate variant `{}` in enum `{name}`", v.name),
                        v.name_span,
                    ));
                    continue;
                }
                variant_map.insert(v.name.clone(), v.payload_types.clone());
            }
            enums_ast.insert(name.clone(), (type_params.clone(), variant_map));
        }
    }

    // Collect type alias declarations.
    let mut aliases: HashMap<String, AliasDef> = HashMap::new();
    for item in items {
        if let AstNode::TypeAlias {
            name,
            type_params,
            target,
            name_span,
            ..
        } = item
        {
            if aliases.contains_key(name) {
                errors.push(SemanticError::new(
                    format!("redefinition of `{name}`"),
                    *name_span,
                ));
                continue;
            }
            aliases.insert(
                name.clone(),
                AliasDef {
                    type_params: type_params.clone(),
                    target: target.clone(),
                },
            );
        }
    }

    // Pre-create empty entries so `ty_from_type_expr` can resolve struct names.
    let mut structs: HashMap<String, StructDef> = HashMap::new();
    for (name, (type_params, _fields, is_unit)) in &structs_ast {
        structs.insert(
            name.clone(),
            StructDef {
                type_params: type_params.clone(),
                fields: HashMap::new(),
                is_unit: *is_unit,
            },
        );
    }

    // Pre-create empty entries so `ty_from_type_expr` can resolve enum names.
    let mut enums: HashMap<String, EnumDef> = HashMap::new();
    for (name, (type_params, variants)) in &enums_ast {
        enums.insert(
            name.clone(),
            EnumDef {
                type_params: type_params.clone(),
                variants: variants.clone(),
            },
        );
    }

    // Now fill in each struct's field types.
    for (name, (type_params, fields, is_unit)) in &structs_ast {
        let mut field_tys: HashMap<String, Ty> = HashMap::new();
        for f in fields {
            if field_tys.contains_key(&f.name) {
                errors.push(SemanticError::new(
                    format!("duplicate field `{}` in struct `{name}`", f.name),
                    f.name_span,
                ));
                continue;
            }
            if let Some(ty) = ty_from_type_expr(
                &f.ty,
                f.ty_span,
                &mut errors,
                type_params,
                &structs,
                &enums,
                &aliases,
            )
            {
                field_tys.insert(f.name.clone(), ty);
            }
        }
        structs.insert(
            name.clone(),
            StructDef {
                type_params: type_params.clone(),
                fields: field_tys,
                is_unit: *is_unit,
            },
        );
    }

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
                if defined_names.contains_key(name) {
                    errors.push(SemanticError::new(
                        format!("redefinition of `{name}`"),
                        *name_span,
                    ));
                    continue;
                }
                defined_names.insert(name.clone(), *name_span);
                if params.iter().any(|p| p.default_value.is_some()) {
                    errors.push(SemanticError::new(
                        format!("internal function `{name}` cannot have default parameter values"),
                        *name_span,
                    ));
                }
                let param_tys = check_param_list(
                    params,
                    type_params,
                    &structs,
                    &enums,
                    &aliases,
                    &mut errors,
                );
                let ret = return_type
                    .as_ref()
                    .map(|t| {
                        ty_from_type_expr(
                            t,
                            *name_span,
                            &mut errors,
                            type_params,
                            &structs,
                            &enums,
                            &aliases,
                        )
                    })
                    .unwrap_or(None);
                if *is_async {
                    if !matches!(ret, Some(Ty::Task(_))) {
                        errors.push(SemanticError::new(
                            format!("`internal async func {name}` must return `Task<...>`"),
                            *name_span,
                        ));
                    }
                }
                registry.insert(
                    name.clone(),
                    FuncSig {
                        type_params: type_params.clone(),
                        params: param_tys,
                        params_ast: params.clone(),
                        ret,
                        is_async: *is_async,
                        extension_receiver_ty: None,
                    },
                );

                // Internal extension-receiver functions (e.g. `internal func Dict<K, V>::get(...)`)
                // must participate in generic dispatch for `foo.bar(...)` calls where the
                // concrete callee name is not monomorphized.
                if let Some((_, method_name)) = name.split_once("::") {
                    if let Some(Ty::Struct(_)) = registry
                        .get(name)
                        .and_then(|sig| sig.params.get(0).cloned())
                    {
                        let entry = generic_struct_ext
                            .entry(method_name.to_string())
                            .or_default();
                        if !entry.contains(name) {
                            entry.push(name.clone());
                        }
                    }
                }
            }
            AstNode::Function {
                name,
                extension_receiver,
                type_params,
                params,
                return_type,
                body: _,
                name_span,
                closing_span: _,
                is_async,
                ..
            } => {
                let is_op_overload = is_operator_method_name(name);
                if defined_names.contains_key(name) && !is_op_overload {
                    errors.push(SemanticError::new(
                        format!("redefinition of `{name}`"),
                        *name_span,
                    ));
                    continue;
                }
                if !defined_names.contains_key(name) {
                    defined_names.insert(name.clone(), *name_span);
                }
                let param_tys = check_param_list(
                    params,
                    type_params,
                    &structs,
                    &enums,
                    &aliases,
                    &mut errors,
                );
                if let Some(ext) = extension_receiver.as_ref() {
                    if let TypeExpr::Array(inner) = &ext.ty {
                        if let TypeExpr::TypeParam(tp) = inner.as_ref() {
                            if !type_params.iter().any(|p| p.name == *tp) {
                                errors.push(SemanticError::new(
                                    format!(
                                        "extension receiver `[type {tp}]` requires `{tp}` to be a type parameter of the function"
                                    ),
                                    *name_span,
                                ));
                            } else {
                                let entry = generic_array_ext
                                    .entry(ext.method_name.clone())
                                    .or_default();
                                if !entry.contains(&name) {
                                    entry.push(name.clone());
                                }
                            }
                        } else if let TypeExpr::EnumApp { args, .. } = inner.as_ref() {
                            if !args.iter().all(|a| matches!(a, TypeExpr::TypeParam(_))) {
                                errors.push(SemanticError::new(
                                    "generic array extension receiver `[Result<...>]` must use only `type` parameters for enum arguments"
                                        .to_string(),
                                    *name_span,
                                ));
                            } else {
                                let mut ok = true;
                                for a in args {
                                    if let TypeExpr::TypeParam(tp) = a {
                                        if !type_params.iter().any(|p| p.name == *tp) {
                                            errors.push(SemanticError::new(
                                                format!(
                                                    "extension receiver enum type argument `{tp}` must be a type parameter of the function"
                                                ),
                                                *name_span,
                                            ));
                                            ok = false;
                                        }
                                    }
                                }
                                if ok {
                                    let entry = generic_array_ext
                                        .entry(ext.method_name.clone())
                                        .or_default();
                                    if !entry.contains(&name) {
                                        entry.insert(0, name.clone());
                                    }
                                }
                            }
                        }
                    } else if let TypeExpr::EnumApp {
                        name: recv_name,
                        args,
                        ..
                    } = &ext.ty
                    {
                        let all_type_params =
                            args.iter().all(|a| matches!(a, TypeExpr::TypeParam(_)));

                        // Old behavior (broken for structs): always enforced the
                        // “generic enum extension receiver must use only `type` parameters ...”
                        // rule, even when the receiver base was a concrete struct (e.g. `Task<()>`).
                        if enums.contains_key(recv_name) {
                            if !all_type_params {
                                errors.push(SemanticError::new(
                                    "generic enum extension receiver must use only `type` parameters for enum arguments"
                                        .to_string(),
                                    *name_span,
                                ));
                            } else {
                                let mut ok = true;
                                for a in args {
                                    if let TypeExpr::TypeParam(tp) = a {
                                        if !type_params.iter().any(|p| p.name == *tp) {
                                            errors.push(SemanticError::new(
                                                format!(
                                                    "extension receiver enum type argument `{tp}` must be a type parameter of the function"
                                                ),
                                                *name_span,
                                            ));
                                            ok = false;
                                        }
                                    }
                                }
                                if ok {
                                    let entry = generic_enum_ext
                                        .entry(ext.method_name.clone())
                                        .or_default();
                                    if !entry.contains(&name) {
                                        entry.insert(0, name.clone());
                                    }
                                }
                            }
                        } else if structs.contains_key(recv_name) {
                            // For structs, allow concrete args like `Task<()>` and only
                            // participate in generic receiver registration when args are
                            // truly type parameters.
                            if all_type_params {
                                let mut ok = true;
                                for a in args {
                                    if let TypeExpr::TypeParam(tp) = a {
                                        if !type_params.iter().any(|p| p.name == *tp) {
                                            errors.push(SemanticError::new(
                                                format!(
                                                    "extension receiver enum type argument `{tp}` must be a type parameter of the function"
                                                ),
                                                *name_span,
                                            ));
                                            ok = false;
                                        }
                                    }
                                }
                                if ok {
                                    let entry = generic_enum_ext
                                        .entry(ext.method_name.clone())
                                        .or_default();
                                    if !entry.contains(&name) {
                                        entry.insert(0, name.clone());
                                    }
                                    // Also register generic `EnumApp` receivers that are structs (e.g. `Task<T>::wait_all`).
                                    let entry = generic_struct_ext
                                        .entry(ext.method_name.clone())
                                        .or_default();
                                    if !entry.contains(&name) {
                                        entry.push(name.clone());
                                    }
                                }
                            }
                        } else {
                            // Unknown receiver base: keep the original strictness.
                            if !all_type_params {
                                errors.push(SemanticError::new(
                                    "generic enum extension receiver must use only `type` parameters for enum arguments"
                                        .to_string(),
                                    *name_span,
                                ));
                            } else {
                                let mut ok = true;
                                for a in args {
                                    if let TypeExpr::TypeParam(tp) = a {
                                        if !type_params.iter().any(|p| p.name == *tp) {
                                            errors.push(SemanticError::new(
                                                format!(
                                                    "extension receiver enum type argument `{tp}` must be a type parameter of the function"
                                                ),
                                                *name_span,
                                            ));
                                            ok = false;
                                        }
                                    }
                                }
                                if ok {
                                    let entry = generic_enum_ext
                                        .entry(ext.method_name.clone())
                                        .or_default();
                                    if !entry.contains(&name) {
                                        entry.insert(0, name.clone());
                                    }
                                }
                            }
                        }
                    } else if let TypeExpr::Named(n) = &ext.ty {
                        if type_params.iter().any(|tp| tp.name == *n) {
                            let entry = generic_struct_ext
                                .entry(ext.method_name.clone())
                                .or_default();
                            if !entry.contains(&name) {
                                entry.push(name.clone());
                            }
                        }
                    }
                    let receiver_ty = ty_from_type_expr(
                        &ext.ty,
                        *name_span,
                        &mut errors,
                        type_params,
                        &structs,
                        &enums,
                        &aliases,
                    );
                    if let Some(receiver_ty) = receiver_ty {
                        if let Some(first_param) = params.first() {
                            if first_param.name == "self" {
                                if param_tys.first() != Some(&receiver_ty) {
                                    errors.push(SemanticError::new(
                                        "extension `self` parameter must use the extension receiver type",
                                        first_param.name_span,
                                    ));
                                }
                            }
                        }
                    }
                }
                let ret = return_type
                    .as_ref()
                    .map(|t| {
                        ty_from_type_expr(
                            t,
                            *name_span,
                            &mut errors,
                            type_params,
                            &structs,
                            &enums,
                            &aliases,
                        )
                    })
                    .unwrap_or(None);
                if *is_async {
                    if !matches!(ret, Some(Ty::Task(_))) {
                        errors.push(SemanticError::new(
                            format!("`async func {name}` must return `Task<...>`"),
                            *name_span,
                        ));
                    }
                }
                let extension_receiver_ty = extension_receiver.as_ref().and_then(|ext| {
                    ty_from_type_expr(
                        &ext.ty,
                        *name_span,
                        &mut errors,
                        type_params,
                        &structs,
                        &enums,
                        &aliases,
                    )
                });
                let reg_name = if is_op_overload {
                    mangle_operator_overload_name(name, &param_tys)
                } else {
                    name.clone()
                };
                function_registry_name.insert(
                    (name.clone(), name_span.line, name_span.column),
                    reg_name.clone(),
                );
                registry.insert(
                    reg_name,
                    FuncSig {
                        type_params: type_params.clone(),
                        params: param_tys,
                        params_ast: params.clone(),
                        ret,
                        is_async: *is_async,
                        extension_receiver_ty,
                    },
                );
            }
            AstNode::StructDef { .. } => {}
            AstNode::EnumDef { .. } => {}
            AstNode::TypeAlias { .. } => {}
            AstNode::Import { .. } | AstNode::ExportAlias { .. } => {}
            AstNode::Let {
                pattern,
                type_annotation,
                initializer,
                span,
            } => {
                for (n, sp) in collect_pattern_binding_names(pattern) {
                    if defined_names.contains_key(&n) {
                        errors.push(SemanticError::new(
                            format!("redefinition of `{n}`"),
                            sp,
                        ));
                        continue;
                    }
                    defined_names.insert(n, sp);
                }
                global_order.push(GlobalDecl {
                    pattern: pattern.clone(),
                    type_annotation: type_annotation.clone(),
                    initializer: initializer.clone(),
                    span: *span,
                });
            }
            AstNode::SingleLineComment(_) | AstNode::MultiLineComment(_) => {}
            _ => {
                errors.push(SemanticError::new(
                    "only `import`, `internal func`, `func`, `type`, `let`, and comments are allowed at the top level",
                    span_of_item(item),
                ));
            }
        }
    }

    if !registry.contains_key("main") {
        errors.push(SemanticError::new(
            "program must define `func main() { ... }` or `async func main() { ... }`",
            Span::new(1, 1, 1),
        ));
    }

    let mut globals: HashMap<String, Ty> = HashMap::new();
    for decl in &global_order {
        type_check_global_decl(
            decl,
            &registry,
            &structs,
            &enums,
            &aliases,
            &mut errors,
            &mut globals,
            &generic_array_ext,
            &generic_enum_ext,
            &generic_struct_ext,
        );
    }

    // Infer return type for `func name(args) = expr;` when the user omitted
    // an explicit `: Type` in the signature.
    //
    // We detect arrow functions by a single-statement body:
    // `return <expr>;`.
    let mut arrow_candidates: Vec<(String, Span, Vec<Param>, AstNode)> = Vec::new();
    for item in items {
        if let AstNode::Function {
            name,
            name_span,
            params,
            return_type: None,
            body,
            ..
        } = item
        {
            if name == "main" {
                continue;
            }
            let non_comment: Vec<&AstNode> = body
                .iter()
                .filter(|n| {
                    !matches!(n, AstNode::SingleLineComment(_) | AstNode::MultiLineComment(_))
                })
                .collect();
            if non_comment.len() == 1 {
                if let AstNode::Return { value: Some(v), .. } = non_comment[0] {
                    arrow_candidates.push((name.clone(), *name_span, params.clone(), (*v.clone())));
                }
            }
        }
    }

    // Fixed-point inference: if arrow expressions call other arrow functions,
    // we may need multiple iterations.
    let mut changed = true;
    for _ in 0..arrow_candidates.len().max(1) {
        if !changed {
            break;
        }
        changed = false;
        for (name, name_span, params_ast, ret_expr) in &arrow_candidates {
            let reg_name = function_registry_name
                .get(&(name.clone(), name_span.line, name_span.column))
                .cloned()
                .unwrap_or_else(|| name.clone());
            let sig_ret_is_none = registry
                .get(&reg_name)
                .and_then(|s| s.ret.as_ref())
                .is_none();
            if !sig_ret_is_none {
                continue;
            }

            let inferred_ty = {
                let mut ctx = BodyCtx::new(
                    &registry,
                    &structs,
                    &enums,
                    &aliases,
                    &globals,
                    &generic_array_ext,
                    &generic_enum_ext,
                    &generic_struct_ext,
                );
                ctx.push_scope();
                match registry.get(&reg_name) {
                    Some(sig) => {
                        for (p, ty) in params_ast.iter().zip(sig.params.iter()) {
                            ctx.declare_param(p, ty.clone(), &mut errors);
                        }
                        check_expr(ret_expr, &mut ctx, &mut errors)
                            .map(|t| infer::resolve_ty(&t, &ctx.infer_ctx))
                    }
                    None => None,
                }
            };

            if let Some(inferred_ty) = inferred_ty {
                if let Some(sig_mut) = registry.get_mut(&reg_name) {
                    // For `main`, a non-unit return type will be rejected later,
                    // but we still set it here so call sites can type-check.
                    let final_ty = if name == "main" && inferred_ty == Ty::Unit {
                        Ty::Unit
                    } else {
                        inferred_ty
                    };
                    if sig_mut.ret.as_ref() != Some(&final_ty) {
                        sig_mut.ret = Some(final_ty);
                        changed = true;
                    }
                }
            }
        }
    }

    for item in items {
        let AstNode::Function {
            name,
            params,
            return_type: _,
            type_params: _,
            body,
            name_span,
            closing_span,
            is_async,
            ..
        } = item
        else {
            continue;
        };
        let reg_name = function_registry_name
            .get(&(name.clone(), name_span.line, name_span.column))
            .cloned()
            .unwrap_or_else(|| name.clone());
        let Some(sig) = registry.get(&reg_name) else {
            continue;
        };
        let ret_ty = sig.ret.clone();

        if name == "main" {
            if !params.is_empty() {
                errors.push(SemanticError::new(
                    "`main` must take no parameters",
                    *name_span,
                ));
            }
            let sync_ok = !*is_async
                && (ret_ty.is_none() || ret_ty.as_ref() == Some(&Ty::Unit));
            let async_ok = *is_async
                && ret_ty.as_ref().is_some_and(|r| {
                    matches!(r, Ty::Task(inner) if **inner == Ty::Unit)
                });
            if !sync_ok && !async_ok {
                errors.push(SemanticError::new(
                    "`main` must be `func main()` / `func main(): ()`, or `async func main(): Task` / `Task<()>`",
                    *name_span,
                ));
            }
        }

        let mut ctx = BodyCtx::new(
            &registry,
            &structs,
            &enums,
            &aliases,
            &globals,
            &generic_array_ext,
            &generic_enum_ext,
            &generic_struct_ext,
        );
        ctx.in_async = *is_async;
        ctx.push_scope();
        for (p, ty) in params.iter().zip(sig.params.iter()) {
            ctx.declare_param(p, ty.clone(), &mut errors);
        }
        check_function_body_stmts(body, ret_ty.as_ref(), *closing_span, &mut ctx, &mut errors);
    }

    errors.sort_by(|a, b| {
        a.span
            .line
            .cmp(&b.span.line)
            .then_with(|| a.span.column.cmp(&b.span.column))
    });
    errors
}

fn collect_pattern_binding_names(pattern: &Pattern) -> Vec<(String, Span)> {
    let mut out = Vec::new();
    fn walk(p: &Pattern, out: &mut Vec<(String, Span)>) {
        match p {
            Pattern::Wildcard { .. } => {}
            Pattern::Binding { name, name_span, .. } => {
                out.push((name.clone(), *name_span));
            }
            Pattern::IntLiteral { .. }
            | Pattern::StringLiteral { .. }
            | Pattern::BoolLiteral { .. } => {}
            Pattern::Tuple { elements, .. } => {
                for e in elements {
                    match e {
                        PatternElem::Pattern(p) => walk(p, out),
                        PatternElem::Rest(_) => {}
                    }
                }
            }
            Pattern::Array { elements, .. } => {
                for e in elements {
                    match e {
                        PatternElem::Pattern(p) => walk(p, out),
                        PatternElem::Rest(_) => {}
                    }
                }
            }
            Pattern::Struct { fields, .. } => {
                for f in fields {
                    walk(&f.pattern, out);
                }
            }
            Pattern::EnumVariant { payloads, .. } => {
                for p in payloads {
                    walk(p, out);
                }
            }
        }
    }
    walk(pattern, &mut out);
    out
}

/// Split at the single `..`. Returns `(prefix, suffix, has_rest)`.
fn tuple_pattern_prefix_suffix<'a>(
    elements: &'a [PatternElem],
    tuple_span: Span,
    errors: &mut Vec<SemanticError>,
) -> Option<(Vec<&'a PatternElem>, Vec<&'a PatternElem>, bool)> {
    let mut rest_idx: Option<usize> = None;
    for (i, e) in elements.iter().enumerate() {
        if matches!(e, PatternElem::Rest(_)) {
            if rest_idx.is_some() {
                errors.push(SemanticError::new(
                    "multiple `..` in tuple pattern",
                    tuple_span,
                ));
                return None;
            }
            rest_idx = Some(i);
        }
    }
    match rest_idx {
        None => Some((elements.iter().collect(), Vec::new(), false)),
        Some(i) => Some((
            elements[0..i].iter().collect(),
            elements[i + 1..].iter().collect(),
            true,
        )),
    }
}

fn type_check_global_decl(
    decl: &GlobalDecl,
    registry: &HashMap<String, FuncSig>,
    structs: &HashMap<String, StructDef>,
    enums: &HashMap<String, EnumDef>,
    aliases: &HashMap<String, AliasDef>,
    errors: &mut Vec<SemanticError>,
    globals: &mut HashMap<String, Ty>,
    generic_array_ext: &HashMap<String, Vec<String>>,
    generic_enum_ext: &HashMap<String, Vec<String>>,
    generic_struct_ext: &HashMap<String, Vec<String>>,
) {
    let init = decl.initializer.as_deref();
    let ann = decl.type_annotation.as_ref();

    let prior_globals = globals.clone();
    let mut expr_ctx = BodyCtx::new(
        registry,
        structs,
        enums,
        aliases,
        &prior_globals,
        generic_array_ext,
        generic_enum_ext,
        generic_struct_ext,
    );
    expr_ctx.push_scope();

    match &decl.pattern {
        Pattern::Wildcard { .. } => {
            if let Some(expr) = init {
                let _ = check_expr(expr, &mut expr_ctx, errors);
            } else {
                errors.push(SemanticError::new(
                    "global `let _` must have an initializer",
                    decl.span,
                ));
            }
        }
        Pattern::Binding { name, name_span } => {
            match (ann, init) {
                (Some(te), Some(expr)) => {
                    let Some(et) = ty_from_type_expr(
                        te,
                        *name_span,
                        errors,
                        &[],
                        structs,
                        enums,
                        aliases,
                    )
                    else {
                        return;
                    };
                    let got = check_expr(expr, &mut expr_ctx, errors);
                    if let Some(got) = got {
                        if got != et {
                            errors.push(SemanticError::new(
                                format!(
                                    "global `{name}` initializer has wrong type (expected `{}`, found `{}`)",
                                    ty_name(&et),
                                    ty_name(&got)
                                ),
                                expr_span(expr),
                            ));
                            return;
                        }
                    }
                    globals.insert(name.clone(), et);
                }
                (None, Some(expr)) => {
                    if let Some(got) = check_expr(expr, &mut expr_ctx, errors) {
                        globals.insert(name.clone(), got);
                    }
                }
                _ => {
                    errors.push(SemanticError::new(
                        format!("global `{name}` must have an initializer"),
                        *name_span,
                    ));
                }
            }
        }
        Pattern::IntLiteral { span, .. }
        | Pattern::StringLiteral { span, .. }
        | Pattern::BoolLiteral { span, .. } => {
            errors.push(SemanticError::new(
                "literal patterns are not allowed at the top level",
                *span,
            ));
        }
        Pattern::Tuple { elements, span } => {
            if elements.is_empty() {
                if let Some(expr) = init {
                    let got = check_expr(expr, &mut expr_ctx, errors);
                    if let Some(got) = got {
                        if got != Ty::Unit {
                            errors.push(SemanticError::new(
                                "expected unit value `()`",
                                expr_span(expr),
                            ));
                        }
                    }
                } else {
                    errors.push(SemanticError::new(
                        "global `let ()` must have an initializer",
                        *span,
                    ));
                }
                return;
            }
            errors.push(SemanticError::new(
                "tuple patterns are not allowed at the top level",
                *span,
            ));
        }
        Pattern::Array { elements: _, span } => {
            errors.push(SemanticError::new(
                "array patterns are not allowed at the top level",
                *span,
            ));
        }
        Pattern::Struct { span, .. } => {
            errors.push(SemanticError::new(
                "struct patterns are not allowed at the top level",
                *span,
            ));
        }
        Pattern::EnumVariant { span, .. } => {
            errors.push(SemanticError::new(
                "enum patterns are not allowed at the top level",
                *span,
            ));
        }
    }
}

/// Given a generic declaration and explicit type arguments, fill trailing parameters from defaults.
fn merge_type_args_with_defaults(
    params: &[GenericParam],
    args: &[TypeExpr],
    span: Span,
    errors: &mut Vec<SemanticError>,
) -> Option<Vec<TypeExpr>> {
    if args.len() > params.len() {
        errors.push(SemanticError::new(
            format!(
                "too many type arguments: expected at most {}, found {}",
                params.len(),
                args.len()
            ),
            span,
        ));
        return None;
    }
    let mut out: Vec<TypeExpr> = args.to_vec();
    for i in args.len()..params.len() {
        match &params[i].default {
            Some(d) => out.push(d.clone()),
            None => {
                errors.push(SemanticError::new(
                    format!(
                        "generic parameter `{}` requires a type argument (no default)",
                        params[i].name
                    ),
                    span,
                ));
                return None;
            }
        }
    }
    Some(out)
}

fn ty_from_type_expr(
    te: &TypeExpr,
    span: Span,
    errors: &mut Vec<SemanticError>,
    type_params: &[GenericParam],
    structs: &HashMap<String, StructDef>,
    enums: &HashMap<String, EnumDef>,
    aliases: &HashMap<String, AliasDef>,
) -> Option<Ty> {
    fn substitute_alias_type(te: &TypeExpr, substs: &HashMap<String, TypeExpr>) -> TypeExpr {
        match te {
            TypeExpr::TypeParam(n) => substs
                .get(n)
                .cloned()
                .unwrap_or_else(|| TypeExpr::TypeParam(n.clone())),
            TypeExpr::Named(n) => substs
                .get(n)
                .cloned()
                .unwrap_or_else(|| TypeExpr::Named(n.clone())),
            TypeExpr::EnumApp { name, args } => TypeExpr::EnumApp {
                name: name.clone(),
                args: args.iter().map(|a| substitute_alias_type(a, substs)).collect(),
            },
            TypeExpr::Tuple(parts) => {
                TypeExpr::Tuple(parts.iter().map(|p| substitute_alias_type(p, substs)).collect())
            }
            TypeExpr::Array(inner) => TypeExpr::Array(Box::new(substitute_alias_type(inner, substs))),
            TypeExpr::Function { params, ret } => TypeExpr::Function {
                params: params
                    .iter()
                    .map(|p| FunctionTypeParam {
                        name: p.name.clone(),
                        ty: substitute_alias_type(&p.ty, substs),
                        has_default: p.has_default,
                    })
                    .collect(),
                ret: Box::new(substitute_alias_type(ret, substs)),
            },
            other => other.clone(),
        }
    }

    fn inner(
        te: &TypeExpr,
        span: Span,
        errors: &mut Vec<SemanticError>,
        type_params: &[GenericParam],
        structs: &HashMap<String, StructDef>,
        enums: &HashMap<String, EnumDef>,
        aliases: &HashMap<String, AliasDef>,
        alias_stack: &mut Vec<String>,
    ) -> Option<Ty> {
    fn struct_inst_name(name: &str, args: &[Ty]) -> String {
        if args.is_empty() {
            name.to_string()
        } else {
            let inner = args.iter().map(ty_name).collect::<Vec<_>>().join(", ");
            format!("{name}<{inner}>")
        }
    }
    match te {
        TypeExpr::Infer => {
            Some(Ty::Any)
        }
        TypeExpr::Named(n) => match n.as_str() {
            "Int" => Some(Ty::Int),
            "Float" => Some(Ty::Float),
            "String" => Some(Ty::String),
            "Bool" => Some(Ty::Bool),
            "Any" => Some(Ty::Any),
            // Type parameter in a generic function signature.
            _ if type_params.iter().any(|tp| tp.name == *n) => Some(Ty::TypeParam(n.clone())),
            _ if structs.contains_key(n) => {
                let sd = structs.get(n).expect("checked contains_key");
                if n == "Task" {
                    let merged = merge_type_args_with_defaults(&sd.type_params, &[], span, errors)?;
                    if merged.len() != 1 {
                        errors.push(SemanticError::new(
                            "`Task` expects exactly one type parameter (or a default)".to_string(),
                            span,
                        ));
                        return None;
                    }
                    let payload = inner(
                        &merged[0],
                        span,
                        errors,
                        type_params,
                        structs,
                        enums,
                        aliases,
                        alias_stack,
                    )?;
                    return Some(Ty::Task(Box::new(payload)));
                }
                if sd.type_params.is_empty() {
                    Some(Ty::Struct(n.clone()))
                } else {
                    let merged = merge_type_args_with_defaults(&sd.type_params, &[], span, errors)?;
                    let mut arg_tys = Vec::with_capacity(merged.len());
                    for a in &merged {
                        arg_tys.push(inner(
                            a,
                            span,
                            errors,
                            type_params,
                            structs,
                            enums,
                            aliases,
                            alias_stack,
                        )?);
                    }
                    Some(Ty::Struct(struct_inst_name(n, &arg_tys)))
                }
            }
            _ if enums.contains_key(n) => {
                // Non-generic enum type usage parses as `TypeExpr::Named`.
                // Resolve it into `Ty::Enum` with zero type arguments (using defaults if any).
                let def = enums.get(n).expect("checked contains_key");
                let merged =
                    merge_type_args_with_defaults(&def.type_params, &[], span, errors)?;
                let mut arg_tys = Vec::with_capacity(merged.len());
                for a in &merged {
                    arg_tys.push(inner(
                        a,
                        span,
                        errors,
                        type_params,
                        structs,
                        enums,
                        aliases,
                        alias_stack,
                    )?);
                }
                Some(Ty::Enum {
                    name: n.clone(),
                    args: arg_tys,
                })
            }
            _ if aliases.contains_key(n) => {
                let ad = aliases.get(n).expect("checked contains_key");
                let full_args = merge_type_args_with_defaults(&ad.type_params, &[], span, errors)?;
                if alias_stack.contains(n) {
                    errors.push(SemanticError::new(
                        format!("cyclic type alias detected involving `{n}`"),
                        span,
                    ));
                    return None;
                }
                let mut substs: HashMap<String, TypeExpr> = HashMap::new();
                for (tp, arg) in ad.type_params.iter().zip(full_args.iter()) {
                    substs.insert(tp.name.clone(), arg.clone());
                }
                alias_stack.push(n.clone());
                let expanded = substitute_alias_type(&ad.target, &substs);
                let out = inner(
                    &expanded,
                    span,
                    errors,
                    type_params,
                    structs,
                    enums,
                    aliases,
                    alias_stack,
                );
                let _ = alias_stack.pop();
                out
            }
            _ => {
                errors.push(SemanticError::new(
                        format!(
                            "unknown type `{n}` (expected `Int`, `Float`, `String`, `Bool`, `Any`, struct types, `()`, an array type, or a tuple type)"
                        ),
                    span,
                ));
                None
            }
        },
        TypeExpr::EnumApp { name, args } => {
            if let Some(ad) = aliases.get(name) {
                let Some(full_args) =
                    merge_type_args_with_defaults(&ad.type_params, args, span, errors)
                else {
                    return None;
                };
                if alias_stack.contains(name) {
                    errors.push(SemanticError::new(
                        format!("cyclic type alias detected involving `{name}`"),
                        span,
                    ));
                    return None;
                }
                let mut substs: HashMap<String, TypeExpr> = HashMap::new();
                for (tp, arg) in ad.type_params.iter().zip(full_args.iter()) {
                    substs.insert(tp.name.clone(), arg.clone());
                }
                alias_stack.push(name.clone());
                let expanded = substitute_alias_type(&ad.target, &substs);
                let out = inner(
                    &expanded,
                    span,
                    errors,
                    type_params,
                    structs,
                    enums,
                    aliases,
                    alias_stack,
                );
                let _ = alias_stack.pop();
                out
            } else if let Some(def) = enums.get(name) {
                let Some(full_args) =
                    merge_type_args_with_defaults(&def.type_params, args, span, errors)
                else {
                    return None;
                };
                let mut arg_tys = Vec::with_capacity(full_args.len());
                for a in &full_args {
                    arg_tys.push(inner(
                        a,
                        span,
                        errors,
                        type_params,
                        structs,
                        enums,
                        aliases,
                        alias_stack,
                    )?);
                }
                Some(Ty::Enum {
                    name: name.clone(),
                    args: arg_tys,
                })
            } else if let Some(sd) = structs.get(name) {
                if name == "Task" {
                    let Some(full_args) =
                        merge_type_args_with_defaults(&sd.type_params, args, span, errors)
                    else {
                        return None;
                    };
                    if full_args.len() != 1 {
                        errors.push(SemanticError::new(
                            "`Task` expects exactly one type parameter".to_string(),
                            span,
                        ));
                        return None;
                    }
                    let payload = inner(
                        &full_args[0],
                        span,
                        errors,
                        type_params,
                        structs,
                        enums,
                        aliases,
                        alias_stack,
                    )?;
                    return Some(Ty::Task(Box::new(payload)));
                }
                let Some(full_args) =
                    merge_type_args_with_defaults(&sd.type_params, args, span, errors)
                else {
                    return None;
                };
                let mut arg_tys = Vec::with_capacity(full_args.len());
                for a in &full_args {
                    arg_tys.push(inner(
                        a,
                        span,
                        errors,
                        type_params,
                        structs,
                        enums,
                        aliases,
                        alias_stack,
                    )?);
                }
                Some(Ty::Struct(struct_inst_name(name, &arg_tys)))
            } else {
                errors.push(SemanticError::new(
                    format!("unknown enum/struct type `{name}`"),
                    span,
                ));
                None
            }
        }
        TypeExpr::Unit => Some(Ty::Unit),
        TypeExpr::Tuple(parts) => {
            let mut tys = Vec::with_capacity(parts.len());
            for p in parts {
                tys.push(inner(
                    p,
                    span,
                    errors,
                    type_params,
                    structs,
                    enums,
                    aliases,
                    alias_stack,
                )?);
            }
            Some(Ty::Tuple(tys))
        }
        TypeExpr::Array(elem_ty) => {
            let et = inner(
                elem_ty,
                span,
                errors,
                type_params,
                structs,
                enums,
                aliases,
                alias_stack,
            )?;
            Some(Ty::Array(Box::new(et)))
        }
        TypeExpr::TypeParam(name) => {
            if type_params.iter().any(|tp| tp.name == *name) {
                Some(Ty::TypeParam(name.clone()))
            } else {
                errors.push(SemanticError::new(
                    format!("unknown type parameter `{name}` in this context"),
                    span,
                ));
                None
            }
        }
        TypeExpr::Function { params, ret } => {
            let mut ptys = Vec::with_capacity(params.len());
            let mut pnames = Vec::with_capacity(params.len());
            for p in params {
                ptys.push(inner(
                    &p.ty,
                    span,
                    errors,
                    type_params,
                    structs,
                    enums,
                    aliases,
                    alias_stack,
                )?);
                pnames.push(p.name.clone());
            }
            let rty = if matches!(ret.as_ref(), TypeExpr::Infer) {
                Ty::Any
            } else {
                inner(
                    ret,
                    span,
                    errors,
                    type_params,
                    structs,
                    enums,
                    aliases,
                    alias_stack,
                )?
            };
            Some(Ty::Function {
                params: ptys,
                param_names: pnames,
                param_has_default: params.iter().map(|p| p.has_default).collect(),
                ret: Box::new(rty),
            })
        }
    }
    }
    let mut alias_stack = Vec::new();
    inner(
        te,
        span,
        errors,
        type_params,
        structs,
        enums,
        aliases,
        &mut alias_stack,
    )
}

fn span_of_item(item: &AstNode) -> Span {
    match item {
        AstNode::IntegerLiteral { span, .. }
        | AstNode::FloatLiteral { span, .. }
        | AstNode::StringLiteral { span, .. }
        | AstNode::Identifier { span, .. }
        | AstNode::BinaryOp { span, .. }
        | AstNode::UnaryOp { span, .. }
        | AstNode::Call { span, .. }
        | AstNode::Return { span, .. }
        | AstNode::Let { span, .. }
        | AstNode::Assign { span, .. }
        | AstNode::UnitLiteral { span, .. }
        | AstNode::TupleLiteral { span, .. }
        | AstNode::TupleField { span, .. }
        | AstNode::ArrayLiteral { span, .. }
        | AstNode::ArrayIndex { span, .. }
        | AstNode::BoolLiteral { span, .. }
        | AstNode::Lambda { span, .. }
        | AstNode::If { span, .. }
        | AstNode::IfLet { span, .. }
        | AstNode::AssignExpr { span, .. }
        | AstNode::CompoundAssign { span, .. }
        | AstNode::While { span, .. }
        | AstNode::Break { span }
        | AstNode::Continue { span } => *span,
        AstNode::Block { closing_span, .. } => *closing_span,
        AstNode::InternalFunction { name_span, .. } | AstNode::Function { name_span, .. } => {
            *name_span
        }
        _ => Span::new(1, 1, 1),
    }
}

fn check_param_list(
    params: &[Param],
    type_params: &[GenericParam],
    structs: &HashMap<String, StructDef>,
    enums: &HashMap<String, EnumDef>,
    aliases: &HashMap<String, AliasDef>,
    errors: &mut Vec<SemanticError>,
) -> Vec<Ty> {
    let mut seen: HashMap<&str, Span> = HashMap::new();
    let mut types = Vec::with_capacity(params.len());
    for (i, p) in params.iter().enumerate() {
        if !p.is_wildcard {
            if seen.insert(p.name.as_str(), p.name_span).is_some() {
                errors.push(SemanticError::new(
                    format!("duplicate parameter `{}`", p.name),
                    p.name_span,
                ));
            }
        }
        if p.is_params {
            if i + 1 != params.len() {
                errors.push(SemanticError::new(
                    "`params` parameter must be the last parameter",
                    p.name_span,
                ));
            }
            if !matches!(p.ty, TypeExpr::Array(_)) {
                errors.push(SemanticError::new(
                    "`params` parameter must have array type `[T]`",
                    p.name_span,
                ));
            }
        }
        types.push(ty_from_type_expr(
            &p.ty,
            p.name_span,
            errors,
            type_params,
            structs,
            enums,
            aliases,
        )
        .unwrap_or(Ty::Int));
    }
    types
}

fn unify_binding(
    ctx: &mut BodyCtx<'_>,
    id: usize,
    got: Ty,
    span: Span,
    errors: &mut Vec<SemanticError>,
) -> bool {
    match &ctx.bindings_ty[id] {
        None => {
            ctx.bindings_ty[id] = Some(got);
            true
        }
        Some(t) if *t == got => true,
        // `Any` is a universal supertype: assignment into an `Any`-annotated
        // binding always succeeds (stored runtime value keeps its concrete type).
        Some(t) if *t == Ty::Any => true,
        Some(t) => infer::unify_types(t, &got, &mut ctx.infer_ctx, errors, span),
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum StmtFlow {
    /// Normal control flow; `reachable` is whether execution can continue to the next statement.
    Next(bool),
    Break,
    Continue,
}

fn check_function_body_stmts(
    body: &[AstNode],
    ret_ty: Option<&Ty>,
    closing_span: Span,
    ctx: &mut BodyCtx<'_>,
    errors: &mut Vec<SemanticError>,
) {
    let mut reachable = true;
    for stmt in body {
        match check_statement(stmt, ret_ty, ctx, reachable, errors) {
            StmtFlow::Next(r) => reachable = r,
            StmtFlow::Break | StmtFlow::Continue => reachable = true,
        }
    }
    if let Some(rt) = ret_ty {
        let must_return = match rt {
            Ty::Task(inner) => **inner != Ty::Unit,
            _ => *rt != Ty::Unit,
        };
        if reachable && must_return {
            errors.push(SemanticError::new(
                "function with return type can fall off the end without `return`",
                closing_span,
            ));
        }
    }
    for ty in &mut ctx.bindings_ty {
        if let Some(t) = ty.clone() {
            *ty = Some(infer::resolve_ty(&t, &ctx.infer_ctx));
        }
    }
    if errors.is_empty() {
        for (i, ty) in ctx.bindings_ty.iter().enumerate() {
            if !ctx.assigned.get(i).copied().unwrap_or(false) {
                continue;
            }
            if let Some(t) = ty {
                if contains_infer_var(t) {
                    errors.push(SemanticError::new(
                        format!("could not resolve concrete type for local binding `{}`", ty_name(t)),
                        closing_span,
                    ));
                    break;
                }
            }
        }
    }
}

fn contains_type_param(ty: &Ty) -> bool {
    match ty {
        Ty::TypeParam(_) => true,
        Ty::Array(inner) => contains_type_param(inner.as_ref()),
        Ty::Tuple(parts) => parts.iter().any(contains_type_param),
        Ty::Enum { args, .. } => args.iter().any(contains_type_param),
        Ty::InferVar(_) => false,
        _ => false,
    }
}

fn contains_struct(ty: &Ty) -> bool {
    match ty {
        Ty::Struct(_) => true,
        Ty::Array(inner) => contains_struct(inner.as_ref()),
        Ty::Tuple(parts) => parts.iter().any(contains_struct),
        Ty::Enum { args, .. } => args.iter().any(contains_struct),
        Ty::InferVar(_) => false,
        _ => false,
    }
}

fn contains_infer_var(ty: &Ty) -> bool {
    match ty {
        Ty::InferVar(_) => true,
        Ty::Array(inner) => contains_infer_var(inner),
        Ty::Tuple(parts) => parts.iter().any(contains_infer_var),
        Ty::Enum { args, .. } => args.iter().any(contains_infer_var),
        Ty::Function { params, ret, .. } => {
            params.iter().any(contains_infer_var) || contains_infer_var(ret)
        }
        Ty::Task(inner) => contains_infer_var(inner),
        _ => false,
    }
}

fn type_matches_with_any_wildcards(expected: &Ty, got: &Ty) -> bool {
    match expected {
        Ty::Any => true,
        Ty::Array(e) => match got {
            Ty::Array(g) => type_matches_with_any_wildcards(e, g),
            _ => false,
        },
        Ty::Tuple(es) => match got {
            Ty::Tuple(gs) => {
                es.len() == gs.len()
                    && es
                        .iter()
                        .zip(gs.iter())
                        .all(|(e, g)| type_matches_with_any_wildcards(e, g))
            }
            _ => false,
        },
        Ty::Enum { name, args } => match got {
            Ty::Enum {
                name: gname,
                args: gargs,
            } => {
                name == gname
                    && args.len() == gargs.len()
                    && args
                        .iter()
                        .zip(gargs.iter())
                        .all(|(e, g)| type_matches_with_any_wildcards(e, g))
            }
            _ => false,
        },
        Ty::Function { params, ret, .. } => match got {
            Ty::Function {
                params: gparams,
                ret: gret,
                ..
            } => {
                params.len() == gparams.len()
                    && params
                        .iter()
                        .zip(gparams.iter())
                        .all(|(e, g)| type_matches_with_any_wildcards(e, g))
                    && type_matches_with_any_wildcards(ret, gret)
            }
            _ => false,
        },
        _ => expected == got,
    }
}

fn check_statement(
    stmt: &AstNode,
    ret_ty: Option<&Ty>,
    ctx: &mut BodyCtx<'_>,
    reachable: bool,
    errors: &mut Vec<SemanticError>,
) -> StmtFlow {
    if !reachable {
        return StmtFlow::Next(false);
    }
    match stmt {
        AstNode::SingleLineComment(_) | AstNode::MultiLineComment(_) => StmtFlow::Next(true),
        AstNode::Block { body, .. } => {
            ctx.push_scope();
            let mut r = true;
            for s in body {
                match check_statement(s, ret_ty, ctx, r, errors) {
                    StmtFlow::Next(rr) => r = rr,
                    StmtFlow::Break => {
                        ctx.pop_scope();
                        return StmtFlow::Break;
                    }
                    StmtFlow::Continue => {
                        ctx.pop_scope();
                        return StmtFlow::Continue;
                    }
                }
            }
            ctx.pop_scope();
            StmtFlow::Next(r)
        }
        AstNode::Let {
            pattern,
            type_annotation,
            initializer,
            ..
        } => {
            let ann_ty = type_annotation
                .as_ref()
                .and_then(|t| {
                    ty_from_type_expr(
                        t,
                        span_of_pattern(pattern),
                        errors,
                        &[],
                        &ctx.structs,
                        &ctx.enums,
                        ctx.aliases,
                    )
                });
            match initializer {
                Some(init) => {
                    let got_ty = match &ann_ty {
                        Some(expected) => {
                            check_expr_expected(init.as_ref(), ctx, errors, Some(expected))
                        }
                        None => check_expr(init.as_ref(), ctx, errors),
                    };
                    if let Some(got_ty) = got_ty {
                        let final_ty = match &ann_ty {
                            Some(a) => {
                                if *a != Ty::Any {
                                    let _ = infer::unify_types(
                                        a,
                                        &got_ty,
                                        &mut ctx.infer_ctx,
                                        errors,
                                        expr_span(init.as_ref()),
                                    );
                                }
                                a.clone()
                            }
                            None => got_ty,
                        };
                        let _ = declare_let_pattern(
                            pattern,
                            &final_ty,
                            Some(init.as_ref()),
                            true,
                            ctx,
                            errors,
                        );
                    }
                }
                None => {
                    if let Some(a) = ann_ty {
                        let _ = declare_let_pattern(pattern, &a, None, false, ctx, errors);
                    } else if let Pattern::Binding { name, name_span } = pattern {
                        // Deferred local inference:
                        // allow `let a;` and infer `a` from the first assignment.
                        ctx.declare_binding(name, *name_span, None, None, errors);
                    } else {
                        errors.push(SemanticError::new(
                            "`let` requires a type annotation when there is no initializer",
                            span_of_pattern(pattern),
                        ));
                    }
                }
            }
            StmtFlow::Next(true)
        }
        AstNode::Assign { pattern, value, .. } => {
            check_assign_pattern(pattern, value.as_ref(), ctx, errors);
            StmtFlow::Next(true)
        }
        AstNode::AssignExpr { lhs, rhs, .. } => {
            check_assign_expr(lhs.as_ref(), rhs.as_ref(), ctx, errors);
            StmtFlow::Next(true)
        }
        AstNode::CompoundAssign { lhs, op, rhs, .. } => {
            check_compound_assign(*op, lhs.as_ref(), rhs.as_ref(), ctx, errors);
            StmtFlow::Next(true)
        }
        AstNode::Call {
            callee,
            type_args,
            arguments,
            span,
        } => {
            let _ = check_call(callee, type_args, arguments, *span, ctx, errors);
            StmtFlow::Next(true)
        }
        AstNode::MethodCall { .. } | AstNode::TypeMethodCall { .. } => {
            let _ = check_expr(stmt, ctx, errors);
            StmtFlow::Next(true)
        }
        AstNode::Return { value, span } => {
            check_return(ret_ty, value.as_deref(), *span, ctx, errors);
            StmtFlow::Next(false)
        }
        AstNode::If {
            condition,
            then_body,
            else_body,
            ..
        } => {
            let ct = check_expr(condition.as_ref(), ctx, errors);
            if let Some(t) = ct {
                if t != Ty::Bool {
                    errors.push(SemanticError::new(
                        format!(
                            "`if` condition must be `Bool`, found `{}`",
                            ty_name(&t)
                        ),
                        expr_span(condition.as_ref()),
                    ));
                }
            }

            let base_len = ctx.bindings_ty.len();
            let entry_assigned: Vec<bool> = ctx.assigned.clone();

            ctx.push_scope();
            let mut r_then = reachable;
            for s in then_body {
                match check_statement(s, ret_ty, ctx, r_then, errors) {
                    StmtFlow::Next(rr) => r_then = rr,
                    f @ (StmtFlow::Break | StmtFlow::Continue) => {
                        ctx.pop_scope();
                        ctx.bindings_ty.truncate(base_len);
                        ctx.assigned.truncate(base_len);
                        for i in 0..base_len {
                            ctx.assigned[i] = entry_assigned[i];
                        }
                        return f;
                    }
                }
            }
            let end_then: Vec<bool> = ctx.assigned[0..base_len].to_vec();
            ctx.pop_scope();
            ctx.bindings_ty.truncate(base_len);
            ctx.assigned.truncate(base_len);

            for i in 0..base_len {
                ctx.assigned[i] = entry_assigned[i];
            }

            let (end_else, r_else) = if let Some(else_stmts) = else_body {
                ctx.push_scope();
                let mut r_e = reachable;
                for s in else_stmts {
                    match check_statement(s, ret_ty, ctx, r_e, errors) {
                        StmtFlow::Next(rr) => r_e = rr,
                        f @ (StmtFlow::Break | StmtFlow::Continue) => {
                            ctx.pop_scope();
                            ctx.bindings_ty.truncate(base_len);
                            ctx.assigned.truncate(base_len);
                            for i in 0..base_len {
                                ctx.assigned[i] = entry_assigned[i];
                            }
                            return f;
                        }
                    }
                }
                let ee: Vec<bool> = ctx.assigned[0..base_len].to_vec();
                ctx.pop_scope();
                ctx.bindings_ty.truncate(base_len);
                ctx.assigned.truncate(base_len);
                (ee, r_e)
            } else {
                (entry_assigned[0..base_len].to_vec(), reachable)
            };

            for i in 0..base_len {
                ctx.assigned[i] = end_then[i] && end_else[i];
            }

            let reach_after = if else_body.is_some() {
                r_then || r_else
            } else {
                true
            };
            StmtFlow::Next(reach_after)
        }
        AstNode::IfLet {
            pattern,
            value,
            then_body,
            else_body,
            ..
        } => {
            let vt = check_expr(value.as_ref(), ctx, errors);

            // For now, we only implement Rust-like "mismatch goes to `else`"
            // for enum-variant patterns (runtime tag check).
            // Tuple/array pattern mismatches would currently error during extraction.
            if matches!(pattern, Pattern::Tuple { .. } | Pattern::Array { .. }) {
                errors.push(SemanticError::new(
                    "`if let` tuple/array patterns are not supported yet",
                    span_of_pattern(pattern),
                ));
            }

            // If we can prove the `then` branch matches for the current static type,
            // treat `else` as unreachable for definite-assignment and reachability.
            let else_irrefutable = matches!(pattern, Pattern::Wildcard { .. } | Pattern::Binding { .. })
                || matches!(
                    (pattern, vt.as_ref()),
                    (Pattern::Struct { name, type_args, .. }, Some(Ty::Struct(n)))
                        if type_args.is_empty() && name == struct_base_name(n)
                );

            let base_len = ctx.bindings_ty.len();
            let entry_assigned: Vec<bool> = ctx.assigned.clone();

            // Then branch: pattern-bound names are visible and definitely assigned.
            ctx.push_scope();
            let mut r_then = reachable;
            if let Some(ty) = vt.as_ref() {
                let _ = declare_let_pattern(pattern, ty, Some(value.as_ref()), true, ctx, errors);
            }
            for s in then_body {
                match check_statement(s, ret_ty, ctx, r_then, errors) {
                    StmtFlow::Next(rr) => r_then = rr,
                    f @ (StmtFlow::Break | StmtFlow::Continue) => {
                        ctx.pop_scope();
                        ctx.bindings_ty.truncate(base_len);
                        ctx.assigned.truncate(base_len);
                        for i in 0..base_len {
                            ctx.assigned[i] = entry_assigned[i];
                        }
                        return f;
                    }
                }
            }
            let end_then: Vec<bool> = ctx.assigned[0..base_len].to_vec();
            ctx.pop_scope();
            ctx.bindings_ty.truncate(base_len);
            ctx.assigned.truncate(base_len);
            for i in 0..base_len {
                ctx.assigned[i] = entry_assigned[i];
            }

            // Else branch: pattern-bound names are not in scope.
            let (end_else, r_else) = if let Some(else_stmts) = else_body {
                ctx.push_scope();
                let mut r_e = reachable;
                for s in else_stmts {
                    match check_statement(s, ret_ty, ctx, r_e, errors) {
                        StmtFlow::Next(rr) => r_e = rr,
                        f @ (StmtFlow::Break | StmtFlow::Continue) => {
                            ctx.pop_scope();
                            ctx.bindings_ty.truncate(base_len);
                            ctx.assigned.truncate(base_len);
                            for i in 0..base_len {
                                ctx.assigned[i] = entry_assigned[i];
                            }
                            return f;
                        }
                    }
                }
                let ee: Vec<bool> = ctx.assigned[0..base_len].to_vec();
                ctx.pop_scope();
                ctx.bindings_ty.truncate(base_len);
                ctx.assigned.truncate(base_len);
                (ee, r_e)
            } else {
                (entry_assigned[0..base_len].to_vec(), reachable)
            };

            for i in 0..base_len {
                ctx.assigned[i] = if else_irrefutable {
                    end_then[i]
                } else {
                    end_then[i] && end_else[i]
                };
            }

            let reach_after = if else_irrefutable {
                r_then
            } else if else_body.is_some() {
                r_then || r_else
            } else {
                true
            };
            StmtFlow::Next(reach_after)
        }
        AstNode::Match { .. } => {
            let _ = check_expr(stmt, ctx, errors);
            StmtFlow::Next(true)
        }
        AstNode::While {
            condition,
            body,
            ..
        } => {
            check_while_statement(condition.as_ref(), body, ret_ty, ctx, reachable, errors);
            StmtFlow::Next(true)
        }
        AstNode::Break { .. } => {
            if ctx.loop_depth == 0 {
                errors.push(SemanticError::new(
                    "`break` outside of a loop",
                    span_of_item(stmt),
                ));
                StmtFlow::Next(true)
            } else {
                StmtFlow::Break
            }
        }
        AstNode::Continue { .. } => {
            if ctx.loop_depth == 0 {
                errors.push(SemanticError::new(
                    "`continue` outside of a loop",
                    span_of_item(stmt),
                ));
                StmtFlow::Next(true)
            } else {
                StmtFlow::Continue
            }
        }
        AstNode::Await { .. } => {
            let _ = check_expr(stmt, ctx, errors);
            StmtFlow::Next(true)
        }
        _ => {
            errors.push(SemanticError::new(
                "invalid statement in function body",
                span_of_item(stmt),
            ));
            StmtFlow::Next(true)
        }
    }
}

fn check_while_statement(
    condition: &AstNode,
    body: &[AstNode],
    ret_ty: Option<&Ty>,
    ctx: &mut BodyCtx<'_>,
    reachable: bool,
    errors: &mut Vec<SemanticError>,
) {
    if !reachable {
        return;
    }
    let ct = check_expr(condition, ctx, errors);
    if let Some(t) = ct {
        if t != Ty::Bool {
            errors.push(SemanticError::new(
                format!(
                    "`while` condition must be `Bool`, found `{}`",
                    ty_name(&t)
                ),
                expr_span(condition),
            ));
        }
    }

    let base_len = ctx.bindings_ty.len();
    let entry_assigned: Vec<bool> = ctx.assigned[0..base_len].to_vec();
    let literal_true = matches!(
        condition,
        AstNode::BoolLiteral { value: true, .. }
    );

    if literal_true {
        let mut hdr = entry_assigned.clone();
        const MAX_ITERS: usize = 64;
        for _ in 0..MAX_ITERS {
            for i in 0..base_len {
                ctx.assigned[i] = hdr[i];
            }
            ctx.push_scope();
            ctx.loop_depth += 1;
            let mut r = true;
            let mut exit_flow = StmtFlow::Next(true);
            for s in body {
                match check_statement(s, ret_ty, ctx, r, errors) {
                    StmtFlow::Next(rr) => r = rr,
                    StmtFlow::Break => {
                        exit_flow = StmtFlow::Break;
                        break;
                    }
                    StmtFlow::Continue => {
                        exit_flow = StmtFlow::Continue;
                        break;
                    }
                }
            }
            let exit: Vec<bool> = ctx.assigned[0..base_len].to_vec();
            ctx.loop_depth -= 1;
            ctx.pop_scope();
            ctx.bindings_ty.truncate(base_len);
            ctx.assigned.truncate(base_len);

            match exit_flow {
                StmtFlow::Break => {
                    for i in 0..base_len {
                        ctx.assigned[i] = entry_assigned[i] || exit[i];
                    }
                    return;
                }
                StmtFlow::Continue | StmtFlow::Next(_) => {
                    let next: Vec<bool> = (0..base_len).map(|i| hdr[i] || exit[i]).collect();
                    if next == hdr {
                        for i in 0..base_len {
                            ctx.assigned[i] = entry_assigned[i] || hdr[i];
                        }
                        return;
                    }
                    hdr = next;
                }
            }
        }
        for i in 0..base_len {
            ctx.assigned[i] = entry_assigned[i] || hdr[i];
        }
        return;
    }

    for i in 0..base_len {
        ctx.assigned[i] = entry_assigned[i];
    }
    ctx.push_scope();
    ctx.loop_depth += 1;
    let mut r = true;
    let mut exit_flow = StmtFlow::Next(true);
    for s in body {
        match check_statement(s, ret_ty, ctx, r, errors) {
            StmtFlow::Next(rr) => r = rr,
            StmtFlow::Break => {
                exit_flow = StmtFlow::Break;
                break;
            }
            StmtFlow::Continue => {
                exit_flow = StmtFlow::Continue;
                break;
            }
        }
    }
    let exit: Vec<bool> = ctx.assigned[0..base_len].to_vec();
    ctx.loop_depth -= 1;
    ctx.pop_scope();
    ctx.bindings_ty.truncate(base_len);
    ctx.assigned.truncate(base_len);

    match exit_flow {
        StmtFlow::Break => {
            for i in 0..base_len {
                ctx.assigned[i] = entry_assigned[i] || exit[i];
            }
        }
        StmtFlow::Continue | StmtFlow::Next(_) => {
            for i in 0..base_len {
                ctx.assigned[i] = entry_assigned[i] && exit[i];
            }
        }
    }
}

fn check_assign_expr(
    lhs: &AstNode,
    rhs: &AstNode,
    ctx: &mut BodyCtx<'_>,
    errors: &mut Vec<SemanticError>,
) {
    match lhs {
        AstNode::Identifier { name, span } => {
            let expected_owned: Option<Ty> = match ctx.lookup(name) {
                Some(NameRes::Local(id)) => ctx.bindings_ty[id].clone(),
                Some(NameRes::Global(ty)) => Some(ty),
                None => None,
            };
            let Some(got) = check_expr_expected(rhs, ctx, errors, expected_owned.as_ref()) else {
                return;
            };
            assign_pattern_types(
                &Pattern::Binding {
                    name: name.clone(),
                    name_span: *span,
                },
                &got,
                rhs,
                ctx,
                errors,
            );
        }
        AstNode::ArrayIndex { .. } => {
            let lt = check_expr(lhs, ctx, errors);
            let rt = check_expr(rhs, ctx, errors);
            if let (Some(lt), Some(rt)) = (lt, rt) {
                if lt != rt {
                    errors.push(SemanticError::new(
                        format!(
                            "type mismatch in array assignment (expected `{}`, found `{}`)",
                            ty_name(&lt),
                            ty_name(&rt)
                        ),
                        expr_span(lhs),
                    ));
                }
            }
        }
        AstNode::TupleField { span, .. } => {
            errors.push(SemanticError::new(
                "assignment to a tuple field (e.g. `t.0 = …`) is not supported",
                *span,
            ));
            let _ = check_expr(rhs, ctx, errors);
        }
        AstNode::FieldAccess { base, field, span } => {
            let lt = check_expr(base, ctx, errors);
            let rt = check_expr(rhs, ctx, errors);
            if let (Some(Ty::Struct(struct_name)), Some(rt_ty)) = (lt, rt) {
                let Some(def) = ctx.structs.get(struct_base_name(&struct_name)) else {
                    errors.push(SemanticError::new(
                        format!("unknown struct `{struct_name}`"),
                        *span,
                    ));
                    return;
                };
                let Some(expected_ty) = def.fields.get(field) else {
                    errors.push(SemanticError::new(
                        format!("unknown field `{field}` on struct `{struct_name}`"),
                        *span,
                    ));
                    return;
                };
                if expected_ty != &Ty::Any && expected_ty != &rt_ty {
                    errors.push(SemanticError::new(
                        format!(
                            "type mismatch in field assignment (expected `{}`, found `{}`)",
                            ty_name(expected_ty),
                            ty_name(&rt_ty)
                        ),
                        expr_span(lhs),
                    ));
                }
            }
        }
        _ => {
            errors.push(SemanticError::new(
                "invalid assignment target",
                expr_span(lhs),
            ));
            let _ = check_expr(rhs, ctx, errors);
        }
    }
}

fn check_compound_assign(
    op: CompoundOp,
    lhs: &AstNode,
    rhs: &AstNode,
    ctx: &mut BodyCtx<'_>,
    errors: &mut Vec<SemanticError>,
) {
    let method = match op {
        CompoundOp::Add => Some("binary_add"),
        CompoundOp::Sub => Some("binary_sub"),
        CompoundOp::Mul => Some("binary_mul"),
        CompoundOp::Div => Some("binary_div"),
        CompoundOp::Mod => Some("binary_mod"),
        CompoundOp::BitAnd => Some("binary_bitwise_and"),
        CompoundOp::BitXor => Some("binary_bitwise_xor"),
        CompoundOp::BitOr => Some("binary_bitwise_or"),
        CompoundOp::ShiftLeft => Some("binary_left_shift"),
        CompoundOp::ShiftRight => Some("binary_right_shift"),
    };
    match lhs {
        AstNode::TupleField { span, .. } => {
            errors.push(SemanticError::new(
                "compound assignment to a tuple field is not supported",
                *span,
            ));
            let _ = check_expr(rhs, ctx, errors);
            return;
        }
        AstNode::Identifier { name, span } => {
            let lt = check_expr(lhs, ctx, errors).map(|t| infer::resolve_ty(&t, &ctx.infer_ctx));
            let rt = check_expr(rhs, ctx, errors).map(|t| infer::resolve_ty(&t, &ctx.infer_ctx));
            let arith_allows_float = matches!(op, CompoundOp::Add | CompoundOp::Sub | CompoundOp::Mul | CompoundOp::Div | CompoundOp::Mod);
            let method = method.expect("all compound ops mapped");

            // Avoid spurious type-mismatch errors by only attempting unification
            // when operand types are compatible with the target.
            match (&lt, &rt) {
                (Some(Ty::Int), Some(Ty::Int)) => {
                    if !ctx.registry.contains_key(&format!("Int::{method}")) {
                        errors.push(SemanticError::new(
                            format!(
                                "operator is not available: missing `internal func Int::{method}(...)`"
                            ),
                            *span,
                        ));
                        return;
                    }
                    assign_pattern_types(
                        &Pattern::Binding {
                            name: name.clone(),
                            name_span: *span,
                        },
                        &Ty::Int,
                        rhs,
                        ctx,
                        errors,
                    );
                }
                (Some(l), Some(r))
                    if matches!(l, Ty::InferVar(_)) && matches!(r, Ty::Int) => {
                    if !ctx.registry.contains_key(&format!("Int::{method}")) {
                        errors.push(SemanticError::new(
                            format!(
                                "operator is not available: missing `internal func Int::{method}(...)`"
                            ),
                            *span,
                        ));
                        return;
                    }
                    let _ = infer::unify_types(
                        l,
                        &Ty::Int,
                        &mut ctx.infer_ctx,
                        errors,
                        *span,
                    ) && infer::unify_types(
                        r,
                        &Ty::Int,
                        &mut ctx.infer_ctx,
                        errors,
                        *span,
                    );
                    assign_pattern_types(
                        &Pattern::Binding {
                            name: name.clone(),
                            name_span: *span,
                        },
                        &Ty::Int,
                        rhs,
                        ctx,
                        errors,
                    );
                }
                (Some(l), Some(r))
                    if matches!(l, Ty::Int) && matches!(r, Ty::InferVar(_)) => {
                    if !ctx.registry.contains_key(&format!("Int::{method}")) {
                        errors.push(SemanticError::new(
                            format!(
                                "operator is not available: missing `internal func Int::{method}(...)`"
                            ),
                            *span,
                        ));
                        return;
                    }
                    let _ = infer::unify_types(
                        l,
                        &Ty::Int,
                        &mut ctx.infer_ctx,
                        errors,
                        *span,
                    ) && infer::unify_types(
                        r,
                        &Ty::Int,
                        &mut ctx.infer_ctx,
                        errors,
                        *span,
                    );
                    assign_pattern_types(
                        &Pattern::Binding {
                            name: name.clone(),
                            name_span: *span,
                        },
                        &Ty::Int,
                        rhs,
                        ctx,
                        errors,
                    );
                }
                (Some(l), Some(r)) if matches!(l, Ty::InferVar(_)) && matches!(r, Ty::InferVar(_)) => {
                    // Arbitrary default for fully-inferred arithmetic.
                    if ctx.registry.contains_key(&format!("Int::{method}")) {
                        let _ = infer::unify_types(
                            l,
                            &Ty::Int,
                            &mut ctx.infer_ctx,
                            errors,
                            *span,
                        ) && infer::unify_types(
                            r,
                            &Ty::Int,
                            &mut ctx.infer_ctx,
                            errors,
                            *span,
                        );
                        assign_pattern_types(
                            &Pattern::Binding {
                                name: name.clone(),
                                name_span: *span,
                            },
                            &Ty::Int,
                            rhs,
                            ctx,
                            errors,
                        );
                    } else if arith_allows_float && ctx.registry.contains_key(&format!("Float::{method}")) {
                        let _ = infer::unify_types(
                            l,
                            &Ty::Float,
                            &mut ctx.infer_ctx,
                            errors,
                            *span,
                        ) && infer::unify_types(
                            r,
                            &Ty::Float,
                            &mut ctx.infer_ctx,
                            errors,
                            *span,
                        );
                        assign_pattern_types(
                            &Pattern::Binding {
                                name: name.clone(),
                                name_span: *span,
                            },
                            &Ty::Float,
                            rhs,
                            ctx,
                            errors,
                        );
                    } else {
                        errors.push(SemanticError::new(
                            "operator is not available: missing required `internal func` operator implementation",
                            *span,
                        ));
                        return;
                    }
                }
                (Some(Ty::Float), Some(Ty::Float)) if arith_allows_float => {
                    if !ctx.registry.contains_key(&format!("Float::{method}")) {
                        errors.push(SemanticError::new(
                            format!(
                                "operator is not available: missing `internal func Float::{method}(...)`"
                            ),
                            *span,
                        ));
                        return;
                    }
                    assign_pattern_types(
                        &Pattern::Binding {
                            name: name.clone(),
                            name_span: *span,
                        },
                        &Ty::Float,
                        rhs,
                        ctx,
                        errors,
                    );
                }
                (Some(l), Some(r))
                    if arith_allows_float
                        && matches!(l, Ty::InferVar(_))
                        && matches!(r, Ty::Float) => {
                    if !ctx.registry.contains_key(&format!("Float::{method}")) {
                        errors.push(SemanticError::new(
                            format!(
                                "operator is not available: missing `internal func Float::{method}(...)`"
                            ),
                            *span,
                        ));
                        return;
                    }
                    let _ = infer::unify_types(
                        l,
                        &Ty::Float,
                        &mut ctx.infer_ctx,
                        errors,
                        *span,
                    ) && infer::unify_types(
                        r,
                        &Ty::Float,
                        &mut ctx.infer_ctx,
                        errors,
                        *span,
                    );
                    assign_pattern_types(
                        &Pattern::Binding {
                            name: name.clone(),
                            name_span: *span,
                        },
                        &Ty::Float,
                        rhs,
                        ctx,
                        errors,
                    );
                }
                (Some(l), Some(r))
                    if arith_allows_float
                        && matches!(l, Ty::Float)
                        && matches!(r, Ty::InferVar(_)) => {
                    if !ctx.registry.contains_key(&format!("Float::{method}")) {
                        errors.push(SemanticError::new(
                            format!(
                                "operator is not available: missing `internal func Float::{method}(...)`"
                            ),
                            *span,
                        ));
                        return;
                    }
                    let _ = infer::unify_types(
                        l,
                        &Ty::Float,
                        &mut ctx.infer_ctx,
                        errors,
                        *span,
                    ) && infer::unify_types(
                        r,
                        &Ty::Float,
                        &mut ctx.infer_ctx,
                        errors,
                        *span,
                    );
                    assign_pattern_types(
                        &Pattern::Binding {
                            name: name.clone(),
                            name_span: *span,
                        },
                        &Ty::Float,
                        rhs,
                        ctx,
                        errors,
                    );
                }
                _ if arith_allows_float => {
                    errors.push(SemanticError::new(
                        "compound assignment requires `Int` or `Float` operands of the same type",
                        *span,
                    ));
                }
                _ => {
                    errors.push(SemanticError::new(
                        "compound assignment requires `Int` operands",
                        *span,
                    ));
                }
            }
        }
        _ => {
            errors.push(SemanticError::new(
                "compound assignment requires a simple variable name on the left",
                expr_span(lhs),
            ));
            let _ = check_expr(rhs, ctx, errors);
        }
    }
}

fn span_of_pattern(p: &Pattern) -> Span {
    match p {
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

fn pattern_is_irrefutable(p: &Pattern) -> bool {
    match p {
        Pattern::Wildcard { .. } | Pattern::Binding { .. } => true,
        Pattern::IntLiteral { .. } | Pattern::StringLiteral { .. } | Pattern::BoolLiteral { .. } => false,
        Pattern::Tuple { elements, .. } | Pattern::Array { elements, .. } => elements.iter().all(|e| match e {
            PatternElem::Rest(_) => true,
            PatternElem::Pattern(sub) => pattern_is_irrefutable(sub),
        }),
        Pattern::Struct { fields, .. } => fields.iter().all(|f| pattern_is_irrefutable(&f.pattern)),
        Pattern::EnumVariant { payloads, .. } => payloads.iter().all(pattern_is_irrefutable),
    }
}

fn declare_let_pattern_elem(
    elem: &PatternElem,
    subty: &Ty,
    _init: Option<&AstNode>,
    from_value: bool,
    ctx: &mut BodyCtx<'_>,
    errors: &mut Vec<SemanticError>,
) -> bool {
    match elem {
        PatternElem::Rest(s) => {
            errors.push(SemanticError::new(
                "internal error: `..` in tuple pattern slice",
                *s,
            ));
            false
        }
        PatternElem::Pattern(Pattern::Wildcard { .. }) => true,
        PatternElem::Pattern(Pattern::Binding { name, name_span }) => {
            if from_value {
                ctx.declare_binding_destructure(name, *name_span, subty.clone(), errors);
            } else {
                ctx.declare_binding(
                    name,
                    *name_span,
                    Some(subty.clone()),
                    None,
                    errors,
                );
            }
            true
        }
        PatternElem::Pattern(p) => declare_let_pattern(p, subty, None, from_value, ctx, errors),
    }
}

fn declare_let_pattern(
    pattern: &Pattern,
    ty: &Ty,
    init: Option<&AstNode>,
    from_value: bool,
    ctx: &mut BodyCtx<'_>,
    errors: &mut Vec<SemanticError>,
) -> bool {
    match pattern {
        Pattern::Wildcard { .. } => true,
        Pattern::IntLiteral { value: _, span, .. } => {
            if ty != &Ty::Int {
                errors.push(SemanticError::new(
                    format!("int literal pattern requires `Int` but got `{}`", ty_name(ty)),
                    *span,
                ));
                return false;
            }
            true
        }
        Pattern::StringLiteral { value: _, span } => {
            if ty != &Ty::String {
                errors.push(SemanticError::new(
                    format!("string literal pattern requires `String` but got `{}`", ty_name(ty)),
                    *span,
                ));
                return false;
            }
            true
        }
        Pattern::BoolLiteral { value: _, span } => {
            if ty != &Ty::Bool {
                errors.push(SemanticError::new(
                    format!("bool literal pattern requires `Bool` but got `{}`", ty_name(ty)),
                    *span,
                ));
                return false;
            }
            true
        }
        Pattern::Binding { name, name_span } => {
            if from_value {
                ctx.declare_binding_destructure(name, *name_span, ty.clone(), errors);
            } else {
                ctx.declare_binding(
                    name,
                    *name_span,
                    Some(ty.clone()),
                    init,
                    errors,
                );
            }
            true
        }
        Pattern::Tuple { elements, span } => {
            let Ty::Tuple(parts) = ty else {
                errors.push(SemanticError::new(
                    "tuple pattern requires a tuple value",
                    *span,
                ));
                return false;
            };
            let n = parts.len();
            let Some((prefix, suffix, has_rest)) =
                tuple_pattern_prefix_suffix(elements, *span, errors)
            else {
                return false;
            };
            let fp = prefix.len();
            let fs = suffix.len();
            if has_rest {
                if fp + fs > n {
                    errors.push(SemanticError::new(
                        format!(
                            "tuple pattern requires at least {} fixed slot(s), but value has only {} element(s)",
                            fp + fs,
                            n
                        ),
                        *span,
                    ));
                    return false;
                }
                for (i, e) in prefix.iter().enumerate() {
                    if !declare_let_pattern_elem(e, &parts[i], init, from_value, ctx, errors) {
                        return false;
                    }
                }
                for (j, e) in suffix.iter().enumerate() {
                    let idx = n - fs + j;
                    if !declare_let_pattern_elem(e, &parts[idx], init, from_value, ctx, errors) {
                        return false;
                    }
                }
            } else if fp != n {
                errors.push(SemanticError::new(
                    format!(
                        "tuple pattern has {} slot(s) but value has {} element(s)",
                        fp,
                        n
                    ),
                    *span,
                ));
                return false;
            } else {
                for (e, subty) in prefix.iter().zip(parts.iter()) {
                    if !declare_let_pattern_elem(e, subty, init, from_value, ctx, errors) {
                        return false;
                    }
                }
            }
            true
        }
        Pattern::Array { elements, span } => {
            let Ty::Array(elem_ty) = ty else {
                errors.push(SemanticError::new(
                    "array pattern requires an array value",
                    *span,
                ));
                return false;
            };

            let rest_count = elements
                .iter()
                .filter(|e| matches!(e, PatternElem::Rest(_)))
                .count();
            if rest_count > 1 {
                errors.push(SemanticError::new(
                    "multiple `..` in array pattern",
                    *span,
                ));
                return false;
            }

            for e in elements {
                if matches!(e, PatternElem::Rest(_)) {
                    continue;
                }
                if !declare_let_pattern_elem(e, elem_ty, init, from_value, ctx, errors) {
                    return false;
                }
            }
            true
        }
        Pattern::Struct {
            name,
            type_args,
            fields,
            rest,
            span,
            ..
        } => {
            let Ty::Struct(expected_name) = ty else {
                errors.push(SemanticError::new(
                    "struct pattern requires a struct value",
                    *span,
                ));
                return false;
            };
            if struct_base_name(expected_name) != name {
                errors.push(SemanticError::new(
                    format!("struct pattern for `{name}` used on `{expected_name}` value"),
                    *span,
                ));
                return false;
            }

            let Some(def) = ctx.structs.get(name) else {
                errors.push(SemanticError::new(
                    format!("unknown struct `{name}`"),
                    *span,
                ));
                return false;
            };

            if type_args.len() != def.type_params.len() && !type_args.is_empty() {
                errors.push(SemanticError::new(
                    format!(
                        "struct `{name}` expects {} type argument(s), found {}",
                        def.type_params.len(),
                        type_args.len()
                    ),
                    *span,
                ));
                return false;
            }
            if !type_args.is_empty() && fields.is_empty() && rest.is_none() && !def.is_unit {
                errors.push(SemanticError::new(
                    format!("`{name}<...>` pattern without fields is only valid for unit structs"),
                    *span,
                ));
                return false;
            }
            if !type_args.is_empty() {
                let mut rendered_args: Vec<String> = Vec::with_capacity(type_args.len());
                for arg in type_args {
                    let Some(arg_ty) =
                        ty_from_type_expr(arg, *span, errors, &[], &ctx.structs, &ctx.enums, ctx.aliases)
                    else {
                        return false;
                    };
                    rendered_args.push(ty_name(&arg_ty));
                }
                let pattern_full_name = format!("{name}<{}>", rendered_args.join(", "));
                if expected_name != &pattern_full_name {
                    typecheck::report_generic_struct_pattern_mismatch(
                        expected_name,
                        &pattern_full_name,
                        *span,
                        errors,
                    );
                    return false;
                }
            }

            // Strict field coverage unless `..` is present.
            if rest.is_none() && fields.len() != def.fields.len() {
                errors.push(SemanticError::new(
                    format!(
                        "struct pattern `{name}` must list all fields (expected {}, found {})",
                        def.fields.len(),
                        fields.len()
                    ),
                    *span,
                ));
                return false;
            }

            let mut seen: HashMap<&str, Span> = HashMap::new();
            for f in fields {
                if seen
                    .insert(f.name.as_str(), f.name_span)
                    .is_some()
                {
                    errors.push(SemanticError::new(
                        format!("duplicate field `{}` in struct pattern", f.name),
                        f.name_span,
                    ));
                    return false;
                }
                if !def.fields.contains_key(&f.name) {
                    errors.push(SemanticError::new(
                        format!("unknown field `{}` in struct `{name}`", f.name),
                        f.name_span,
                    ));
                    return false;
                }
            }

            if rest.is_none() {
                for field_name in def.fields.keys() {
                    if !seen.contains_key(field_name.as_str()) {
                        errors.push(SemanticError::new(
                            format!(
                                "missing field `{}` in struct pattern `{name}`",
                                field_name
                            ),
                            *span,
                        ));
                        return false;
                    }
                }
            }

            let struct_substs = match struct_type_substs_for_concrete_name(
                expected_name,
                def,
                *span,
                errors,
                &ctx.structs,
                &ctx.enums,
                ctx.aliases,
            ) {
                Some(s) => s,
                None => return false,
            };
            for f in fields {
                let expected_field_ty = def.fields.get(&f.name).unwrap();
                let expected_field_ty = infer::instantiate_ty(expected_field_ty, &struct_substs);
                // When destructuring from a value, binding leaves are definitely assigned.
                if from_value {
                    if let Pattern::Binding {
                        name: bname,
                        name_span: bspan,
                    } = &f.pattern
                    {
                        ctx.declare_binding_destructure(bname, *bspan, expected_field_ty, errors);
                        continue;
                    }
                }
                if !declare_let_pattern(
                    &f.pattern,
                    &expected_field_ty,
                    init,
                    from_value,
                    ctx,
                    errors,
                ) {
                    return false;
                }
            }
            true
        }
        Pattern::EnumVariant {
            enum_name,
            variant,
            payloads,
            type_args,
            span,
            ..
        } => {
            let Ty::Enum {
                name: expected_name,
                args: expected_args,
            } = ty
            else {
                errors.push(SemanticError::new(
                    "enum pattern requires an enum value",
                    *span,
                ));
                return false;
            };

            if expected_name != enum_name {
                errors.push(SemanticError::new(
                    format!("enum pattern for `{enum_name}` used on `{expected_name}` value"),
                    *span,
                ));
                return false;
            }

            let Some(def) = ctx.enums.get(enum_name) else {
                errors.push(SemanticError::new(
                    format!("unknown enum `{enum_name}`"),
                    *span,
                ));
                return false;
            };

            let Some(payload_templates) = def.variants.get(variant) else {
                errors.push(SemanticError::new(
                    format!("unknown variant `{variant}` for enum `{enum_name}`"),
                    *span,
                ));
                return false;
            };

            if payload_templates.len() != payloads.len() {
                errors.push(SemanticError::new(
                    format!(
                        "enum variant `{enum_name}::{variant}` expects {} payload(s), found {}",
                        payload_templates.len(),
                        payloads.len()
                    ),
                    *span,
                ));
                return false;
            }

            // Resolve the enum generic args from the pattern's explicit type args
            // (or `_` / empty meaning "infer from scrutinee context").
            let resolved_args: Vec<Ty> = if type_args.is_empty() {
                expected_args.clone()
            } else {
                if type_args.len() != def.type_params.len() {
                    errors.push(SemanticError::new(
                        format!(
                            "enum `{enum_name}` expects {} type argument(s) in pattern, found {}",
                            def.type_params.len(),
                            type_args.len()
                        ),
                        *span,
                    ));
                    return false;
                }
                let mut out = Vec::with_capacity(type_args.len());
                for (i, tpl) in type_args.iter().enumerate() {
                    match tpl {
                        TypeExpr::Infer => out.push(expected_args[i].clone()),
                        other => {
                            let Some(got) = ty_from_type_expr(
                                other,
                                *span,
                                errors,
                                &def.type_params,
                                &ctx.structs,
                                &ctx.enums,
                                ctx.aliases,
                            ) else {
                                return false;
                            };
                            if got != expected_args[i] {
                                errors.push(SemanticError::new(
                                    format!(
                                        "enum pattern type argument mismatch in `{enum_name}`: expected `{}`, found `{}`",
                                        ty_name(&expected_args[i]),
                                        ty_name(&got)
                                    ),
                                    *span,
                                ));
                                return false;
                            }
                            out.push(got);
                        }
                    }
                }
                out
            };

            if payload_templates.len() == 0 {
                return true;
            }

            let mut substs: HashMap<String, Ty> = HashMap::new();
            for (tp, arg_ty) in def.type_params.iter().zip(resolved_args.iter()) {
                substs.insert(tp.name.clone(), arg_ty.clone());
            }

            for (tpl, pat) in payload_templates.iter().zip(payloads.iter()) {
                let Some(tpl_ty) = ty_from_type_expr(
                    tpl,
                    *span,
                    errors,
                    &def.type_params,
                    &ctx.structs,
                    &ctx.enums,
                    ctx.aliases,
                ) else {
                    return false;
                };
                let expected_payload_ty = infer::instantiate_ty(&tpl_ty, &substs);
                if !declare_let_pattern(pat, &expected_payload_ty, init, from_value, ctx, errors)
                {
                    return false;
                }
            }
            true
        }
    }
}

fn check_assign_pattern(
    pattern: &Pattern,
    value: &AstNode,
    ctx: &mut BodyCtx<'_>,
    errors: &mut Vec<SemanticError>,
) {
    let expected_owned: Option<Ty> = match pattern {
        Pattern::Binding { name, .. } => match ctx.lookup(name) {
            Some(NameRes::Local(id)) => ctx.bindings_ty[id].clone(),
            Some(NameRes::Global(ty)) => Some(ty),
            None => None,
        },
        _ => None,
    };
    let Some(got) = check_expr_expected(value, ctx, errors, expected_owned.as_ref()) else {
        return;
    };
    assign_pattern_types(pattern, &got, value, ctx, errors);
}

fn assign_pattern_elem(
    elem: &PatternElem,
    subty: &Ty,
    value: &AstNode,
    ctx: &mut BodyCtx<'_>,
    errors: &mut Vec<SemanticError>,
) {
    match elem {
        PatternElem::Rest(s) => {
            errors.push(SemanticError::new(
                "internal error: `..` in tuple pattern slice",
                *s,
            ));
        }
        PatternElem::Pattern(p) => assign_pattern_types(p, subty, value, ctx, errors),
    }
}

fn assign_pattern_types(
    pattern: &Pattern,
    ty: &Ty,
    value: &AstNode,
    ctx: &mut BodyCtx<'_>,
    errors: &mut Vec<SemanticError>,
) {
    match pattern {
        Pattern::Wildcard { .. } => {}
        Pattern::IntLiteral { span, .. }
        | Pattern::StringLiteral { span, .. }
        | Pattern::BoolLiteral { span, .. } => {
            errors.push(SemanticError::new(
                "assignment patterns cannot contain literal patterns",
                *span,
            ));
        }
        Pattern::Binding { name, name_span } => match ctx.lookup(name) {
            None => errors.push(SemanticError::new(
                format!("unknown identifier `{name}`"),
                *name_span,
            )),
            Some(NameRes::Local(id)) => {
                if unify_binding(ctx, id, ty.clone(), expr_span(value), errors) {
                    ctx.assigned[id] = true;
                }
            }
            Some(NameRes::Global(ety)) => {
                if ty != &ety {
                    errors.push(SemanticError::new(
                        format!(
                            "type mismatch: expected `{}`, found `{}`",
                            ty_name(&ety),
                            ty_name(ty)
                        ),
                        *name_span,
                    ));
                }
            }
        },
        Pattern::Tuple { elements, span } => {
            let Ty::Tuple(parts) = ty else {
                errors.push(SemanticError::new(
                    "tuple assignment pattern requires a tuple value",
                    *span,
                ));
                return;
            };
            let n = parts.len();
            let Some((prefix, suffix, has_rest)) =
                tuple_pattern_prefix_suffix(elements, *span, errors)
            else {
                return;
            };
            let fp = prefix.len();
            let fs = suffix.len();
            if has_rest {
                if fp + fs > n {
                    errors.push(SemanticError::new(
                        format!(
                            "tuple assignment pattern requires at least {} fixed slot(s), but value has only {} element(s)",
                            fp + fs,
                            n
                        ),
                        *span,
                    ));
                    return;
                }
                for (i, e) in prefix.iter().enumerate() {
                    assign_pattern_elem(e, &parts[i], value, ctx, errors);
                }
                for (j, e) in suffix.iter().enumerate() {
                    let idx = n - fs + j;
                    assign_pattern_elem(e, &parts[idx], value, ctx, errors);
                }
            } else if fp != n {
                errors.push(SemanticError::new(
                    format!(
                        "tuple pattern has {} slot(s) but value has {} element(s)",
                        fp,
                        n
                    ),
                    *span,
                ));
            } else {
                for (e, subty) in prefix.iter().zip(parts.iter()) {
                    assign_pattern_elem(e, subty, value, ctx, errors);
                }
            }
        }
        Pattern::Array { elements, span } => {
            let Ty::Array(elem_ty) = ty else {
                errors.push(SemanticError::new(
                    "array assignment pattern requires an array value",
                    *span,
                ));
                return;
            };

            let rest_count = elements
                .iter()
                .filter(|e| matches!(e, PatternElem::Rest(_)))
                .count();
            if rest_count > 1 {
                errors.push(SemanticError::new(
                    "multiple `..` in array assignment pattern",
                    *span,
                ));
                return;
            }

            for e in elements {
                if matches!(e, PatternElem::Rest(_)) {
                    continue;
                }
                assign_pattern_elem(e, elem_ty, value, ctx, errors);
            }
        }
        Pattern::Struct {
            name,
            type_args,
            fields,
            rest,
            span,
            ..
        } => {
            let Ty::Struct(expected_name) = ty else {
                errors.push(SemanticError::new(
                    "struct assignment pattern requires a struct value",
                    *span,
                ));
                return;
            };
            if struct_base_name(expected_name) != name {
                errors.push(SemanticError::new(
                    format!("struct pattern for `{name}` used on `{expected_name}` value"),
                    *span,
                ));
                return;
            }

            let Some(def) = ctx.structs.get(name) else {
                errors.push(SemanticError::new(
                    format!("unknown struct `{name}`"),
                    *span,
                ));
                return;
            };

            if type_args.len() != def.type_params.len() && !type_args.is_empty() {
                errors.push(SemanticError::new(
                    format!(
                        "struct `{name}` expects {} type argument(s), found {}",
                        def.type_params.len(),
                        type_args.len()
                    ),
                    *span,
                ));
                return;
            }
            if !type_args.is_empty() && fields.is_empty() && rest.is_none() && !def.is_unit {
                errors.push(SemanticError::new(
                    format!("`{name}<...>` pattern without fields is only valid for unit structs"),
                    *span,
                ));
                return;
            }
            if !type_args.is_empty() {
                let mut rendered_args: Vec<String> = Vec::with_capacity(type_args.len());
                for arg in type_args {
                    let Some(arg_ty) =
                        ty_from_type_expr(arg, *span, errors, &[], &ctx.structs, &ctx.enums, ctx.aliases)
                    else {
                        return;
                    };
                    rendered_args.push(ty_name(&arg_ty));
                }
                let pattern_full_name = format!("{name}<{}>", rendered_args.join(", "));
                if expected_name != &pattern_full_name {
                    typecheck::report_generic_struct_pattern_mismatch(
                        expected_name,
                        &pattern_full_name,
                        *span,
                        errors,
                    );
                    return;
                }
            }

            if rest.is_none() && fields.len() != def.fields.len() {
                errors.push(SemanticError::new(
                    format!(
                        "struct assignment pattern `{name}` must list all fields (expected {}, found {})",
                        def.fields.len(),
                        fields.len()
                    ),
                    *span,
                ));
                return;
            }

            let mut seen: HashMap<&str, Span> = HashMap::new();
            for f in fields {
                if seen
                    .insert(f.name.as_str(), f.name_span)
                    .is_some()
                {
                    errors.push(SemanticError::new(
                        format!("duplicate field `{}` in struct assignment pattern", f.name),
                        f.name_span,
                    ));
                    return;
                }
                if !def.fields.contains_key(&f.name) {
                    errors.push(SemanticError::new(
                        format!("unknown field `{}` in struct `{name}`", f.name),
                        f.name_span,
                    ));
                    return;
                }
            }

            if rest.is_none() {
                for field_name in def.fields.keys() {
                    if !seen.contains_key(field_name.as_str()) {
                        errors.push(SemanticError::new(
                            format!(
                                "missing field `{}` in struct assignment pattern `{name}`",
                                field_name
                            ),
                            *span,
                        ));
                        return;
                    }
                }
            }

            let struct_substs = match struct_type_substs_for_concrete_name(
                expected_name,
                def,
                *span,
                errors,
                &ctx.structs,
                &ctx.enums,
                ctx.aliases,
            ) {
                Some(s) => s,
                None => return,
            };
            for f in fields {
                let expected_field_ty = def.fields.get(&f.name).unwrap();
                let expected_field_ty = infer::instantiate_ty(expected_field_ty, &struct_substs);
                assign_pattern_types(
                    &f.pattern,
                    &expected_field_ty,
                    value,
                    ctx,
                    errors,
                );
            }
        }
        Pattern::EnumVariant {
            enum_name,
            variant,
            payloads,
            span,
            ..
        } => {
            let Ty::Enum {
                name: expected_name,
                args: expected_args,
            } = ty
            else {
                errors.push(SemanticError::new(
                    "enum assignment pattern requires an enum value",
                    *span,
                ));
                return;
            };

            if expected_name != enum_name {
                errors.push(SemanticError::new(
                    format!(
                        "enum pattern for `{enum_name}` used on `{expected_name}` value"
                    ),
                    *span,
                ));
                return;
            }

            let Some(def) = ctx.enums.get(enum_name) else {
                errors.push(SemanticError::new(
                    format!("unknown enum `{enum_name}`"),
                    *span,
                ));
                return;
            };
            let Some(payload_templates) = def.variants.get(variant) else {
                errors.push(SemanticError::new(
                    format!("unknown variant `{variant}` for enum `{enum_name}`"),
                    *span,
                ));
                return;
            };

            if payload_templates.len() != payloads.len() {
                errors.push(SemanticError::new(
                    format!(
                        "enum variant `{enum_name}::{variant}` expects {} payload(s), found {}",
                        payload_templates.len(),
                        payloads.len()
                    ),
                    *span,
                ));
                return;
            }

            let substs: HashMap<String, Ty> = def
                .type_params
                .iter()
                .zip(expected_args.iter())
                .map(|(tp, ty)| (tp.name.clone(), ty.clone()))
                .collect();

            for (tpl, pat) in payload_templates.iter().zip(payloads.iter()) {
                let tpl_ty = match ty_from_type_expr(
                    tpl,
                    *span,
                    errors,
                    &def.type_params,
                    &ctx.structs,
                    &ctx.enums,
                    ctx.aliases,
                ) {
                    Some(t) => t,
                    None => return,
                };
                let expected_payload_ty = infer::instantiate_ty(&tpl_ty, &substs);
                assign_pattern_types(pat, &expected_payload_ty, value, ctx, errors);
            }
        }
    }
}

fn check_return(
    ret_ty: Option<&Ty>,
    value: Option<&AstNode>,
    span: Span,
    ctx: &mut BodyCtx<'_>,
    errors: &mut Vec<SemanticError>,
) {
    match (ret_ty, value) {
        (None, None) => {}
        (None, Some(boxed)) => {
            let got = check_expr(boxed, ctx, errors);
            if got != Some(Ty::Unit) {
                errors.push(SemanticError::new(
                    "`return` with a value in a function with no return type (only `return ();` is allowed)",
                    span,
                ));
            }
        }
        (Some(ety), None) => {
            // In non-async functions returning `Task<T>`, `return;` must be a `Unit`-typed return
            // (i.e. only `: Unit` functions can omit an explicit value).
            // In `async func` bodies returning `Task<T>`, `return;` can only omit a value when the
            // payload type `T` is `Unit`.
            let unit_ok = match ety {
                Ty::Unit => true,
                Ty::Task(inner) => ctx.in_async && **inner == Ty::Unit,
                _ => false,
            };
            if !unit_ok {
                errors.push(SemanticError::new(
                    "`return` with no value in a function that expects a non-unit return",
                    span,
                ));
            }
        }
        (Some(ety), Some(boxed)) => {
            // Language rule:
            // - `async func ...: Task<T>`: `return expr;` returns a payload `T` (not `Task<T>`).
            // - non-`async func ...: Task<T>`: `return expr;` returns a value of type `Task<T>`.
            let expected = match (ety, ctx.in_async) {
                (Ty::Task(inner), true) => Some(inner.as_ref()),
                _ => Some(ety),
            };
            let got = if let Some(exp) = expected {
                check_expr_expected(boxed, ctx, errors, Some(exp))
            } else {
                check_expr(boxed, ctx, errors)
            };
            let unify_target: &Ty = match (ety, ctx.in_async) {
                (Ty::Task(inner), true) => inner.as_ref(),
                _ => ety,
            };
            match got {
                Some(g) if *unify_target != Ty::Any => {
                    if !type_matches_with_any_wildcards(unify_target, &g) {
                        let _ =
                            infer::unify_types(unify_target, &g, &mut ctx.infer_ctx, errors, span);
                    }
                }
                Some(_) => {}
                None => {
                    if let AstNode::Call {
                        callee,
                        span: cspan,
                        ..
                    } = boxed
                    {
                        if ctx
                            .registry
                            .get(callee)
                            .is_some_and(|s| s.ret.is_none())
                        {
                            errors.push(SemanticError::new(
                                format!(
                                    "function `{callee}` does not return a value but a return type was expected"
                                ),
                                *cspan,
                            ));
                        }
                    }
                }
            }
        }
    }
}

fn struct_type_substs_for_concrete_name(
    concrete_name: &str,
    def: &StructDef,
    span: Span,
    errors: &mut Vec<SemanticError>,
    structs: &HashMap<String, StructDef>,
    enums: &HashMap<String, EnumDef>,
    aliases: &HashMap<String, AliasDef>,
) -> Option<HashMap<String, Ty>> {
    if def.type_params.is_empty() {
        return Some(HashMap::new());
    }
    let Ok(te) = crate::parser::Parser::parse_type_expr_from_source(concrete_name) else {
        errors.push(SemanticError::new(
            format!("cannot parse concrete struct type `{concrete_name}`"),
            span,
        ));
        return None;
    };
    let TypeExpr::EnumApp { args, .. } = te else {
        errors.push(SemanticError::new(
            format!("expected concrete generic struct type, found `{concrete_name}`"),
            span,
        ));
        return None;
    };
    if args.len() != def.type_params.len() {
        errors.push(SemanticError::new(
            format!(
                "struct type argument count mismatch: expected {}, found {}",
                def.type_params.len(),
                args.len()
            ),
            span,
        ));
        return None;
    }
    let mut substs = HashMap::new();
    for (tp, arg_expr) in def.type_params.iter().zip(args.iter()) {
        let Some(arg_ty) = ty_from_type_expr(arg_expr, span, errors, &[], structs, enums, aliases) else {
            return None;
        };
        substs.insert(tp.name.clone(), arg_ty);
    }
    Some(substs)
}

fn check_call(
    callee: &str,
    type_args: &[TypeExpr],
    arguments: &[CallArg],
    span: Span,
    ctx: &mut BodyCtx<'_>,
    errors: &mut Vec<SemanticError>,
) -> Option<Ty> {
    fn arg_type_compatible(got: &Ty, expected: &Ty) -> bool {
        if got == expected {
            return true;
        }
        if matches!(got, Ty::InferVar(_)) || matches!(expected, Ty::InferVar(_)) {
            return true;
        }
        match expected {
            Ty::Any => true,
            Ty::Array(elem) if *elem.as_ref() == Ty::Any => matches!(got, Ty::Array(_)),
            _ => false,
        }
    }
    let Some(sig) = ctx.registry.get(callee) else {
        errors.push(SemanticError::new(
            format!("call to unknown function `{callee}`"),
            span,
        ));
        for arg in arguments {
            match arg {
                CallArg::Positional(expr) => {
                    let _ = check_expr(expr, ctx, errors);
                }
                CallArg::Named { value, .. } => {
                    let _ = check_expr(value, ctx, errors);
                }
            }
        }
        return None;
    };

    let params_index = sig.params_ast.iter().position(|p| p.is_params);
    let fixed_count = params_index.unwrap_or(sig.params.len());
    let mut ordered: Vec<Option<&AstNode>> = vec![None; sig.params.len()];
    let mut saw_named = false;
    let mut positional_idx = 0usize;
    let mut packed_params: Vec<&AstNode> = Vec::new();
    let mut params_explicit_by_name = false;
    for arg in arguments {
        match arg {
            CallArg::Positional(expr) => {
                if saw_named {
                    errors.push(SemanticError::new(
                        "positional arguments cannot follow named arguments",
                        expr_span(expr),
                    ));
                    continue;
                }
                if positional_idx >= fixed_count {
                    if params_index.is_some() {
                        packed_params.push(expr);
                        continue;
                    }
                    errors.push(SemanticError::new(
                        format!(
                            "function `{callee}` expects {} argument(s), found at least {}",
                            sig.params.len(),
                            positional_idx + 1
                        ),
                        expr_span(expr),
                    ));
                    continue;
                }
                if ordered[positional_idx].is_some() {
                    errors.push(SemanticError::new(
                        "duplicate argument provided",
                        expr_span(expr),
                    ));
                } else {
                    ordered[positional_idx] = Some(expr);
                }
                positional_idx += 1;
            }
            CallArg::Named {
                name,
                name_span,
                value,
            } => {
                saw_named = true;
                let Some(param_idx) = sig.params_ast.iter().position(|p| p.name == *name) else {
                    errors.push(SemanticError::new(
                        format!("unknown named argument `{name}` for `{callee}`"),
                        *name_span,
                    ));
                    let _ = check_expr(value, ctx, errors);
                    continue;
                };
                if ordered[param_idx].is_some() {
                    errors.push(SemanticError::new(
                        format!("duplicate argument `{name}`"),
                        *name_span,
                    ));
                } else {
                    ordered[param_idx] = Some(value);
                    if Some(param_idx) == params_index {
                        params_explicit_by_name = true;
                    }
                }
            }
        }
    }

    if let Some(pi) = params_index {
        if params_explicit_by_name && !packed_params.is_empty() {
            errors.push(SemanticError::new(
                "cannot use both packed positional arguments and explicit named `params` argument",
                span,
            ));
            return None;
        }
        if !params_explicit_by_name && ordered[pi].is_none() {
            // Omitted params becomes empty-array at call-site; semantic type-check happens via template.
        }
    }

    // `int_array_len` is intentionally polymorphic over the array element type.
    if callee == "int_array_len" {
        for (i, arg) in ordered.iter().enumerate() {
            let Some(arg_expr) = arg else {
                continue;
            };
            let Some(got) = check_expr(arg_expr, ctx, errors) else {
                continue;
            };
            if !matches!(got, Ty::Array(_)) {
                errors.push(SemanticError::new(
                    format!("`{callee}` expects an array argument (found `{}`)", ty_name(&got)),
                    expr_span(arg_expr),
                ));
            }
            if i > 0 {
                break;
            }
        }
        return sig.ret.clone();
    }

    let mut arg_tys: Vec<Ty> = Vec::with_capacity(sig.params.len());
    let mut arg_spans: Vec<Span> = Vec::with_capacity(sig.params.len());
    for (idx, arg) in ordered.iter().enumerate() {
        if Some(idx) == params_index && !params_explicit_by_name {
            let Ty::Array(elem) = &sig.params[idx] else {
                errors.push(SemanticError::new(
                    "`params` parameter must have array type `[T]`",
                    span,
                ));
                return None;
            };
            for v in &packed_params {
                if let Some(got) = check_expr(v, ctx, errors) {
                    // Monomorphic `params`: check element types immediately.
                    // Generic callee: infer `T` from arguments in the generic-inference pass below.
                    if sig.type_params.is_empty()
                        && *elem.as_ref() != Ty::Any
                        && got != *elem.as_ref()
                    {
                        errors.push(SemanticError::new(
                            format!(
                                "packed argument for `params` in `{callee}` has type `{}`, expected `{}`",
                                ty_name(&got),
                                ty_name(elem.as_ref())
                            ),
                            expr_span(v),
                        ));
                    }
                }
            }
            arg_spans.push(span);
            arg_tys.push(sig.params[idx].clone());
            continue;
        }
        let expr = if let Some(a) = arg {
            *a
        } else if let Some(def) = sig.params_ast[idx].default_value.as_ref() {
            def.as_ref()
        } else {
            errors.push(SemanticError::new(
                format!(
                    "function `{callee}` expects {} argument(s), found {} (missing `{}`)",
                    sig.params.len(),
                    arguments.len(),
                    sig.params_ast[idx].name
                ),
                span,
            ));
            return None;
        };
        arg_spans.push(expr_span(expr));
        if let Some(got) = check_expr(expr, ctx, errors) {
            arg_tys.push(got);
        } else {
            // keep placeholder to keep indices aligned; semantic errors were already emitted.
            arg_tys.push(Ty::Any);
        }
    }

    // Non-generic call: preserve the existing exact-type checks.
    if sig.type_params.is_empty() {
        for (i, got) in arg_tys.iter().enumerate() {
            let expected = &sig.params[i];
            if matches!(got, Ty::InferVar(_)) || matches!(expected, Ty::InferVar(_)) {
                let _ = infer::unify_types(
                    got,
                    expected,
                    &mut ctx.infer_ctx,
                    errors,
                    arg_spans[i],
                );
                continue;
            }
            if !arg_type_compatible(got, expected) {
                errors.push(SemanticError::new(
                    format!(
                        "argument {} to `{callee}` has type `{}`, expected `{}`",
                        i + 1,
                        ty_name(got),
                        ty_name(expected)
                    ),
                    arg_spans[i],
                ));
            }
        }
        return sig.ret.clone();
    }

    // Generic call: infer or validate substitutions for type parameters.
    let mut substs: HashMap<String, Ty> = HashMap::new();

    if !type_args.is_empty() {
        if type_args.len() != sig.type_params.len() {
            errors.push(SemanticError::new(
                format!(
                    "`{callee}` expects {} type argument(s), found {}",
                    sig.type_params.len(),
                    type_args.len()
                ),
                span,
            ));
            return None;
        }

        for (tp, te) in sig.type_params.iter().zip(type_args.iter()) {
            let Some(ty) =
                ty_from_type_expr(te, span, errors, &[], &ctx.structs, &ctx.enums, ctx.aliases)
            else {
                return None;
            };
            substs.insert(tp.name.clone(), ty);
        }
    } else {
        // Infer from value parameters (and nested structure) only.
        for (i, expected_template) in sig.params.iter().enumerate() {
            if Some(i) == params_index && !params_explicit_by_name && !packed_params.is_empty() {
                let Ty::Array(elem_tmpl) = expected_template else {
                    errors.push(SemanticError::new(
                        "`params` parameter must have array type `[T]`",
                        span,
                    ));
                    return None;
                };
                for v in &packed_params {
                    if let Some(got) = check_expr(v, ctx, errors) {
                        if !infer::infer_generic_from_template(
                            elem_tmpl.as_ref(),
                            &got,
                            &mut substs,
                            errors,
                            span,
                        ) {
                            return None;
                        }
                    }
                }
                continue;
            }
            let got = &arg_tys[i];
            if !infer::infer_generic_from_template(
                expected_template,
                got,
                &mut substs,
                errors,
                span,
            ) {
                return None;
            }
        }
    }

    for tp in &sig.type_params {
        if substs.contains_key(&tp.name) {
            continue;
        }
        if let Some(def_te) = &tp.default {
            if let Some(ty) = ty_from_type_expr(
                def_te,
                span,
                errors,
                &sig.type_params,
                &ctx.structs,
                &ctx.enums,
                ctx.aliases,
            ) {
                substs.insert(tp.name.clone(), ty);
            }
        }
    }

    for tp in &sig.type_params {
        if !substs.contains_key(&tp.name) {
            errors.push(SemanticError::new(
                format!("cannot infer type argument `{}` for call to `{callee}`", tp.name),
                span,
            ));
            return None;
        }
    }

    // Validate argument types after instantiation.
    for (i, got) in arg_tys.iter().enumerate() {
        if Some(i) == params_index && !params_explicit_by_name && !packed_params.is_empty() {
            let expected_template = &sig.params[i];
            let Ty::Array(elem_tmpl) = expected_template else {
                continue;
            };
            let expected_elem = infer::instantiate_ty(elem_tmpl.as_ref(), &substs);
            for v in &packed_params {
                if let Some(got_v) = check_expr(v, ctx, errors) {
                    if matches!(got_v, Ty::InferVar(_)) || matches!(expected_elem, Ty::InferVar(_)) {
                        let _ = infer::unify_types(
                            &got_v,
                            &expected_elem,
                            &mut ctx.infer_ctx,
                            errors,
                            expr_span(v),
                        );
                        continue;
                    }
                    if !arg_type_compatible(&got_v, &expected_elem) {
                        errors.push(SemanticError::new(
                            format!(
                                "packed argument for `params` in `{callee}` has type `{}`, expected `{}`",
                                ty_name(&got_v),
                                ty_name(&expected_elem)
                            ),
                            expr_span(v),
                        ));
                    }
                }
            }
            continue;
        }
        let expected_template = &sig.params[i];
        let expected_inst = infer::instantiate_ty(expected_template, &substs);
        if matches!(got, Ty::InferVar(_)) || matches!(expected_inst, Ty::InferVar(_)) {
            let _ = infer::unify_types(
                got,
                &expected_inst,
                &mut ctx.infer_ctx,
                errors,
                arg_spans[i],
            );
            continue;
        }
        if !arg_type_compatible(got, &expected_inst) {
            errors.push(SemanticError::new(
                format!(
                    "argument {} to `{callee}` has type `{}`, expected `{}`",
                    i + 1,
                    ty_name(got),
                    ty_name(&expected_inst)
                ),
                arg_spans[i],
            ));
        }
    }

    sig.ret
        .as_ref()
        .map(|ret_template| infer::instantiate_ty(ret_template, &substs))
}

fn ty_to_receiver_key(ty: &Ty) -> Option<String> {
    match ty {
        Ty::Int => Some("Int".to_string()),
        Ty::Float => Some("Float".to_string()),
        Ty::String => Some("String".to_string()),
        Ty::Bool => Some("Bool".to_string()),
        Ty::Unit => Some("()".to_string()),
        Ty::Array(elem) => {
            let inner = ty_to_receiver_key(elem.as_ref())?;
            Some(format!("[{inner}]"))
        }
        Ty::Tuple(parts) => {
            let mut out = String::from("(");
            for (i, p) in parts.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                out.push_str(&ty_to_receiver_key(p)?);
            }
            if parts.len() == 1 {
                out.push(',');
            }
            out.push(')');
            Some(out)
        }
        Ty::Struct(n) => Some(n.clone()),
        Ty::Enum { name, args } => {
            if args.is_empty() {
                Some(name.clone())
            } else {
                let mut parts = Vec::new();
                for a in args {
                    parts.push(ty_to_receiver_key(a)?);
                }
                Some(format!("{}<{}>", name, parts.join(", ")))
            }
        }
        Ty::Any => Some("Any".to_string()),
        Ty::TypeParam(name) => Some(name.clone()),
        Ty::Function { .. } => None,
        Ty::InferVar(_) => None,
        Ty::Task(_) => None,
    }
}

fn check_expr(
    expr: &AstNode,
    ctx: &mut BodyCtx<'_>,
    errors: &mut Vec<SemanticError>,
) -> Option<Ty> {
    match expr {
        AstNode::IntegerLiteral { .. } => Some(Ty::Int),
        AstNode::FloatLiteral { .. } => Some(Ty::Float),
        AstNode::StringLiteral { .. } => Some(Ty::String),
        AstNode::BoolLiteral { .. } => Some(Ty::Bool),
        AstNode::UnitLiteral { .. } => Some(Ty::Unit),
        AstNode::Await { expr, span } => {
            if !ctx.in_async {
                errors.push(SemanticError::new(
                    "`await` is only allowed inside `async` functions",
                    *span,
                ));
                let _ = check_expr(expr.as_ref(), ctx, errors);
                return None;
            }
            let inner_ty = check_expr(expr.as_ref(), ctx, errors)?;
            let resolved = infer::resolve_ty(&inner_ty, &ctx.infer_ctx);
            match resolved {
                Ty::Task(payload) => Some((payload.as_ref()).clone()),
                _ => {
                    errors.push(SemanticError::new(
                        format!(
                            "`await` requires a `Task` value, found `{}`",
                            ty_name(&resolved)
                        ),
                        *span,
                    ));
                    None
                }
            }
        }
        AstNode::Block { body, .. } => {
            // Blocks used as expression evaluate to `()`.
            // For now we disallow `return` inside blocks in expression position.
            if body.iter().any(|s| matches!(s, AstNode::Return { .. })) {
                errors.push(SemanticError::new(
                    "`return` is not allowed in a block used as an expression",
                    span_of_item(expr),
                ));
                return Some(Ty::Unit);
            }
            ctx.push_scope();
            for s in body {
                let _ = check_statement(s, None, ctx, true, errors);
            }
            ctx.pop_scope();
            Some(Ty::Unit)
        }
        AstNode::Match { scrutinee, arms, span } => {
            let scr_ty = check_expr(scrutinee, ctx, errors)?;

            if arms.is_empty() {
                errors.push(SemanticError::new("`match` requires at least one arm", *span));
                return None;
            }

            // Exhaustiveness (Rust-like subset):
            // - `match` over an enum must cover all variants unless `_` / binding is present.
            // - otherwise requires `_` / binding.
            let mut exhaustive_by_wildcard = false;
            let mut covered_variants: std::collections::HashSet<String> = std::collections::HashSet::new();
            if let Ty::Enum { name, .. } = &scr_ty {
                if let Some(def) = ctx.enums.get(name) {
                    for arm in arms {
                        for p in &arm.patterns {
                            match p {
                                Pattern::Wildcard { .. } | Pattern::Binding { .. }
                                    if arm.guard.is_none() =>
                                {
                                    exhaustive_by_wildcard = true;
                                }
                                Pattern::EnumVariant {
                                    enum_name,
                                    variant,
                                    payloads,
                                    ..
                                } if enum_name == name
                                    && arm.guard.is_none()
                                    && payloads.iter().all(pattern_is_irrefutable) =>
                                {
                                    covered_variants.insert(variant.clone());
                                }
                                _ => {}
                            }
                        }
                    }
                    if !exhaustive_by_wildcard {
                        let all: std::collections::HashSet<String> =
                            def.variants.keys().cloned().collect();
                        if covered_variants != all {
                            errors.push(SemanticError::new(
                                format!(
                                    "`match` over `{name}` is not exhaustive: missing variants {:?}",
                                    all.difference(&covered_variants).collect::<Vec<_>>()
                                ),
                                *span,
                            ));
                        }
                    }
                }
            } else {
                for arm in arms {
                    for p in &arm.patterns {
                        if matches!(p, Pattern::Wildcard { .. } | Pattern::Binding { .. }) {
                            exhaustive_by_wildcard = true;
                        }
                    }
                }
                if !exhaustive_by_wildcard {
                    errors.push(SemanticError::new(
                        "`match` on non-enum types requires a catch-all pattern (`_` or a binding)",
                        *span,
                    ));
                }
            }

            let mut result_ty: Option<Ty> = None;
            // Type-check each arm and ensure a single result type.
            for arm in arms {
                let mut arm_result_ty: Option<Ty> = None;

                for pat in &arm.patterns {
                    // Pattern bindings live only inside this alternative.
                    ctx.push_scope();
                    let base_len = ctx.bindings_ty.len();

                    let decl_ok =
                        declare_let_pattern(pat, &scr_ty, Some(scrutinee.as_ref()), true, ctx, errors);
                    let alt_ty = if decl_ok {
                        if let Some(g) = arm.guard.as_ref() {
                            let gt = check_expr(g.as_ref(), ctx, errors);
                            if let Some(t) = gt {
                                if t != Ty::Bool {
                                    errors.push(SemanticError::new(
                                        format!(
                                            "match guard must be `Bool`, found `{}`",
                                            ty_name(&t)
                                        ),
                                        expr_span(g.as_ref()),
                                    ));
                                }
                            }
                        }

                        check_expr(arm.body.as_ref(), ctx, errors)
                    } else {
                        None
                    };

                    ctx.pop_scope();
                    ctx.bindings_ty.truncate(base_len);
                    ctx.assigned.truncate(base_len);

                    if let Some(t) = alt_ty {
                        if let Some(prev) = &arm_result_ty {
                            if prev != &t {
                                errors.push(SemanticError::new(
                                    format!(
                                        "inconsistent arm result types in `match`: expected `{}`, found `{}`",
                                        ty_name(prev),
                                        ty_name(&t)
                                    ),
                                    *span,
                                ));
                            }
                        } else {
                            arm_result_ty = Some(t);
                        }
                    }
                }

                let Some(arm_ty) = arm_result_ty else {
                    return None;
                };
                match &result_ty {
                    None => result_ty = Some(arm_ty),
                    Some(prev) if prev == &arm_ty => {}
                    Some(prev) => {
                        errors.push(SemanticError::new(
                            format!(
                                "inconsistent `match` result types: expected `{}`, found `{}`",
                                ty_name(prev),
                                ty_name(&arm_ty)
                            ),
                            *span,
                        ));
                    }
                }
            }

            result_ty
        }
        AstNode::TupleLiteral { elements, .. } => {
            let mut tys = Vec::new();
            for e in elements {
                tys.push(check_expr(e, ctx, errors)?);
            }
            Some(Ty::Tuple(tys))
        }
        AstNode::TupleField { base, index, span } => {
            let base_ty = infer::resolve_ty(&check_expr(base, ctx, errors)?, &ctx.infer_ctx);
            match base_ty {
                Ty::Tuple(parts) => {
                    let i = *index as usize;
                    if i >= parts.len() {
                        errors.push(SemanticError::new(
                            format!("tuple index `.{}` out of range", index),
                            *span,
                        ));
                        return None;
                    }
                    Some(parts[i].clone())
                }
                _ => {
                    errors.push(SemanticError::new(
                        "tuple field access requires a tuple value",
                        *span,
                    ));
                    None
                }
            }
        }
        AstNode::ArrayLiteral { elements, span } => {
            // Infer element type from non-empty elements.
            // Empty array literals (`[]`) rely on surrounding context (i.e. inferred element type).
            let mut inferred_elem_ty: Option<Ty> = None;
            let mut had_non_empty = false;

            for e in elements {
                if let AstNode::ArrayLiteral {
                    elements: inner,
                    ..
                } = e
                {
                    if inner.is_empty() {
                        // Context-dependent: handled after inference.
                        continue;
                    }
                }
                had_non_empty = true;
                let got = check_expr(e, ctx, errors)?;
                match &inferred_elem_ty {
                    None => inferred_elem_ty = Some(got),
                    Some(prev) if prev == &got => {}
                    Some(prev) => {
                        errors.push(SemanticError::new(
                            format!(
                                "array literal elements must have the same type (expected `{}`, found `{}`)",
                                ty_name(prev),
                                ty_name(&got)
                            ),
                            expr_span(e),
                        ));
                        return None;
                    }
                }
            }

            if !had_non_empty {
                errors.push(SemanticError::new(
                    "cannot infer type of empty array literal",
                    *span,
                ));
                return None;
            }

            let elem_ty = inferred_elem_ty.expect("had_non_empty implies inferred type");
            // Validate empty array elements if present.
            for e in elements {
                if let AstNode::ArrayLiteral {
                    elements: inner,
                    ..
                } = e
                {
                    if inner.is_empty() {
                        // An empty array literal is still an array value, so the
                        // expected element type must itself be an array type.
                        if !matches!(elem_ty, Ty::Array(_)) {
                            errors.push(SemanticError::new(
                                "empty array literal is not compatible with the inferred element type",
                                expr_span(e),
                            ));
                            return None;
                        }
                    }
                }
            }

            Some(Ty::Array(Box::new(elem_ty)))
        }
        AstNode::DictLiteral { entries, span } => {
            if entries.is_empty() {
                errors.push(SemanticError::new(
                    "cannot infer type of empty dict literal",
                    *span,
                ));
                return None;
            }

            let mut key_ty: Option<Ty> = None;
            let mut val_ty: Option<Ty> = None;

            for (k, v) in entries {
                let got_k = check_expr(k, ctx, errors)?;
                let got_v = check_expr(v, ctx, errors)?;

                match &key_ty {
                    None => key_ty = Some(got_k),
                    Some(prev) if type_matches_with_any_wildcards(prev, &got_k) => {}
                    Some(prev) => {
                        errors.push(SemanticError::new(
                            format!(
                                "dict literal keys must have the same type (expected `{}`, found `{}`)",
                                ty_name(prev),
                                ty_name(&got_k)
                            ),
                            expr_span(k),
                        ));
                        return None;
                    }
                }

                match &val_ty {
                    None => val_ty = Some(got_v),
                    Some(prev) if type_matches_with_any_wildcards(prev, &got_v) => {}
                    Some(prev) => {
                        errors.push(SemanticError::new(
                            format!(
                                "dict literal values must have the same type (expected `{}`, found `{}`)",
                                ty_name(prev),
                                ty_name(&got_v)
                            ),
                            expr_span(v),
                        ));
                        return None;
                    }
                }
            }

            let key_ty = key_ty.expect("entries non-empty implies inferred key type");
            let val_ty = val_ty.expect("entries non-empty implies inferred value type");

            // Ensure `Dict` is declared.
            if ctx.structs.get("Dict").is_none() {
                errors.push(SemanticError::new(
                    "unknown struct `Dict`".to_string(),
                    *span,
                ));
                return None;
            }

            let inst_name = format!(
                "Dict<{}, {}>",
                ty_name(&key_ty),
                ty_name(&val_ty)
            );
            Some(Ty::Struct(inst_name))
        }
        AstNode::ArrayIndex { base, index, span } => {
            let base_ty = check_expr(base, ctx, errors)?;
            let idx_ty = check_expr(index, ctx, errors)?;
            match (base_ty, idx_ty) {
                (Ty::Array(elem), Ty::Int) => Some((*elem).clone()),
                (Ty::Array(_), other) => {
                    errors.push(SemanticError::new(
                        format!("array index requires `Int` (found `{}`)", ty_name(&other)),
                        *span,
                    ));
                    None
                }
                (other_base, _) => {
                    errors.push(SemanticError::new(
                        format!(
                            "array indexing requires an array base value (found `{}`)",
                            ty_name(&other_base)
                        ),
                        *span,
                    ));
                    None
                }
            }
        }
        AstNode::StructLiteral {
            name,
            type_args,
            fields,
            update,
            span,
        } => {
            let Some(def) = ctx.structs.get(name) else {
                errors.push(SemanticError::new(
                    format!("unknown struct `{name}`"),
                    *span,
                ));
                return None;
            };
            if def.is_unit {
                errors.push(SemanticError::new(
                    format!(
                        "`{name}` is a unit struct (`struct {name};`) and must be used as `{name}`, not `{name}{{...}}`"
                    ),
                    *span,
                ));
                return None;
            }

            // Resolve struct generic arguments.
            let mut struct_substs: HashMap<String, Ty> = HashMap::new();
            if !def.type_params.is_empty() {
                if !type_args.is_empty() {
                    if type_args.len() != def.type_params.len() {
                        errors.push(SemanticError::new(
                            format!(
                                "struct `{name}` expects {} type argument(s), found {}",
                                def.type_params.len(),
                                type_args.len()
                            ),
                            *span,
                        ));
                        return None;
                    }
                    for (tp, a) in def.type_params.iter().zip(type_args.iter()) {
                        let aty = ty_from_type_expr(
                            a,
                            *span,
                            errors,
                            &def.type_params,
                            &ctx.structs,
                            &ctx.enums,
                            ctx.aliases,
                        )?;
                        struct_substs.insert(tp.name.clone(), aty);
                    }
                } else {
                    // Infer from explicit field values.
                    for (f_name, f_expr) in fields {
                        let Some(tmpl) = def.fields.get(f_name) else {
                            continue;
                        };
                        let got_ty = check_expr(f_expr, ctx, errors)?;
                        let _ = infer::infer_generic_from_template(
                            tmpl,
                            &got_ty,
                            &mut struct_substs,
                            errors,
                            expr_span(f_expr),
                        );
                    }
                    if !def
                        .type_params
                        .iter()
                        .all(|tp| struct_substs.contains_key(&tp.name))
                    {
                        errors.push(SemanticError::new(
                            format!("cannot infer all type arguments for struct `{name}`"),
                            *span,
                        ));
                        return None;
                    }
                }
            }

            let expected_inst_name = if def.type_params.is_empty() {
                name.clone()
            } else {
                let args = def
                    .type_params
                    .iter()
                    .map(|tp| struct_substs.get(&tp.name).expect("inferred struct type arg"))
                    .map(ty_name)
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("{name}<{args}>")
            };

            // Validate update base (if present).
            if let Some(upd) = update.as_deref() {
                let base_ty = check_expr(upd, ctx, errors)?;
                match base_ty {
                    Ty::Struct(base_name) if base_name == expected_inst_name => {}
                    other => {
                        errors.push(SemanticError::new(
                            format!(
                                "struct update `..{name}` requires a `{expected_inst_name}` base value (found `{}`)",
                                ty_name(&other)
                            ),
                            expr_span(upd),
                        ));
                        return None;
                    }
                }
            } else if fields.len() != def.fields.len() {
                // No update => strict full initialization.
                errors.push(SemanticError::new(
                    format!(
                        "struct literal `{name}` requires all fields (expected {}, found {})",
                        def.fields.len(),
                        fields.len()
                    ),
                    *span,
                ));
                return None;
            }

            // Validate explicit fields.
            let mut seen: HashMap<&str, Span> = HashMap::new();
            for (f_name, f_expr) in fields {
                if seen.insert(f_name.as_str(), *span).is_some() {
                    errors.push(SemanticError::new(
                        format!("duplicate field `{f_name}` in struct literal `{name}`"),
                        *span,
                    ));
                    return None;
                }
                let Some(expected_ty) = def.fields.get(f_name) else {
                    errors.push(SemanticError::new(
                        format!("unknown field `{f_name}` in struct `{name}`"),
                        expr_span(f_expr),
                    ));
                    return None;
                };
                let expected_ty = infer::instantiate_ty(expected_ty, &struct_substs);
                let got_ty = check_expr(f_expr, ctx, errors)?;
                if expected_ty != Ty::Any && got_ty != expected_ty {
                    errors.push(SemanticError::new(
                        format!(
                            "field `{f_name}` initializer has wrong type (expected `{}`, found `{}`)",
                            ty_name(&expected_ty),
                            ty_name(&got_ty)
                        ),
                        expr_span(f_expr),
                    ));
                    return None;
                }
            }

            Some(Ty::Struct(expected_inst_name))
        }
        AstNode::FieldAccess { base, field, span } => {
            let base_ty = check_expr(base, ctx, errors)?;
            match base_ty {
                Ty::Struct(struct_name) => {
                    let Some(def) = ctx.structs.get(struct_base_name(&struct_name)) else {
                        errors.push(SemanticError::new(
                            format!("missing definition for struct `{struct_name}`"),
                            *span,
                        ));
                        return None;
                    };
                    if let Some(fty) = def.fields.get(field) {
                        Some(fty.clone())
                    } else {
                        errors.push(SemanticError::new(
                            format!("unknown field `{field}` on struct `{struct_name}`"),
                            *span,
                        ));
                        None
                    }
                }
                Ty::TypeParam(_) => {
                    errors.push(SemanticError::new(
                        "field access on generic type parameters requires constraints (not supported yet)",
                        *span,
                    ));
                    None
                }
                Ty::Any => {
                    errors.push(SemanticError::new(
                        "field access is not allowed on `Any` values",
                        *span,
                    ));
                    None
                }
                other => {
                    errors.push(SemanticError::new(
                        format!(
                            "field access requires a struct base value (found `{}`)",
                            ty_name(&other)
                        ),
                        *span,
                    ));
                    None
                }
            }
        }
        AstNode::EnumVariantCtor {
            enum_name,
            type_args,
            variant,
            payloads,
            span,
        } => {
            let Some(def) = ctx.enums.get(enum_name) else {
                errors.push(SemanticError::new(
                    format!("unknown enum `{enum_name}`"),
                    *span,
                ));
                return None;
            };
            let Some(payload_templates) = def.variants.get(variant) else {
                errors.push(SemanticError::new(
                    format!("unknown variant `{variant}` for enum `{enum_name}`"),
                    *span,
                ));
                return None;
            };
            if payload_templates.len() != payloads.len() {
                errors.push(SemanticError::new(
                    format!(
                        "enum variant `{enum_name}::{variant}` expects {} payload(s), found {}",
                        payload_templates.len(),
                        payloads.len()
                    ),
                    *span,
                ));
                return None;
            }

            if !type_args.is_empty() && type_args.len() != def.type_params.len() {
                errors.push(SemanticError::new(
                    format!(
                        "`{enum_name}` expects {} type argument(s) in constructor, found {}",
                        def.type_params.len(),
                        type_args.len()
                    ),
                    *span,
                ));
                return None;
            }

            // Seed known type arguments (for `_` we leave them unset and infer later).
            let mut substs: HashMap<String, Ty> = HashMap::new();
            for (tp, te) in def.type_params.iter().zip(type_args.iter()) {
                match te {
                    TypeExpr::Infer => {}
                    other => {
                        let Some(ty) = ty_from_type_expr(
                            other,
                            *span,
                            errors,
                            &[],
                            &ctx.structs,
                            &ctx.enums,
                            ctx.aliases,
                        ) else {
                            return None;
                        };
                        substs.insert(tp.name.clone(), ty);
                    }
                }
            }

            // Zero-payload variants: type args must be fully known unless contextual typing is used.
            if payload_templates.is_empty() {
                for tp in &def.type_params {
                    if !substs.contains_key(&tp.name) {
                        errors.push(SemanticError::new(
                            format!(
                                "cannot infer type argument `{}` for enum `{enum_name}` from zero-payload variant `{variant}`",
                                tp.name
                            ),
                            *span,
                        ));
                        return None;
                    }
                }

                let args = def
                    .type_params
                    .iter()
                    .map(|tp| substs.get(&tp.name).unwrap().clone())
                    .collect();

                return Some(Ty::Enum {
                    name: enum_name.clone(),
                    args,
                });
            }

            // Infer remaining type args from payload expressions.
            for (tpl, payload_expr) in payload_templates.iter().zip(payloads.iter()) {
                let tpl_ty = ty_from_type_expr(
                    tpl,
                    expr_span(payload_expr),
                    errors,
                    &def.type_params,
                    &ctx.structs,
                    &ctx.enums,
                    ctx.aliases,
                )?;
                let got_ty = check_expr(payload_expr, ctx, errors)?;
                if !infer::infer_generic_from_template(
                    &tpl_ty,
                    &got_ty,
                    &mut substs,
                    errors,
                    *span,
                ) {
                    return None;
                }
            }

            // All enum type arguments must be resolved.
            let mut args = Vec::with_capacity(def.type_params.len());
            for tp in &def.type_params {
                match substs.get(&tp.name) {
                    Some(ty) => args.push(ty.clone()),
                    None => {
                        errors.push(SemanticError::new(
                            format!(
                                "cannot infer type argument `{}` for enum `{enum_name}`",
                                tp.name
                            ),
                            *span,
                        ));
                        return None;
                    }
                }
            }

            Some(Ty::Enum {
                name: enum_name.clone(),
                args,
            })
        }
        AstNode::Identifier { name, span } => match ctx.lookup(name) {
            None => {
                if let Some(sig) = ctx.registry.get(name) {
                    return Some(Ty::Function {
                        params: sig.params.clone(),
                        param_names: sig
                            .params_ast
                            .iter()
                            .map(|p| Some(p.name.clone()))
                            .collect(),
                        param_has_default: sig
                            .params_ast
                            .iter()
                            .map(|p| p.default_value.is_some())
                            .collect(),
                        ret: Box::new(sig.ret.clone().unwrap_or(Ty::Unit)),
                    });
                }
                if let Some(sd) = ctx.structs.get(name) {
                    if sd.is_unit {
                        Some(Ty::Struct(name.clone()))
                    } else {
                        errors.push(SemanticError::new(
                            format!("unknown identifier `{name}`"),
                            *span,
                        ));
                        None
                    }
                } else {
                    errors.push(SemanticError::new(
                        format!("unknown identifier `{name}`"),
                        *span,
                    ));
                    None
                }
            }
            Some(NameRes::Local(id)) => {
                if !ctx.assigned[id] {
                    errors.push(SemanticError::new(
                        format!("`{name}` may be uninitialized"),
                        *span,
                    ));
                    return None;
                }
                match &ctx.bindings_ty[id] {
                    Some(ty) => Some(ty.clone()),
                    None => {
                        errors.push(SemanticError::new(
                            format!("`{name}` has no inferred type yet"),
                            *span,
                        ));
                        None
                    }
                }
            }
            Some(NameRes::Global(ty)) => Some(ty.clone()),
        },
        AstNode::TypeValue { type_name, span } => {
            if let Ok(te) = crate::parser::Parser::parse_type_expr_from_source(type_name) {
                if let Some(ty) = ty_from_type_expr(
                    &te,
                    *span,
                    errors,
                    &[],
                    &ctx.structs,
                    &ctx.enums,
                    ctx.aliases,
                ) {
                    if let Ty::Struct(struct_name) = &ty {
                        let base = struct_name
                            .split_once('<')
                            .map(|(b, _)| b.to_string())
                            .unwrap_or_else(|| struct_name.clone());
                        if ctx.structs.get(&base).is_some_and(|s| s.is_unit) {
                            return Some(ty);
                        }
                    }
                }
            }
            errors.push(SemanticError::new(
                format!("`{type_name}` is not a valid type value expression"),
                *span,
            ));
            None
        }
        AstNode::Lambda { params, body, span: _ } => {
            let inferred_param_tys: Vec<Ty> = (0..params.len())
                .map(|_| ctx.infer_ctx.fresh_var())
                .collect();
            ctx.push_scope();
            for (idx, p) in params.iter().enumerate() {
                let ty = inferred_param_tys[idx].clone();
                ctx.declare_binding_destructure(&p.name, p.name_span, ty, errors);
            }
            let ret_var = ctx.infer_ctx.fresh_var();
            let ret = match body.as_ref() {
                crate::ast::LambdaBody::Expr(expr) => {
                    let got = check_expr(expr, ctx, errors).unwrap_or_else(|| ctx.infer_ctx.fresh_var());
                    let _ = infer::unify_types(
                        &ret_var,
                        &got,
                        &mut ctx.infer_ctx,
                        errors,
                        expr_span(expr),
                    );
                    ret_var.clone()
                }
                crate::ast::LambdaBody::Block(items) => {
                    let mut reachable = true;
                    for s in items {
                        match check_statement(s, Some(&ret_var), ctx, reachable, errors) {
                            StmtFlow::Next(r) => reachable = r,
                            _ => {}
                        }
                    }
                    ret_var.clone()
                }
            };
            ctx.pop_scope();
            let resolved_params = inferred_param_tys
                .iter()
                .map(|p| infer::resolve_ty(p, &ctx.infer_ctx))
                .collect::<Vec<_>>();
            let resolved_ret = infer::resolve_ty(&ret, &ctx.infer_ctx);
            Some(Ty::Function {
                params: resolved_params,
                param_names: vec![None; params.len()],
                param_has_default: vec![false; params.len()],
                ret: Box::new(resolved_ret),
            })
        }
        AstNode::BinaryOp {
            left,
            op,
            right,
            span,
        } => {
            let mut lt = check_expr(left, ctx, errors);
            let mut rt = check_expr(right, ctx, errors);
            lt = lt.map(|t| infer::resolve_ty(&t, &ctx.infer_ctx));
            rt = rt.map(|t| infer::resolve_ty(&t, &ctx.infer_ctx));
            fn op_key(recv: &Ty, method: &str) -> Option<String> {
                let tyname = match recv {
                    Ty::Int => "Int",
                    Ty::Float => "Float",
                    Ty::String => "String",
                    Ty::Bool => "Bool",
                    _ => return None,
                };
                Some(format!("{tyname}::{method}"))
            }

            fn push_missing_op(errors: &mut Vec<SemanticError>, span: Span, key: &str) {
                errors.push(SemanticError::new(
                    format!("operator is not available: missing `internal func {key}(...)`"),
                    span,
                ));
            }

            fn require_op(
                registry: &HashMap<String, FuncSig>,
                errors: &mut Vec<SemanticError>,
                span: Span,
                recv: &Ty,
                method: &str,
            ) -> bool {
                let Some(key) = op_key(recv, method) else {
                    return false;
                };
                if registry.contains_key(&key) {
                    true
                } else {
                    push_missing_op(errors, span, &key);
                    false
                }
            }

            fn recv_custom_name(ty: &Ty) -> Option<String> {
                match ty {
                    Ty::Struct(n) => Some(struct_base_name(n).to_string()),
                    Ty::Enum { name, .. } => Some(name.clone()),
                    _ => None,
                }
            }

            let mut resolve_custom_binary = |lhs_ty: &Ty, rhs_ty: &Ty, method: &str| -> Option<Ty> {
                let recv = recv_custom_name(lhs_ty)?;
                let base = format!("{recv}::{method}");
                let mut candidates: Vec<(&String, &FuncSig)> = ctx
                    .registry
                    .iter()
                    .filter(|(k, sig)| {
                        (k.as_str() == base || k.starts_with(&(base.clone() + "#op(")))
                            && sig.params.len() >= 2
                    })
                    .collect();
                if candidates.is_empty() {
                    errors.push(SemanticError::new(
                        format!("operator is not available: missing overload `{base}(self, rhs)`"),
                        *span,
                    ));
                    return None;
                }
                candidates.sort_by(|a, b| a.0.cmp(b.0));
                let mut matches: Vec<&FuncSig> = Vec::new();
                for (_k, sig) in candidates {
                    let exp = &sig.params[1];
                    let compatible = rhs_ty == exp
                        || matches!(rhs_ty, Ty::InferVar(_))
                        || matches!(exp, Ty::InferVar(_))
                        || matches!(exp, Ty::Any);
                    if compatible {
                        matches.push(sig);
                    }
                }
                if matches.is_empty() {
                    errors.push(SemanticError::new(
                        format!(
                            "no overload for operator method `{base}` matches rhs type `{}`",
                            ty_name(rhs_ty)
                        ),
                        *span,
                    ));
                    return None;
                }
                if matches.len() > 1 {
                    errors.push(SemanticError::new(
                        format!("ambiguous operator overload for `{base}`"),
                        *span,
                    ));
                    return None;
                }
                matches[0].ret.clone().or(Some(Ty::Unit))
            };

            fn unify_both(
                target: &Ty,
                lt: &Option<Ty>,
                rt: &Option<Ty>,
                infer_ctx: &mut infer::InferCtx,
                errors: &mut Vec<SemanticError>,
                span: Span,
            ) -> bool {
                let ok_l = lt
                    .as_ref()
                    .is_some_and(|l| infer::unify_types(l, target, infer_ctx, errors, span));
                let ok_r = rt
                    .as_ref()
                    .is_some_and(|r| infer::unify_types(r, target, infer_ctx, errors, span));
                ok_l && ok_r
            }

            if let (Some(lty), Some(rty)) = (&lt, &rt) {
                let custom_method = match op {
                    BinaryOp::Add => Some("binary_add"),
                    BinaryOp::Sub => Some("binary_sub"),
                    BinaryOp::Mul => Some("binary_mul"),
                    BinaryOp::Div => Some("binary_div"),
                    BinaryOp::Mod => Some("binary_mod"),
                    BinaryOp::BitAnd => Some("binary_bitwise_and"),
                    BinaryOp::BitOr => Some("binary_bitwise_or"),
                    BinaryOp::BitXor => Some("binary_bitwise_xor"),
                    BinaryOp::ShiftLeft => Some("binary_left_shift"),
                    BinaryOp::ShiftRight => Some("binary_right_shift"),
                    BinaryOp::Eq => Some("compare_equal"),
                    BinaryOp::Ne => Some("compare_not_equal"),
                    BinaryOp::Lt => Some("compare_less"),
                    BinaryOp::Le => Some("compare_less_or_equal"),
                    BinaryOp::Gt => Some("compare_greater"),
                    BinaryOp::Ge => Some("compare_greater_or_equal"),
                    BinaryOp::And => Some("binary_and"),
                    BinaryOp::Or => Some("binary_or"),
                };
                if matches!(lty, Ty::Struct(_) | Ty::Enum { .. }) {
                    if let Some(m) = custom_method {
                        if let Some(ret) = resolve_custom_binary(lty, rty, m) {
                            return Some(ret);
                        }
                        return None;
                    }
                }
            }
            if lt.as_ref().is_some_and(contains_type_param)
                || rt.as_ref().is_some_and(contains_type_param)
                || lt.as_ref().is_some_and(contains_struct)
                || rt.as_ref().is_some_and(contains_struct)
            {
                errors.push(SemanticError::new(
                    "operators are not allowed on generic type parameters / structs (no constraints yet)",
                    *span,
                ));
                return None;
            }
            match op {
                BinaryOp::Add => match (&lt, &rt) {
                    (Some(Ty::Int), Some(Ty::Int)) => {
                        if require_op(&ctx.registry, errors, *span, &Ty::Int, "binary_add") {
                            Some(Ty::Int)
                        } else {
                            None
                        }
                    }
                    (Some(Ty::Float), Some(Ty::Float)) => {
                        if require_op(&ctx.registry, errors, *span, &Ty::Float, "binary_add") {
                            Some(Ty::Float)
                        } else {
                            None
                        }
                    }
                    (Some(Ty::String), Some(Ty::String)) => {
                        if require_op(&ctx.registry, errors, *span, &Ty::String, "binary_add") {
                            Some(Ty::String)
                        } else {
                            None
                        }
                    }
                    (Some(Ty::InferVar(_)), Some(Ty::Int))
                    | (Some(Ty::Int), Some(Ty::InferVar(_))) => {
                        if require_op(&ctx.registry, errors, *span, &Ty::Int, "binary_add") {
                            let _ = unify_both(
                                &Ty::Int,
                                &lt,
                                &rt,
                                &mut ctx.infer_ctx,
                                errors,
                                *span,
                            );
                            Some(Ty::Int)
                        } else {
                            None
                        }
                    }
                    (Some(Ty::InferVar(_)), Some(Ty::Float))
                    | (Some(Ty::Float), Some(Ty::InferVar(_))) => {
                        if require_op(&ctx.registry, errors, *span, &Ty::Float, "binary_add") {
                            let _ = unify_both(
                                &Ty::Float,
                                &lt,
                                &rt,
                                &mut ctx.infer_ctx,
                                errors,
                                *span,
                            );
                            Some(Ty::Float)
                        } else {
                            None
                        }
                    }
                    (Some(Ty::InferVar(_)), Some(Ty::String))
                    | (Some(Ty::String), Some(Ty::InferVar(_))) => {
                        if require_op(&ctx.registry, errors, *span, &Ty::String, "binary_add") {
                            let _ = unify_both(
                                &Ty::String,
                                &lt,
                                &rt,
                                &mut ctx.infer_ctx,
                                errors,
                                *span,
                            );
                            Some(Ty::String)
                        } else {
                            None
                        }
                    }
                    _ => {
                        errors.push(SemanticError::new(
                            "operator `+` is only defined for `Int + Int`, `Float + Float`, or `String + String`",
                            *span,
                        ));
                        None
                    }
                },
                BinaryOp::Sub | BinaryOp::Mul | BinaryOp::Div | BinaryOp::Mod => {
                    let method = match op {
                        BinaryOp::Sub => "binary_sub",
                        BinaryOp::Mul => "binary_mul",
                        BinaryOp::Div => "binary_div",
                        BinaryOp::Mod => "binary_mod",
                        _ => unreachable!("covered by outer match"),
                    };
                    match (&lt, &rt) {
                        (Some(Ty::Int), Some(Ty::Int)) => {
                            if require_op(&ctx.registry, errors, *span, &Ty::Int, method) {
                                Some(Ty::Int)
                            } else {
                                None
                            }
                        }
                        (Some(Ty::Float), Some(Ty::Float)) => {
                            if require_op(&ctx.registry, errors, *span, &Ty::Float, method) {
                                Some(Ty::Float)
                            } else {
                                None
                            }
                        }
                        (Some(Ty::InferVar(_)), Some(Ty::Int))
                        | (Some(Ty::Int), Some(Ty::InferVar(_))) => {
                            if require_op(&ctx.registry, errors, *span, &Ty::Int, method) {
                                let _ = unify_both(
                                    &Ty::Int,
                                    &lt,
                                    &rt,
                                    &mut ctx.infer_ctx,
                                    errors,
                                    *span,
                                );
                                Some(Ty::Int)
                            } else {
                                None
                            }
                        }
                        (Some(Ty::InferVar(_)), Some(Ty::Float))
                        | (Some(Ty::Float), Some(Ty::InferVar(_))) => {
                            if require_op(&ctx.registry, errors, *span, &Ty::Float, method) {
                                let _ = unify_both(
                                    &Ty::Float,
                                    &lt,
                                    &rt,
                                    &mut ctx.infer_ctx,
                                    errors,
                                    *span,
                                );
                                Some(Ty::Float)
                            } else {
                                None
                            }
                        }
                        (Some(Ty::InferVar(_)), Some(Ty::InferVar(_))) => {
                            // Prefer Int if available, otherwise Float; otherwise reject.
                            if ctx
                                .registry
                                .contains_key(&op_key(&Ty::Int, method).expect("Int key"))
                            {
                                let _ = require_op(&ctx.registry, errors, *span, &Ty::Int, method);
                                let _ = unify_both(
                                    &Ty::Int,
                                    &lt,
                                    &rt,
                                    &mut ctx.infer_ctx,
                                    errors,
                                    *span,
                                );
                                Some(Ty::Int)
                            } else if ctx
                                .registry
                                .contains_key(&op_key(&Ty::Float, method).expect("Float key"))
                            {
                                let _ =
                                    require_op(&ctx.registry, errors, *span, &Ty::Float, method);
                                let _ = unify_both(
                                    &Ty::Float,
                                    &lt,
                                    &rt,
                                    &mut ctx.infer_ctx,
                                    errors,
                                    *span,
                                );
                                Some(Ty::Float)
                            } else {
                                push_missing_op(
                                    errors,
                                    *span,
                                    &op_key(&Ty::Int, method).expect("Int key"),
                                );
                                push_missing_op(
                                    errors,
                                    *span,
                                    &op_key(&Ty::Float, method).expect("Float key"),
                                );
                                None
                            }
                        }
                        _ => {
                            errors.push(SemanticError::new(
                                "arithmetic operators require `Int` or `Float` operands of the same type",
                                *span,
                            ));
                            None
                        }
                    }
                }
                BinaryOp::BitAnd | BinaryOp::BitXor | BinaryOp::BitOr => {
                    let method = match op {
                        BinaryOp::BitAnd => "binary_bitwise_and",
                        BinaryOp::BitXor => "binary_bitwise_xor",
                        BinaryOp::BitOr => "binary_bitwise_or",
                        _ => unreachable!("covered by outer match"),
                    };
                    if ((lt == Some(Ty::Int) && rt == Some(Ty::Int))
                        || unify_both(&Ty::Int, &lt, &rt, &mut ctx.infer_ctx, errors, *span))
                        && require_op(&ctx.registry, errors, *span, &Ty::Int, method)
                    {
                        Some(Ty::Int)
                    } else {
                        errors.push(SemanticError::new(
                            "bitwise `&`, `^`, and `|` require `Int` operands",
                            *span,
                        ));
                        None
                    }
                }
                BinaryOp::ShiftLeft | BinaryOp::ShiftRight => {
                    let method = match op {
                        BinaryOp::ShiftLeft => "binary_left_shift",
                        BinaryOp::ShiftRight => "binary_right_shift",
                        _ => unreachable!("covered by outer match"),
                    };
                    if ((lt == Some(Ty::Int) && rt == Some(Ty::Int))
                        || unify_both(&Ty::Int, &lt, &rt, &mut ctx.infer_ctx, errors, *span))
                        && require_op(&ctx.registry, errors, *span, &Ty::Int, method)
                    {
                        Some(Ty::Int)
                    } else {
                        errors.push(SemanticError::new(
                            "shift operators `<<` and `>>` require `Int` operands",
                            *span,
                        ));
                        None
                    }
                }
                BinaryOp::Eq | BinaryOp::Ne => match (&lt, &rt) {
                    (Some(Ty::Any), _) | (_, Some(Ty::Any)) => {
                        errors.push(SemanticError::new(
                            "operators are not allowed on `Any` values",
                            *span,
                        ));
                        None
                    }
                    (Some(a), Some(b)) if a == b => match a {
                        Ty::Int => {
                            if require_op(
                                &ctx.registry,
                                errors,
                                *span,
                                &Ty::Int,
                                if matches!(op, BinaryOp::Eq) {
                                    "compare_equal"
                                } else {
                                    "compare_not_equal"
                                },
                            ) {
                                Some(Ty::Bool)
                            } else {
                                None
                            }
                        }
                        Ty::Float => {
                            if require_op(
                                &ctx.registry,
                                errors,
                                *span,
                                &Ty::Float,
                                if matches!(op, BinaryOp::Eq) {
                                    "compare_equal"
                                } else {
                                    "compare_not_equal"
                                },
                            ) {
                                Some(Ty::Bool)
                            } else {
                                None
                            }
                        }
                        Ty::String => {
                            if require_op(
                                &ctx.registry,
                                errors,
                                *span,
                                &Ty::String,
                                if matches!(op, BinaryOp::Eq) {
                                    "compare_equal"
                                } else {
                                    "compare_not_equal"
                                },
                            ) {
                                Some(Ty::Bool)
                            } else {
                                None
                            }
                        }
                        Ty::Bool => {
                            if require_op(
                                &ctx.registry,
                                errors,
                                *span,
                                &Ty::Bool,
                                if matches!(op, BinaryOp::Eq) {
                                    "compare_equal"
                                } else {
                                    "compare_not_equal"
                                },
                            ) {
                                Some(Ty::Bool)
                            } else {
                                None
                            }
                        }
                        Ty::Unit => Some(Ty::Bool),
                        Ty::Tuple(_) => Some(Ty::Bool),
                        Ty::Array(_) => Some(Ty::Bool),
                        Ty::Enum { .. } => {
                            errors.push(SemanticError::new(
                                "operators are not allowed on enums",
                                *span,
                            ));
                            None
                        }
                        Ty::Any => {
                            errors.push(SemanticError::new(
                                "operators are not allowed on `Any` values",
                                *span,
                            ));
                            None
                        }
                    Ty::TypeParam(_) => {
                            errors.push(SemanticError::new(
                                "operators are not allowed on generic type parameters (no constraints yet)",
                                *span,
                            ));
                            None
                        }
                    Ty::Struct(_) => {
                        errors.push(SemanticError::new(
                            "operators are not allowed on structs",
                            *span,
                        ));
                        None
                    }
                        Ty::Function { .. } => {
                            errors.push(SemanticError::new(
                                "operators are not allowed on functions/lambdas",
                                *span,
                            ));
                            None
                        }
                        Ty::InferVar(_) => Some(Ty::Bool),
                        Ty::Task(_) => {
                            errors.push(SemanticError::new(
                                "operators are not allowed on `Task` values",
                                *span,
                            ));
                            None
                        }
                    },
                    (Some(a), Some(b)) => {
                        errors.push(SemanticError::new(
                            format!(
                                "equality requires operands of the same type (found `{}` and `{}`)",
                                ty_name(a),
                                ty_name(b)
                            ),
                            *span,
                        ));
                        None
                    }
                    _ => None,
                },
                BinaryOp::Lt | BinaryOp::Gt | BinaryOp::Le | BinaryOp::Ge => {
                    let method = match op {
                        BinaryOp::Lt => "compare_less",
                        BinaryOp::Le => "compare_less_or_equal",
                        BinaryOp::Gt => "compare_greater",
                        BinaryOp::Ge => "compare_greater_or_equal",
                        _ => unreachable!("covered by outer match"),
                    };
                    match (&lt, &rt) {
                        (Some(Ty::Int), Some(Ty::Int)) => {
                            if require_op(&ctx.registry, errors, *span, &Ty::Int, method) {
                                Some(Ty::Bool)
                            } else {
                                None
                            }
                        }
                        (Some(Ty::Float), Some(Ty::Float)) => {
                            if require_op(&ctx.registry, errors, *span, &Ty::Float, method) {
                                Some(Ty::Bool)
                            } else {
                                None
                            }
                        }
                        (Some(Ty::InferVar(_)), Some(Ty::Int))
                        | (Some(Ty::Int), Some(Ty::InferVar(_))) => {
                            if require_op(&ctx.registry, errors, *span, &Ty::Int, method) {
                                let _ = unify_both(
                                    &Ty::Int,
                                    &lt,
                                    &rt,
                                    &mut ctx.infer_ctx,
                                    errors,
                                    *span,
                                );
                                Some(Ty::Bool)
                            } else {
                                None
                            }
                        }
                        (Some(Ty::InferVar(_)), Some(Ty::Float))
                        | (Some(Ty::Float), Some(Ty::InferVar(_))) => {
                            if require_op(&ctx.registry, errors, *span, &Ty::Float, method) {
                                let _ = unify_both(
                                    &Ty::Float,
                                    &lt,
                                    &rt,
                                    &mut ctx.infer_ctx,
                                    errors,
                                    *span,
                                );
                                Some(Ty::Bool)
                            } else {
                                None
                            }
                        }
                        (Some(Ty::InferVar(_)), Some(Ty::InferVar(_))) => {
                            if ctx
                                .registry
                                .contains_key(&op_key(&Ty::Int, method).expect("Int key"))
                            {
                                let _ = require_op(&ctx.registry, errors, *span, &Ty::Int, method);
                                let _ = unify_both(
                                    &Ty::Int,
                                    &lt,
                                    &rt,
                                    &mut ctx.infer_ctx,
                                    errors,
                                    *span,
                                );
                                Some(Ty::Bool)
                            } else if ctx
                                .registry
                                .contains_key(&op_key(&Ty::Float, method).expect("Float key"))
                            {
                                let _ =
                                    require_op(&ctx.registry, errors, *span, &Ty::Float, method);
                                let _ = unify_both(
                                    &Ty::Float,
                                    &lt,
                                    &rt,
                                    &mut ctx.infer_ctx,
                                    errors,
                                    *span,
                                );
                                Some(Ty::Bool)
                            } else {
                                push_missing_op(
                                    errors,
                                    *span,
                                    &op_key(&Ty::Int, method).expect("Int key"),
                                );
                                push_missing_op(
                                    errors,
                                    *span,
                                    &op_key(&Ty::Float, method).expect("Float key"),
                                );
                                None
                            }
                        }
                        _ => {
                            errors.push(SemanticError::new(
                                "ordering comparisons `<`, `>`, `<=`, `>=` require `Int` or `Float` operands of the same type",
                                *span,
                            ));
                            None
                        }
                    }
                }
                BinaryOp::And | BinaryOp::Or => {
                    if (lt == Some(Ty::Bool) && rt == Some(Ty::Bool))
                        || unify_both(&Ty::Bool, &lt, &rt, &mut ctx.infer_ctx, errors, *span)
                    {
                        let method = match op {
                            BinaryOp::And => "binary_and",
                            BinaryOp::Or => "binary_or",
                            _ => unreachable!("covered by outer match"),
                        };
                        if !require_op(&ctx.registry, errors, *span, &Ty::Bool, method) {
                            return None;
                        }
                        Some(Ty::Bool)
                    } else {
                        errors.push(SemanticError::new(
                            "logical `&&` and `||` require `Bool` operands",
                            *span,
                        ));
                        None
                    }
                }
            }
        }
        AstNode::UnaryOp { op, operand, span } => {
            let t = check_expr(operand, ctx, errors).map(|tt| infer::resolve_ty(&tt, &ctx.infer_ctx));
            if let Some(tt) = t.as_ref() {
                let recv = match tt {
                    Ty::Struct(n) => Some(struct_base_name(n).to_string()),
                    Ty::Enum { name, .. } => Some(name.clone()),
                    _ => None,
                };
                if let Some(recv) = recv {
                    let method = match op {
                        UnaryOp::Not => Some("unary_not"),
                        UnaryOp::BitNot => Some("unary_bitwise_not"),
                        UnaryOp::Plus => Some("unary_plus"),
                        UnaryOp::Minus => Some("unary_minus"),
                    };
                    if let Some(method) = method {
                        let base = format!("{recv}::{method}");
                        let mut matches = 0usize;
                        let mut ret: Option<Ty> = None;
                        for (k, sig) in ctx.registry {
                            if (k.as_str() == base || k.starts_with(&(base.clone() + "#op(")))
                                && !sig.params.is_empty()
                            {
                                matches += 1;
                                ret = sig.ret.clone();
                            }
                        }
                        if matches == 1 {
                            return ret.or(Some(Ty::Unit));
                        }
                        if matches > 1 {
                            errors.push(SemanticError::new(
                                format!("ambiguous operator overload for `{base}`"),
                                *span,
                            ));
                            return None;
                        }
                    }
                }
            }
            if t.as_ref().is_some_and(contains_type_param) {
                errors.push(SemanticError::new(
                    "operators are not allowed on generic type parameters (no constraints yet)",
                    *span,
                ));
                return None;
            }
            if t.as_ref().is_some_and(contains_struct) {
                errors.push(SemanticError::new("operators are not allowed on structs", *span));
                return None;
            }
            match op {
                UnaryOp::Not => {
                    if t.as_ref().is_some_and(|tt| {
                        infer::unify_types(tt, &Ty::Bool, &mut ctx.infer_ctx, errors, *span)
                    }) && ctx.registry.contains_key("Bool::unary_not") {
                        Some(Ty::Bool)
                    } else {
                        if !ctx.registry.contains_key("Bool::unary_not") {
                            errors.push(SemanticError::new(
                                "operator is not available: missing `internal func Bool::unary_not(...)`",
                                *span,
                            ));
                            return None;
                        }
                        errors.push(SemanticError::new(
                            "operator `!` requires a `Bool` operand",
                            *span,
                        ));
                        None
                    }
                }
                UnaryOp::BitNot => {
                    if t.as_ref().is_some_and(|tt| {
                        infer::unify_types(tt, &Ty::Int, &mut ctx.infer_ctx, errors, *span)
                    }) && ctx.registry.contains_key("Int::unary_bitwise_not") {
                        Some(Ty::Int)
                    } else {
                        if !ctx.registry.contains_key("Int::unary_bitwise_not") {
                            errors.push(SemanticError::new(
                                "operator is not available: missing `internal func Int::unary_bitwise_not(...)`",
                                *span,
                            ));
                            return None;
                        }
                        errors.push(SemanticError::new(
                            "unary `~` requires an `Int` operand",
                            *span,
                        ));
                        None
                    }
                }
                UnaryOp::Plus | UnaryOp::Minus => {
                    let method = match op {
                        UnaryOp::Plus => "unary_plus",
                        UnaryOp::Minus => "unary_minus",
                        _ => unreachable!("covered by outer match"),
                    };
                    match t.as_ref() {
                        // Avoid spurious "type mismatch expected X found Y" errors:
                        // if the operand already resolved to Float, don't try to unify it
                        // against Int first.
                        Some(Ty::Int) => {
                            if ctx.registry.contains_key(&format!("Int::{method}")) {
                                Some(Ty::Int)
                            } else {
                                errors.push(SemanticError::new(
                                    format!(
                                        "operator is not available: missing `internal func Int::{method}(...)`"
                                    ),
                                    *span,
                                ));
                                None
                            }
                        }
                        Some(Ty::Float) => {
                            if ctx.registry.contains_key(&format!("Float::{method}")) {
                                Some(Ty::Float)
                            } else {
                                errors.push(SemanticError::new(
                                    format!(
                                        "operator is not available: missing `internal func Float::{method}(...)`"
                                    ),
                                    *span,
                                ));
                                None
                            }
                        }
                        Some(Ty::InferVar(_)) => {
                            // Prefer Int if available, otherwise Float; otherwise reject.
                            if ctx.registry.contains_key(&format!("Int::{method}"))
                                && infer::unify_types(
                                    t.as_ref().unwrap(),
                                    &Ty::Int,
                                    &mut ctx.infer_ctx,
                                    errors,
                                    *span,
                                )
                            {
                                Some(Ty::Int)
                            } else if ctx.registry.contains_key(&format!("Float::{method}"))
                                && infer::unify_types(
                                    t.as_ref().unwrap(),
                                    &Ty::Float,
                                    &mut ctx.infer_ctx,
                                    errors,
                                    *span,
                                )
                            {
                                Some(Ty::Float)
                            } else {
                                errors.push(SemanticError::new(
                                    "unary `+` and `-` require an `Int` or `Float` operand",
                                    *span,
                                ));
                                None
                            }
                        }
                        _ => {
                            errors.push(SemanticError::new(
                                "unary `+` and `-` require an `Int` or `Float` operand",
                                *span,
                            ));
                            None
                        }
                    }
                }
            }
        }
        AstNode::Call {
            callee,
            type_args,
            arguments,
            span,
        } => {
            if type_args.is_empty() {
                if let Some(NameRes::Local(id)) = ctx.lookup(callee) {
                    if let Some(Ty::Function {
                        params,
                        param_names,
                        param_has_default,
                        ret,
                    }) = ctx.bindings_ty[id].clone()
                    {
                        let invoke_expr = AstNode::Invoke {
                            callee: Box::new(AstNode::Identifier {
                                name: callee.clone(),
                                span: *span,
                            }),
                            arguments: arguments.clone(),
                            span: *span,
                        };
                        let _ = (params, param_names, param_has_default); // shape checked above
                        let _ = ret;
                        return check_expr(&invoke_expr, ctx, errors);
                    }
                }
            }
            let ret = check_call(callee, type_args, arguments, *span, ctx, errors);
            ret
        }
        AstNode::Invoke {
            callee,
            arguments,
            span,
        } => {
            if let AstNode::Identifier { name, .. } = callee.as_ref() {
                if let Some(NameRes::Local(id)) = ctx.lookup(name) {
                    if let Some(Ty::Function {
                        params,
                        param_names,
                        param_has_default,
                        ret,
                    }) = ctx.bindings_ty[id].clone()
                    {
                        let mut new_params = params.clone();
                        let mut arg_values: Vec<&AstNode> = Vec::new();
                        for a in arguments {
                            match a {
                                CallArg::Positional(v) => arg_values.push(v),
                                CallArg::Named { value, .. } => arg_values.push(value),
                            }
                        }
                        for (i, a) in arg_values.iter().enumerate() {
                            if i >= new_params.len() {
                                break;
                            }
                            let got = check_expr(a, ctx, errors);
                            if let Some(got) = got {
                                let _ = infer::unify_types(
                                    &new_params[i],
                                    &got,
                                    &mut ctx.infer_ctx,
                                    errors,
                                    expr_span(a),
                                );
                                new_params[i] = infer::resolve_ty(&new_params[i], &ctx.infer_ctx);
                            }
                        }
                        ctx.bindings_ty[id] = Some(Ty::Function {
                            params: new_params,
                            param_names,
                            param_has_default,
                            ret,
                        });
                    }
                }
            }
            let Some(callee_ty_raw) = check_expr(callee, ctx, errors) else {
                return None;
            };
            let callee_ty = infer::resolve_ty(&callee_ty_raw, &ctx.infer_ctx);
            let (params, param_names, param_has_default, ret): (
                Vec<Ty>,
                Vec<Option<String>>,
                Vec<bool>,
                Box<Ty>,
            ) = match callee_ty {
                Ty::Function {
                    params,
                    param_names,
                    param_has_default,
                    ret,
                } => (params, param_names, param_has_default, ret),
                Ty::InferVar(_) => {
                    let params = (0..arguments.len())
                        .map(|_| ctx.infer_ctx.fresh_var())
                        .collect::<Vec<_>>();
                    let ret = Box::new(ctx.infer_ctx.fresh_var());
                    let fn_ty = Ty::Function {
                        params: params.clone(),
                        param_names: vec![None; params.len()],
                        param_has_default: vec![false; params.len()],
                        ret: ret.clone(),
                    };
                    let _ = infer::unify_types(
                        &callee_ty,
                        &fn_ty,
                        &mut ctx.infer_ctx,
                        errors,
                        *span,
                    );
                    (params, vec![None; arguments.len()], vec![false; arguments.len()], ret)
                }
                _ => {
                    errors.push(SemanticError::new(
                        format!("cannot call non-function value of type `{}`", ty_name(&callee_ty)),
                        *span,
                    ));
                    return None;
                }
            };
            let mut ordered: Vec<Option<&AstNode>> = vec![None; params.len()];
            let mut saw_named = false;
            let mut positional_idx = 0usize;
            for arg in arguments {
                match arg {
                    CallArg::Positional(expr) => {
                        if saw_named {
                            errors.push(SemanticError::new(
                                "positional arguments cannot follow named arguments",
                                expr_span(expr),
                            ));
                            continue;
                        }
                        if positional_idx >= params.len() {
                            errors.push(SemanticError::new(
                                format!(
                                    "call expects {} argument(s), found at least {}",
                                    params.len(),
                                    positional_idx + 1
                                ),
                                expr_span(expr),
                            ));
                            continue;
                        }
                        ordered[positional_idx] = Some(expr);
                        positional_idx += 1;
                    }
                    CallArg::Named {
                        name,
                        name_span,
                        value,
                    } => {
                        saw_named = true;
                        let Some(i) = param_names
                            .iter()
                            .position(|n| n.as_ref().is_some_and(|nn| nn == name))
                        else {
                            errors.push(SemanticError::new(
                                format!("unknown named argument `{name}`"),
                                *name_span,
                            ));
                            let _ = check_expr(value, ctx, errors);
                            continue;
                        };
                        if ordered[i].is_some() {
                            errors.push(SemanticError::new(
                                format!("duplicate argument `{name}`"),
                                *name_span,
                            ));
                        } else {
                            ordered[i] = Some(value);
                        }
                    }
                }
            }
            for (i, expected) in params.iter().enumerate() {
                if let Some(expr) = ordered[i] {
                    let got = check_expr_expected(expr, ctx, errors, Some(expected));
                    if let Some(got) = got {
                        let _ = infer::unify_types(
                            expected,
                            &got,
                            &mut ctx.infer_ctx,
                            errors,
                            expr_span(expr),
                        );
                    }
                } else if !param_has_default.get(i).copied().unwrap_or(false) {
                    errors.push(SemanticError::new(
                        format!("missing required argument {}", i + 1),
                        *span,
                    ));
                }
            }
            Some(infer::resolve_ty(ret.as_ref(), &ctx.infer_ctx))
        }
        AstNode::MethodCall {
            receiver,
            method,
            arguments,
            span,
        } => {
            let Some(recv_ty) = check_expr(receiver.as_ref(), ctx, errors) else {
                return None;
            };
            let Some(recv_key) = ty_to_receiver_key(&recv_ty) else {
                errors.push(SemanticError::new(
                    format!("no extension methods are available for `{}`", ty_name(&recv_ty)),
                    *span,
                ));
                return None;
            };
            let mut callee = format!("{recv_key}::{method}");
            let mut sig = ctx.registry.get(&callee);
            if sig.is_none() {
                if matches!(recv_ty, Ty::Array(_)) {
                    if let Some(candidates) = ctx.generic_array_ext.get(method) {
                        for g in candidates {
                            let mut substs = HashMap::new();
                            let mut probe_errors = Vec::new();
                            if let Some(s) = ctx.registry.get(g) {
                                if infer::infer_generic_from_template(
                                    &s.params[0],
                                    &recv_ty,
                                    &mut substs,
                                    &mut probe_errors,
                                    *span,
                                ) && s.type_params.iter().all(|tp| substs.contains_key(&tp.name))
                                    && probe_errors.is_empty()
                                {
                                    callee = g.clone();
                                    sig = Some(s);
                                    break;
                                }
                            }
                        }
                    }
                }
                if sig.is_none() && matches!(recv_ty, Ty::Enum { .. }) {
                    if let Some(candidates) = ctx.generic_enum_ext.get(method) {
                        for g in candidates {
                            let mut substs = HashMap::new();
                            let mut probe_errors = Vec::new();
                            if let Some(s) = ctx.registry.get(g) {
                                if infer::infer_generic_from_template(
                                    &s.params[0],
                                    &recv_ty,
                                    &mut substs,
                                    &mut probe_errors,
                                    *span,
                                ) && s.type_params.iter().all(|tp| substs.contains_key(&tp.name))
                                    && probe_errors.is_empty()
                                {
                                    callee = g.clone();
                                    sig = Some(s);
                                    break;
                                }
                            }
                        }
                    }
                }
                if sig.is_none() && matches!(recv_ty, Ty::Struct(_)) {
                    if let Some(candidates) = ctx.generic_struct_ext.get(method) {
                        for g in candidates {
                            let mut substs = HashMap::new();
                            let mut probe_errors = Vec::new();
                            if let Some(s) = ctx.registry.get(g) {
                                // `infer_generic_from_template` doesn't understand
                                // `Ty::Struct(...)` yet, so for struct-receiver templates we
                                // infer generic type parameters by parsing the inst name,
                                // e.g. `Dict<K, V>` vs `Dict<String, Float>`.
                                let ok = match (&s.params[0], &recv_ty) {
                                    (Ty::Struct(tmpl_inst), Ty::Struct(got_inst)) => {
                                        let tmpl_base = struct_base_name(tmpl_inst);
                                        let got_base = struct_base_name(got_inst);
                                        if tmpl_base != got_base {
                                            false
                                        } else {
                                            let parse_inst_args =
                                                |inst: &str| -> Option<Vec<String>> {
                                                    let (_, rest) = inst.split_once('<')?;
                                                    let inner = rest.strip_suffix('>')?;
                                                    let mut out = Vec::new();
                                                    let mut buf = String::new();
                                                    let mut depth = 0usize;
                                                    for ch in inner.chars() {
                                                        match ch {
                                                            '<' => {
                                                                depth += 1;
                                                                buf.push(ch);
                                                            }
                                                            '>' => {
                                                                depth = depth.saturating_sub(1);
                                                                buf.push(ch);
                                                            }
                                                            ',' if depth == 0 => {
                                                                let part = buf.trim();
                                                                if !part.is_empty() {
                                                                    out.push(part.to_string());
                                                                }
                                                                buf.clear();
                                                            }
                                                            _ => buf.push(ch),
                                                        }
                                                    }
                                                    let part = buf.trim();
                                                    if !part.is_empty() {
                                                        out.push(part.to_string());
                                                    }
                                                    Some(out)
                                                };

                                            if let (Some(tmpl_args), Some(got_args)) =
                                                (parse_inst_args(tmpl_inst),
                                                 parse_inst_args(got_inst))
                                            {
                                                if tmpl_args.len() != got_args.len() {
                                                    false
                                                } else {
                                                    let mut ok = true;
                                                    for (tmpl_arg, got_arg) in
                                                        tmpl_args.iter().zip(got_args.iter())
                                                    {
                                                        let tmpl_arg_str = tmpl_arg.as_str();
                                                        if let Some(tp) = s
                                                            .type_params
                                                            .iter()
                                                            .find(|tp| tp.name == tmpl_arg_str)
                                                        {
                                                            let te = match crate::parser::Parser
                                                                ::parse_type_expr_from_source(
                                                                    got_arg,
                                                                )
                                                            {
                                                                Ok(te) => te,
                                                                Err(_) => {
                                                                    ok = false;
                                                                    break;
                                                                }
                                                            };

                                                            let inferred_ty = ty_from_type_expr(
                                                                &te,
                                                                *span,
                                                                &mut probe_errors,
                                                                &[],
                                                                &ctx.structs,
                                                                &ctx.enums,
                                                                ctx.aliases,
                                                            );
                                                            if inferred_ty.is_none() {
                                                                ok = false;
                                                                break;
                                                            }
                                                            substs.insert(
                                                                tp.name.clone(),
                                                                inferred_ty.unwrap(),
                                                            );
                                                        }
                                                    }

                                                    ok
                                                        && s.type_params
                                                            .iter()
                                                            .all(|tp| substs.contains_key(&tp.name))
                                                        && probe_errors.is_empty()
                                                }
                                            } else {
                                                false
                                            }
                                        }
                                    }
                                    _ => false,
                                };

                                if ok {
                                    callee = g.clone();
                                    sig = Some(s);
                                    break;
                                }
                            }
                        }
                    }
                }
            }
            let Some(sig) = sig else {
                errors.push(SemanticError::new(
                    format!("no extension method `{method}` for `{}`", ty_name(&recv_ty)),
                    *span,
                ));
                return None;
            };
            let is_instance_method = if sig.params.is_empty() {
                false
            } else if sig.type_params.is_empty() {
                sig.params[0] == recv_ty
            } else {
                let mut substs = HashMap::new();
                let mut probe_errors = Vec::new();

                // `infer_generic_from_template` doesn't support `Ty::Struct(...)` templates.
                // For struct-receiver extensions, infer by parsing the inst name strings,
                // e.g. `Dict<K, V>` vs `Dict<String, Float>`.
                let ok = match (&sig.params[0], &recv_ty) {
                    (Ty::Struct(tmpl_inst), Ty::Struct(got_inst)) => {
                        let tmpl_base = struct_base_name(tmpl_inst);
                        let got_base = struct_base_name(got_inst);
                        if tmpl_base != got_base {
                            false
                        } else {
                            let parse_inst_args = |inst: &str| -> Option<Vec<String>> {
                                let Some((_, rest)) = inst.split_once('<')
                                else {
                                    return Some(Vec::new());
                                };
                                let inner = rest.strip_suffix('>')?;
                                let mut out = Vec::new();
                                let mut buf = String::new();
                                let mut depth = 0usize;
                                for ch in inner.chars() {
                                    match ch {
                                        '<' => {
                                            depth += 1;
                                            buf.push(ch);
                                        }
                                        '>' => {
                                            depth = depth.saturating_sub(1);
                                            buf.push(ch);
                                        }
                                        ',' if depth == 0 => {
                                            let part = buf.trim();
                                            if !part.is_empty() {
                                                out.push(part.to_string());
                                            }
                                            buf.clear();
                                        }
                                        _ => buf.push(ch),
                                    }
                                }
                                let part = buf.trim();
                                if !part.is_empty() {
                                    out.push(part.to_string());
                                }
                                Some(out)
                            };

                            if let (Some(tmpl_args), Some(got_args)) =
                                (parse_inst_args(tmpl_inst), parse_inst_args(got_inst))
                            {
                                if tmpl_args.len() != got_args.len() {
                                    false
                                } else {
                                    let mut ok = true;
                                    for (tmpl_arg, got_arg) in
                                        tmpl_args.iter().zip(got_args.iter())
                                    {
                                        let tmpl_arg = tmpl_arg.as_str();
                                        let Some(tp) = sig
                                            .type_params
                                            .iter()
                                            .find(|tp| tp.name == tmpl_arg)
                                        else {
                                            continue;
                                        };

                                    let te =
                                        match crate::parser::Parser
                                            ::parse_type_expr_from_source(got_arg)
                                        {
                                            Ok(te) => te,
                                            Err(_) => {
                                                ok = false;
                                                break;
                                            }
                                        };

                                    let inferred_ty = ty_from_type_expr(
                                        &te,
                                        *span,
                                        &mut probe_errors,
                                        &[],
                                        &ctx.structs,
                                        &ctx.enums,
                                        ctx.aliases,
                                    );
                                    let Some(inferred_ty) = inferred_ty else {
                                        ok = false;
                                        break;
                                    };
                                        substs.insert(tp.name.clone(), inferred_ty);
                                    }

                                    ok && probe_errors.is_empty()
                                }
                            } else {
                                false
                            }
                        }
                    }
                    _ => infer::infer_generic_from_template(
                        &sig.params[0],
                        &recv_ty,
                        &mut substs,
                        &mut probe_errors,
                        *span,
                    ) && probe_errors.is_empty(),
                };

                ok
            };
            if !is_instance_method {
                // Convenience for unit structs: allow `None.foo()` as shorthand for `None::foo()`
                // when `foo` is a static extension method.
                if sig.params.is_empty() {
                    if let AstNode::Identifier { name, .. } = receiver.as_ref() {
                        if ctx.lookup(name).is_none()
                            && ctx.structs.get(name).is_some_and(|s| s.is_unit)
                        {
                            return check_call(&callee, &[], arguments, *span, ctx, errors);
                        }
                    }
                }
                errors.push(SemanticError::new(
                    format!("`{method}` is static and cannot be called as an instance method"),
                    *span,
                ));
                return None;
            }
            let mut call_args = Vec::with_capacity(arguments.len() + 1);
            call_args.push(CallArg::Positional((**receiver).clone()));
            call_args.extend(arguments.iter().cloned());
            check_call(&callee, &[], &call_args, *span, ctx, errors)
        }
        AstNode::TypeMethodCall {
            type_name,
            method,
            arguments,
            span,
        } => {
            let mut callee = format!("{type_name}::{method}");
            if !ctx.registry.contains_key(&callee) {
                if type_name.starts_with('[') && type_name.ends_with(']') {
                    if let Some(candidates) = ctx.generic_array_ext.get(method) {
                        if let Ok(te) = crate::parser::Parser::parse_type_expr_from_source(type_name)
                        {
                            if let Some(recv_ty) = ty_from_type_expr(
                                &te,
                                *span,
                                errors,
                                &[],
                                &ctx.structs,
                                &ctx.enums,
                                ctx.aliases,
                            ) {
                                for g in candidates {
                                    let mut substs = HashMap::new();
                                    let mut probe_errors = Vec::new();
                                    if let Some(s) = ctx.registry.get(g) {
                                        let tmpl = s
                                            .extension_receiver_ty
                                            .as_ref()
                                            .unwrap_or(&s.params[0]);
                                        if infer::infer_generic_from_template(
                                            tmpl,
                                            &recv_ty,
                                            &mut substs,
                                            &mut probe_errors,
                                            *span,
                                        ) && s.type_params.iter().all(|tp| substs.contains_key(&tp.name))
                                            && probe_errors.is_empty()
                                        {
                                            callee = g.clone();
                                            break;
                                        }
                                    }
                                }
                            }
                        }
                    }
                } else if let Ok(te) = crate::parser::Parser::parse_type_expr_from_source(type_name) {
                    if matches!(&te, TypeExpr::EnumApp { .. }) {
                        if let Some(recv_ty) = ty_from_type_expr(
                            &te,
                            *span,
                            errors,
                            &[],
                            &ctx.structs,
                            &ctx.enums,
                            ctx.aliases,
                        ) {
                            if let Some(candidates) = ctx.generic_enum_ext.get(method) {
                                for g in candidates {
                                    let mut substs = HashMap::new();
                                    let mut probe_errors = Vec::new();
                                    if let Some(s) = ctx.registry.get(g) {
                                        let tmpl = s
                                            .extension_receiver_ty
                                            .as_ref()
                                            .unwrap_or(&s.params[0]);
                                        if infer::infer_generic_from_template(
                                            tmpl,
                                            &recv_ty,
                                            &mut substs,
                                            &mut probe_errors,
                                            *span,
                                        ) && s.type_params.iter().all(|tp| substs.contains_key(&tp.name))
                                            && probe_errors.is_empty()
                                        {
                                            callee = g.clone();
                                            break;
                                        }
                                    }
                                }
                            }
                        }
                    }
                    if let TypeExpr::Named(ref n) = te {
                        if ctx.structs.contains_key(n) {
                            let recv_ty = ty_from_type_expr(
                                &te,
                                *span,
                                errors,
                                &[],
                                &ctx.structs,
                                &ctx.enums,
                                ctx.aliases,
                            )
                            .unwrap_or_else(|| Ty::Struct(n.clone()));
                            if let Some(candidates) = ctx.generic_struct_ext.get(method) {
                                for g in candidates {
                                    let mut substs = HashMap::new();
                                    let mut probe_errors = Vec::new();
                                    if let Some(s) = ctx.registry.get(g) {
                                        let tmpl = s
                                            .extension_receiver_ty
                                            .as_ref()
                                            .unwrap_or(&s.params[0]);
                                        if infer::infer_generic_from_template(
                                            tmpl,
                                            &recv_ty,
                                            &mut substs,
                                            &mut probe_errors,
                                            *span,
                                        ) && s.type_params.iter().all(|tp| substs.contains_key(&tp.name))
                                            && probe_errors.is_empty()
                                        {
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
            if ctx.registry.contains_key(&callee) {
                return check_call(&callee, &[], arguments, *span, ctx, errors);
            }
            if !ctx.enums.contains_key(type_name)
                && let Ok(mut te) = crate::parser::Parser::parse_type_expr_from_source(type_name)
            {
                if let TypeExpr::Named(n) = &te {
                    if let Some(ad) = ctx.aliases.get(n) {
                        if !ad.type_params.is_empty() {
                            te = TypeExpr::EnumApp {
                                name: n.clone(),
                                args: vec![TypeExpr::Infer; ad.type_params.len()],
                            };
                        }
                    }
                }
                if let Some(Ty::Enum { name, args }) =
                    ty_from_type_expr(&te, *span, errors, &[], &ctx.structs, &ctx.enums, ctx.aliases)
                {
                    if let Some(def) = ctx.enums.get(&name) {
                        if def.variants.contains_key(method) {
                            if arguments.iter().any(|a| matches!(a, CallArg::Named { .. })) {
                                errors.push(SemanticError::new(
                                    "enum variant constructor does not support named arguments",
                                    *span,
                                ));
                                return None;
                            }
                            fn ty_to_type_expr(ty: &Ty) -> TypeExpr {
                                match ty {
                                    Ty::Int => TypeExpr::Named("Int".to_string()),
                                    Ty::Float => TypeExpr::Named("Float".to_string()),
                                    Ty::String => TypeExpr::Named("String".to_string()),
                                    Ty::Bool => TypeExpr::Named("Bool".to_string()),
                                    Ty::Any | Ty::InferVar(_) => TypeExpr::Infer,
                                    Ty::Unit => TypeExpr::Unit,
                                    Ty::Array(inner) => TypeExpr::Array(Box::new(ty_to_type_expr(inner))),
                                    Ty::Tuple(parts) => {
                                        TypeExpr::Tuple(parts.iter().map(ty_to_type_expr).collect())
                                    }
                                    Ty::Enum { name, args } => TypeExpr::EnumApp {
                                        name: name.clone(),
                                        args: args.iter().map(ty_to_type_expr).collect(),
                                    },
                                    Ty::Struct(name) => TypeExpr::Named(name.clone()),
                                    Ty::TypeParam(name) => TypeExpr::TypeParam(name.clone()),
                                    Ty::Function { .. } => TypeExpr::Infer,
                                    Ty::Task(inner) => TypeExpr::EnumApp {
                                        name: "Task".to_string(),
                                        args: vec![ty_to_type_expr(inner)],
                                    },
                                }
                            }
                            let payloads = arguments
                                .iter()
                                .map(|a| match a {
                                    CallArg::Positional(v) => v.clone(),
                                    CallArg::Named { value, .. } => value.clone(),
                                })
                                .collect::<Vec<_>>();
                            let as_ctor = AstNode::EnumVariantCtor {
                                enum_name: name.clone(),
                                type_args: args.iter().map(ty_to_type_expr).collect(),
                                variant: method.clone(),
                                payloads,
                                span: *span,
                            };
                            return check_expr(&as_ctor, ctx, errors);
                        }
                    }
                }
            }
            if let Some(def) = ctx.enums.get(type_name) {
                if def.variants.contains_key(method) {
                    if arguments.iter().any(|a| matches!(a, CallArg::Named { .. })) {
                        errors.push(SemanticError::new(
                            "enum variant constructor does not support named arguments",
                            *span,
                        ));
                        return None;
                    }
                    let payloads = arguments
                        .iter()
                        .map(|a| match a {
                            CallArg::Positional(v) => v.clone(),
                            CallArg::Named { value, .. } => value.clone(),
                        })
                        .collect::<Vec<_>>();
                    let as_ctor = AstNode::EnumVariantCtor {
                        enum_name: type_name.clone(),
                        type_args: Vec::new(),
                        variant: method.clone(),
                        payloads,
                        span: *span,
                    };
                    return check_expr(&as_ctor, ctx, errors);
                }
            }
            errors.push(SemanticError::new(
                format!("unknown static method `{type_name}::{method}`"),
                *span,
            ));
            None
        }
        _ => {
            errors.push(SemanticError::new(
                "invalid expression in this context",
                expr_span(expr),
            ));
            None
        }
    }
}

fn expr_span(expr: &AstNode) -> Span {
    match expr {
        AstNode::IntegerLiteral { span, .. }
        | AstNode::FloatLiteral { span, .. }
        | AstNode::StringLiteral { span, .. }
        | AstNode::Identifier { span, .. }
        | AstNode::BinaryOp { span, .. }
        | AstNode::UnaryOp { span, .. }
        | AstNode::Call { span, .. }
        | AstNode::MethodCall { span, .. }
        | AstNode::TypeMethodCall { span, .. }
        | AstNode::UnitLiteral { span, .. }
        | AstNode::TupleLiteral { span, .. }
        | AstNode::TupleField { span, .. }
        | AstNode::ArrayLiteral { span, .. }
        | AstNode::ArrayIndex { span, .. }
        | AstNode::StructLiteral { span, .. }
        | AstNode::FieldAccess { span, .. }
        | AstNode::EnumVariantCtor { span, .. }
        | AstNode::BoolLiteral { span, .. }
        | AstNode::Await { span, .. } => *span,
        _ => Span::new(1, 1, 1),
    }
}

fn ty_name(t: &Ty) -> String {
    match t {
        Ty::Int => "Int".to_string(),
        Ty::Float => "Float".to_string(),
        Ty::String => "String".to_string(),
        Ty::Bool => "Bool".to_string(),
        Ty::Unit => "()".to_string(),
        Ty::Tuple(parts) => {
            let inner: Vec<String> = parts.iter().map(ty_name).collect();
            format!("({})", inner.join(", "))
        }
        Ty::Array(elem) => format!("[{}]", ty_name(elem)),
        Ty::Any => "Any".to_string(),
        Ty::TypeParam(n) => n.clone(),
        Ty::Struct(name) => name.clone(),
        Ty::Enum { name, args } => {
            let inner: Vec<String> = args.iter().map(ty_name).collect();
            format!("{name}<{}>", inner.join(", "))
        }
        Ty::Function {
            params,
            param_names,
            param_has_default: _,
            ret,
        } => {
            let mut p = Vec::new();
            for (i, ty) in params.iter().enumerate() {
                if let Some(Some(name)) = param_names.get(i) {
                    p.push(format!("{name}: {}", ty_name(ty)));
                } else {
                    p.push(ty_name(ty));
                }
            }
            format!("({}) => {}", p.join(", "), ty_name(ret))
        }
        Ty::InferVar(_) => "an inferred type".to_string(),
        Ty::Task(inner) => format!("Task<{}>", ty_name(inner)),
    }
}

fn struct_base_name(name: &str) -> &str {
    name.split_once('<').map(|(b, _)| b).unwrap_or(name)
}

fn is_operator_method_name(name: &str) -> bool {
    let Some((_, method)) = name.split_once("::") else {
        return false;
    };
    matches!(
        method,
        "binary_add"
            | "binary_sub"
            | "binary_mul"
            | "binary_div"
            | "binary_mod"
            | "binary_bitwise_and"
            | "binary_bitwise_or"
            | "binary_bitwise_xor"
            | "binary_left_shift"
            | "binary_right_shift"
            | "compare_less"
            | "compare_less_or_equal"
            | "compare_greater"
            | "compare_greater_or_equal"
            | "compare_equal"
            | "compare_not_equal"
            | "binary_and"
            | "binary_or"
            | "unary_plus"
            | "unary_minus"
            | "unary_not"
            | "unary_bitwise_not"
    )
}

fn mangle_operator_overload_name(name: &str, params: &[Ty]) -> String {
    if !is_operator_method_name(name) {
        return name.to_string();
    }
    let sig = params.iter().map(ty_name).collect::<Vec<_>>().join("|");
    format!("{name}#op({sig})")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::Lexer;
    use crate::parser::Parser;

    fn core_operator_prelude() -> String {
        let core = include_str!("../std/core.vc");
        let cut = core
            .find("export enum Option")
            .expect("std/core.vc must contain `export enum Option`");
        core[..cut].to_string()
    }

    fn check_src_with_prelude(prelude: &str, src: &str) -> Vec<SemanticError> {
        let full = format!("{prelude}\n{src}");
        let mut lexer = Lexer::new(&full);
        let tokens = lexer.tokenize().expect("lex");
        let mut parser = Parser::new(tokens);
        let ast = parser.parse().expect("parse");
        check_program(&ast)
    }

    fn check_src(src: &str) -> Vec<SemanticError> {
        let prelude = core_operator_prelude();
        check_src_with_prelude(&prelude, src)
    }

    #[test]
    fn example4_passes() {
        let src = include_str!("../examples/example4.vc");
        let errs = check_src(src);
        assert!(errs.is_empty(), "{:?}", errs);
    }

    #[test]
    fn example5_passes() {
        let src = include_str!("../examples/example5.vc");
        let errs = check_src(src);
        assert!(errs.is_empty(), "{:?}", errs);
    }

    #[test]
    fn example7_passes() {
        let src = include_str!("../examples/example7.vc");
        let errs = check_src(src);
        assert!(errs.is_empty(), "{:?}", errs);
    }

    #[test]
    fn example13_arrow_function_infers_return_type() {
        let src = include_str!("../examples/example13.vc");
        let errs = check_src(src);
        assert!(errs.is_empty(), "{:?}", errs);
    }

    #[test]
    fn example14_enum_types() {
        let src = include_str!("../examples/example14.vc");
        let errs = check_src(src);
        assert!(errs.is_empty(), "{:?}", errs);
    }

    #[test]
    fn example15_enum_construction_syntax() {
        let src = include_str!("../examples/example15.vc");
        let errs = check_src(src);
        assert!(errs.is_empty(), "{:?}", errs);
    }

    #[test]
    fn example16_match_basic() {
        let src = r#"
internal func print_gen<T>(t: T);

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
}
"#;
        let errs = check_src(src);
        assert!(errs.is_empty(), "{:?}", errs);
    }

    #[test]
    fn string_equality_ok() {
        let errs = check_src(r#"func main() { let _ = "a" == "b"; }"#);
        assert!(errs.is_empty(), "{:?}", errs);
    }

    #[test]
    fn string_equality_requires_compare_equal_decl() {
        let prelude = core_operator_prelude().replace(
            "internal func String::compare_equal(self, other: String): Bool; // String == String\n",
            "",
        );
        let errs = check_src_with_prelude(&prelude, r#"func main() { let _ = "a" == "b"; }"#);
        assert!(
            errs.iter().any(|e| e.message.contains("String::compare_equal")),
            "{:?}",
            errs
        );
    }

    #[test]
    fn tuple_equality_ok() {
        let errs = check_src("func main() { let _ = (1, 2) == (1, 2); }");
        assert!(errs.is_empty(), "{:?}", errs);
    }

    #[test]
    fn any_equality_rejected() {
        let errs = check_src("func main() { let x: Any = 1; let _ = x == 1; }");
        assert!(
            errs.iter().any(|e| e.message.contains("Any")),
            "{:?}",
            errs
        );
    }

    #[test]
    fn float_arithmetic_and_ordering_ok() {
        let errs = check_src(
            r#"func main() {
                let _: Float = 1.0 + 2.0;
                let _: Bool = 1.0 < 2.0;
            }"#,
        );
        assert!(errs.is_empty(), "{:?}", errs);
    }

    #[test]
    fn float_bitnot_rejected() {
        let errs = check_src("func main() { let _ = ~1.0; }");
        assert!(
            errs.iter().any(|e| e.message.contains("unary `~`") || e.message.contains("BitNot")),
            "{:?}",
            errs
        );
    }

    #[test]
    fn float_string_plus_rejected() {
        let errs = check_src(r#"func main() { let _ = ("a" + 1.0); }"#);
        assert!(
            errs.iter().any(|e| e.message.contains("operator `+`") && e.message.contains("Float")),
            "{:?}",
            errs
        );
    }

    #[test]
    fn int_plus_requires_binary_add_decl() {
        let prelude = core_operator_prelude().replace(
            "internal func Int::binary_add(self, other: Int): Int; // Int + Int\n",
            "",
        );
        let errs = check_src_with_prelude(&prelude, "func main() { let _ = 1 + 2; }");
        assert!(
            errs.iter().any(|e| e.message.contains("Int::binary_add")),
            "{:?}",
            errs
        );
    }

    #[test]
    fn bool_equality_requires_compare_equal_decl() {
        let prelude = core_operator_prelude().replace(
            "internal func Bool::compare_equal(self, other: Bool): Bool; // Bool == Bool\n",
            "",
        );
        let errs = check_src_with_prelude(&prelude, "func main() { let _ = true == true; }");
        assert!(
            errs.iter().any(|e| e.message.contains("Bool::compare_equal")),
            "{:?}",
            errs
        );
    }

    #[test]
    fn bool_and_requires_binary_and_decl() {
        let prelude = core_operator_prelude().replace(
            "internal func Bool::binary_and(self, other: Bool): Bool; // Bool && Bool\n",
            "",
        );
        let errs = check_src_with_prelude(&prelude, "func main() { let _ = true && false; }");
        assert!(
            errs.iter().any(|e| e.message.contains("Bool::binary_and")),
            "{:?}",
            errs
        );
    }

    #[test]
    fn bool_not_requires_unary_not_decl() {
        let prelude = core_operator_prelude().replace(
            "internal func Bool::unary_not(self): Bool; // !Bool\n",
            "",
        );
        let errs = check_src_with_prelude(&prelude, "func main() { let _ = !true; }");
        assert!(
            errs.iter().any(|e| e.message.contains("Bool::unary_not")),
            "{:?}",
            errs
        );
    }

    #[test]
    fn custom_struct_operator_overload_by_rhs_type() {
        let errs = check_src(
            r#"struct Foo;
               func Foo::compare_greater(self, other: Float): Bool { return false; }
               func Foo::compare_greater(self, other: Int): Bool { return true; }
               func Foo::binary_add(self, other: Bool): Bool { return other; }
               func main() {
                   let a: Bool = Foo > 1.5;
                   let b: Bool = Foo > 1;
                   let c: Bool = Foo + true;
               }"#,
        );
        assert!(errs.is_empty(), "{:?}", errs);
    }

    #[test]
    fn custom_struct_operator_missing_overload_rejected() {
        let errs = check_src(
            r#"struct Foo;
               func Foo::compare_greater(self, other: Int): Bool { return true; }
               func main() { let _ = Foo > true; }"#,
        );
        assert!(
            errs.iter().any(|e| e.message.contains("no overload for operator method")),
            "{:?}",
            errs
        );
    }

    #[test]
    fn task_comparisons_rejected() {
        let errs = check_src(
            r#"struct Task<T = ()>;
               internal async func sleep(ms: Int): Task;
               async func main(): Task {
                   let t = sleep(0);
                   let _ = t == t;
                   return await sleep(0);
               }"#,
        );
        assert!(
            errs.iter().any(|e| e.message.contains("Task") && e.message.contains("operators")),
            "{:?}",
            errs
        );
    }

    #[test]
    fn any_passed_to_print_any_ok() {
        let errs = check_src("internal func print_any(a: Any);\nfunc main() { let x: Any = 1; print_any(x); }");
        assert!(errs.is_empty(), "{:?}", errs);
    }

    #[test]
    fn any_to_int_param_rejected() {
        let errs = check_src(
            "internal func print_int(v: Int);\nfunc main() { let x: Any = 1; print_int(x); }",
        );
        assert!(
            errs.iter().any(|e| e.message.contains("expected `Int`") || e.message.contains("print_int")),
            "{:?}",
            errs
        );
    }

    #[test]
    fn unit_equality_ok() {
        let errs = check_src("func main() { let _ = () == (); }");
        assert!(errs.is_empty(), "{:?}", errs);
    }

    #[test]
    fn cross_type_equality_rejected() {
        let errs = check_src("func main() { let _ = 1 == true; }");
        assert!(
            errs.iter()
                .any(|e| e.message.contains("same type") || e.message.contains("equality")),
            "{:?}",
            errs
        );
    }

    #[test]
    fn string_ordering_rejected() {
        let errs = check_src(r#"func main() { let _ = "a" < "b"; }"#);
        assert!(errs.iter().any(|e| e.message.contains("Int")), "{:?}", errs);
    }

    #[test]
    fn shift_int_ok() {
        let errs = check_src("func main() { let _ = 1 << 3; let _ = 16 >> 2; }");
        assert!(errs.is_empty(), "{:?}", errs);
    }

    #[test]
    fn shift_requires_int_operands() {
        let errs = check_src("func main() { let _ = \"a\" << 1; }");
        assert!(errs.iter().any(|e| e.message.contains("shift operators")), "{:?}", errs);
    }

    #[test]
    fn generic_operator_rejected_without_constraints() {
        let errs = check_src(
            r#"func add_gen<T>(x: T, y: T): T { return x + y; }
               func main() { let _ = add_gen<Int>(1, 2); }"#,
        );
        assert!(
            errs.iter()
                .any(|e| e.message.contains("generic type parameters")),
            "{:?}",
            errs
        );
    }

    #[test]
    fn any_shift_rejected() {
        let errs = check_src(
            "internal func print_any(a: Any);\nfunc main() { let x: Any = 1; let _ = x << 1; }",
        );
        assert!(
            errs.iter()
                .any(|e| e.message.contains("shift operators") || e.message.contains("Any")),
            "{:?}",
            errs
        );
    }

    #[test]
    fn if_condition_must_be_bool() {
        let errs = check_src("func main() { if 1 { } }");
        assert!(errs.iter().any(|e| e.message.contains("Bool")), "{:?}", errs);
    }

    #[test]
    fn logical_ops_require_bool() {
        let errs = check_src("func main() { let _ = 1 && 2; }");
        assert!(
            errs.iter().any(|e| e.message.contains("logical")),
            "{:?}",
            errs
        );
    }

    #[test]
    fn bang_requires_bool() {
        let errs = check_src("func main() { let _ = !1; }");
        assert!(errs.iter().any(|e| e.message.contains("!")), "{:?}", errs);
    }

    #[test]
    fn bool_return_ok() {
        let errs = check_src("func f(): Bool { return true; }\nfunc main() { let _ = f(); }");
        assert!(errs.is_empty(), "{:?}", errs);
    }

    #[test]
    fn missing_main() {
        let errs = check_src("internal func f(): Int;\nfunc g(): Int { return 1; }");
        assert!(errs.iter().any(|e| e.message.contains("main")));
    }

    #[test]
    fn duplicate_top_level_func() {
        let errs = check_src("func a() {}\nfunc a() {}\nfunc main() {}");
        assert!(errs.iter().any(|e| e.message.contains("redefinition")));
    }

    #[test]
    fn func_conflicts_with_internal() {
        let errs = check_src(
            "internal func foo(): Int;\nfunc foo() {}\nfunc main() {}",
        );
        assert!(errs.iter().any(|e| e.message.contains("redefinition")));
    }

    #[test]
    fn main_with_params_rejected() {
        let errs = check_src("func main(x: Int) {}");
        assert!(errs.iter().any(|e| e.message.contains("main")));
    }

    #[test]
    fn main_non_unit_return_type_rejected() {
        let errs = check_src("func main(): Int { return 1; }");
        assert!(errs.iter().any(|e| e.message.contains("main")));
    }

    #[test]
    fn main_unit_return_ok() {
        let errs = check_src("func main(): () { }");
        assert!(errs.is_empty(), "{:?}", errs);
    }

    #[test]
    fn string_plus_int_rejected() {
        let errs = check_src(
            r#"internal func f(s: String, i: Int): Int;
func main() { f("a" + 1, 0); }"#,
        );
        assert!(errs.iter().any(|e| e.message.contains("+")));
    }

    #[test]
    fn typed_function_missing_return() {
        let errs = check_src(
            "internal func p(s: String);\nfunc f(): Int { p(\"x\"); }\nfunc main() {}",
        );
        assert!(errs.iter().any(|e| e.message.contains("fall off")));
    }

    #[test]
    fn return_value_in_void_func() {
        let errs = check_src("func main() { return 1; }");
        assert!(errs.iter().any(|e| e.message.contains("no return type")));
    }

    #[test]
    fn bare_return_in_typed_func() {
        let errs = check_src("func f(): Int { return; }\nfunc main() {}");
        assert!(errs.iter().any(|e| e.message.contains("no value")));
    }

    #[test]
    fn unknown_type_in_signature() {
        let errs = check_src("func f(): UnknownT { return 1; }\nfunc main() {}");
        assert!(errs.iter().any(|e| e.message.contains("unknown type")));
    }

    #[test]
    fn call_wrong_arity() {
        let errs = check_src("func f(a: Int) {}\nfunc main() { f(); }");
        assert!(errs.iter().any(|e| e.message.contains("expects")));
    }

    #[test]
    fn call_wrong_arg_type() {
        let errs = check_src("func f(a: Int) {}\nfunc main() { f(\"x\"); }");
        assert!(errs.iter().any(|e| e.message.contains("argument")));
    }

    #[test]
    fn example17_defaults_and_named_args() {
        let errs = check_src(include_str!("../examples/example17.vc"));
        assert!(errs.is_empty(), "{:?}", errs);
    }

    #[test]
    fn named_call_duplicate_argument_rejected() {
        let errs = check_src(
            r#"func bar(a: Int, b: Bool = true) {}
               func main() { bar(b: true, b: false, a: 1); }"#,
        );
        assert!(errs.iter().any(|e| e.message.contains("duplicate argument `b`")), "{:?}", errs);
    }

    #[test]
    fn named_call_unknown_argument_rejected() {
        let errs = check_src(
            r#"func bar(a: Int, b: Bool = true) {}
               func main() { bar(x: true, a: 1); }"#,
        );
        assert!(
            errs.iter()
                .any(|e| e.message.contains("unknown named argument `x`")),
            "{:?}",
            errs
        );
    }

    #[test]
    fn named_then_positional_rejected() {
        let errs = check_src(
            r#"func bar(a: Int, b: Bool = true) {}
               func main() { bar(b: false, 17); }"#,
        );
        assert!(
            errs.iter()
                .any(|e| e.message.contains("positional arguments cannot follow named arguments")),
            "{:?}",
            errs
        );
    }

    #[test]
    fn missing_required_arg_with_named_others_rejected() {
        let errs = check_src(
            r#"func bar(a: Int, b: Bool = true) {}
               func main() { bar(b: false); }"#,
        );
        assert!(
            errs.iter().any(|e| e.message.contains("expects 2 argument(s), found 1")),
            "{:?}",
            errs
        );
    }

    #[test]
    fn params_autopack_and_empty_ok() {
        let errs = check_src(
            r#"internal func int_array_len(a: [Int]): Int;
               func sum(params numbers: [Int]): Int {
                   let i = 0;
                   let out = 0;
                   while int_array_len(numbers) > i { out += numbers[i]; i += 1; }
                   return out;
               }
               func main() {
                   let _ = sum();
                   let _ = sum(1, 2, 3);
               }"#,
        );
        assert!(errs.is_empty(), "{:?}", errs);
    }

    #[test]
    fn params_packed_type_mismatch_rejected() {
        let errs = check_src(
            r#"func sum(params numbers: [Int]): Int { return 0; }
               func main() { let _ = sum(1, true); }"#,
        );
        assert!(
            errs.iter().any(|e| e.message.contains("packed argument for `params`")),
            "{:?}",
            errs
        );
    }

    #[test]
    fn params_named_explicit_array_ok() {
        let errs = check_src(
            r#"internal func int_array_len(a: [Int]): Int;
               func sum(params numbers: [Int]): Int { return int_array_len(numbers); }
               func main() { let _ = sum(numbers: [1, 2, 3]); }"#,
        );
        assert!(errs.is_empty(), "{:?}", errs);
    }

    #[test]
    fn params_named_explicit_and_packed_rejected() {
        let errs = check_src(
            r#"func sum(params numbers: [Int]): Int { return 0; }
               func main() { let _ = sum(1, 2, numbers: [3]); }"#,
        );
        assert!(
            errs.iter().any(|e| e.message.contains("explicit named `params`")),
            "{:?}",
            errs
        );
    }

    #[test]
    fn params_any_accepts_mixed_packed_types() {
        let errs = check_src(
            r#"internal func print_gen<T>(t: T);
               struct Point { x: Int, y: Int, }
               func new_print(params p: [Any]) { print_gen(p); }
               func main() { new_print("hello", 12, true, (), Point { x: 1, y: 2 }); }"#,
        );
        assert!(errs.is_empty(), "{:?}", errs);
    }

    #[test]
    fn multiple_errors_collected() {
        let errs = check_src(
            "internal func p(s: String);\nfunc f(): Int { p(\"\"); }\nfunc main() { no_such_fn(); }",
        );
        assert!(errs.len() >= 2, "{:?}", errs);
    }

    #[test]
    fn global_conflicts_with_func() {
        let errs = check_src("func x() {}\nlet x: Int = 1;\nfunc main() {}");
        assert!(errs.iter().any(|e| e.message.contains("redefinition")));
    }

    #[test]
    fn use_before_assign() {
        let errs = check_src(
            "internal func id(i: Int): Int;\nfunc main() { let a: Int; id(a); }",
        );
        assert!(errs.iter().any(|e| e.message.contains("uninitialized")));
    }

    #[test]
    fn let_is_mutable() {
        let errs = check_src("func main() { let a: Int = 1; a = 2; }");
        assert!(errs.is_empty(), "{:?}", errs);
    }

    #[test]
    fn tuple_rest_let_ok() {
        let errs = check_src("func main() { let t = (1, 2, 3); let (a, .., b) = t; let _ = a + b; }");
        assert!(errs.is_empty(), "{:?}", errs);
    }

    #[test]
    fn tuple_rest_assign_ok() {
        let errs = check_src(
            "func main() { let t = (1, 2, 3); let a: Int; let b: Int; (a, .., b) = t; let _ = a + b; }",
        );
        assert!(errs.is_empty(), "{:?}", errs);
    }

    #[test]
    fn definite_assign_if_both_branches() {
        let errs = check_src(
            "func main() { let x: Int; if true { x = 1; } else { x = 2; } let _ = x; }",
        );
        assert!(errs.is_empty(), "{:?}", errs);
    }

    #[test]
    fn definite_assign_if_only_one_branch_errors() {
        let errs = check_src("func main() { let x: Int; if true { x = 1; } let _ = x; }");
        assert!(
            errs.iter().any(|e| e.message.contains("uninitialized")),
            "{:?}",
            errs
        );
    }

    #[test]
    fn infer_local_from_assign() {
        let errs = check_src("func main() { let a: Int; a = 1; }");
        assert!(errs.is_empty(), "{:?}", errs);
    }

    #[test]
    fn duplicate_in_same_scope() {
        let errs = check_src("func main() { let a: Int; let a: Int; }");
        assert!(errs.iter().any(|e| e.message.contains("redefinition")));
    }

    #[test]
    fn shadowing_inner_scope_ok() {
        let errs = check_src("func main() { let a: Int = 1; { let a: Int = 2; } }");
        assert!(errs.is_empty(), "{:?}", errs);
    }

    #[test]
    fn example8_passes() {
        let src = include_str!("../examples/example8.vc");
        let errs = check_src(src);
        assert!(errs.is_empty(), "{:?}", errs);
    }

    #[test]
    fn while_false_body_assign_not_definite_after() {
        let errs = check_src("func main() { let x: Int; while false { x = 1; } let _ = x; }");
        assert!(
            errs.iter().any(|e| e.message.contains("uninitialized")),
            "{:?}",
            errs
        );
    }

    #[test]
    fn while_true_break_definite_after() {
        let errs = check_src("func main() { let x: Int; while true { x = 1; break; } let _ = x; }");
        assert!(errs.is_empty(), "{:?}", errs);
    }

    #[test]
    fn tuple_field_assign_rejected() {
        let errs = check_src("func main() { let t = (1, 2); t.0 = 3; }");
        assert!(
            errs.iter().any(|e| e.message.contains("tuple field")),
            "{:?}",
            errs
        );
    }

    #[test]
    fn break_outside_loop_errors() {
        let errs = check_src("func main() { break; }");
        assert!(
            errs.iter().any(|e| e.message.contains("break")),
            "{:?}",
            errs
        );
    }

    #[test]
    fn continue_outside_loop_errors() {
        let errs = check_src("func main() { continue; }");
        assert!(
            errs.iter().any(|e| e.message.contains("continue")),
            "{:?}",
            errs
        );
    }

    #[test]
    fn bitwise_int_ok() {
        let errs = check_src("func main() { let _ = 3 & 1 | 2 ^ 0; }");
        assert!(errs.is_empty(), "{:?}", errs);
    }

    #[test]
    fn compound_assign_int_ok() {
        let errs = check_src(
            "func main() { let a: Int = 1; a += 2; a -= 1; a *= 2; a /= 2; a %= 2; a &= 1; a |= 0; a ^= 0; }",
        );
        assert!(errs.is_empty(), "{:?}", errs);
    }

    #[test]
    fn while_condition_must_be_bool() {
        let errs = check_src("func main() { while 1 { } }");
        assert!(
            errs
                .iter()
                .any(|e| e.message.contains("while") && e.message.contains("Bool")),
            "{:?}",
            errs
        );
    }

    #[test]
    fn nested_loop_break_inner_only() {
        let errs = check_src(
            "func main() { let x: Int; while true { while true { x = 1; break; } break; } let _ = x; }",
        );
        assert!(errs.is_empty(), "{:?}", errs);
    }

    fn assert_sem_ok(src: &str) {
        let errs = check_src(src);
        assert!(errs.is_empty(), "{:?}", errs);
    }

    fn assert_sem_err_contains(src: &str, needle: &str) {
        let errs = check_src(src);
        assert!(
            errs.iter().any(|e| e.message.contains(needle)),
            "expected `{needle}` in {:?}",
            errs
        );
    }

    macro_rules! match_nested_pass_case {
        ($name:ident, $k:expr) => {
            #[test]
            fn $name() {
                let src = format!(
                    r#"enum Option<T> {{ None, Some(T) }}
struct Inner {{ coords: (Int, Int, Int), tags: [Int], }}
struct Wrapper {{ inner: Inner, flag: Bool, }}
func mk(i: Int): Option<Wrapper> {{
    if i > 0 {{
        return Option::Some(Wrapper {{
            inner: Inner {{ coords: (i, i + 1, i + 2), tags: [i, i + 1, i + 2, i + 3] }},
            flag: true
        }});
    }}
    return Option::None;
}}
func main() {{
    let m = mk(10);
    let _: Int = match m {{
        Option::Some(Wrapper {{ inner: Inner {{ coords: (x, .., z), tags: [head, .., tail] }}, flag: _ }}) => x + z + head + tail + {},
        Option::None => 0,
    }};
}}"#,
                    $k
                );
                assert_sem_ok(&src);
            }
        };
    }

    macro_rules! match_nested_non_exhaustive_case {
        ($name:ident, $a:expr, $b:expr) => {
            #[test]
            fn $name() {
                let src = format!(
                    r#"enum Option<T> {{ None, Some(T) }}
struct Inner {{ coords: (Int, Int, Int), tags: [Int], }}
struct Wrapper {{ inner: Inner, flag: Bool, }}
func mk(): Option<Wrapper> {{
    return Option::Some(Wrapper {{
        inner: Inner {{ coords: ({}, {}, 3), tags: [1, 2, 3, 4] }},
        flag: true
    }});
}}
func main() {{
    let _: Int = match mk() {{
        Option::Some(Wrapper {{ inner: Inner {{ coords: ({}, {}, 3), tags: [1, 2, 3, 4] }}, flag: true }}) => 1,
        Option::Some(Wrapper {{ inner: Inner {{ coords: ({}, {}, 3), tags: [1, 2, 3, 4] }}, flag: true }}) => 2,
        Option::None => 0,
    }};
}}"#,
                    $a,
                    $b,
                    $a,
                    $b,
                    $a + 1,
                    $b
                );
                assert_sem_err_contains(&src, "not exhaustive");
            }
        };
    }

    macro_rules! match_guard_non_exhaustive_case {
        ($name:ident, $threshold:expr) => {
            #[test]
            fn $name() {
                let src = format!(
                    r#"enum Option<T> {{ None, Some(T) }}
struct Inner {{ coords: (Int, Int, Int), tags: [Int], }}
struct Wrapper {{ inner: Inner, flag: Bool, }}
func mk(i: Int): Option<Wrapper> {{
    return Option::Some(Wrapper {{
        inner: Inner {{ coords: (i, i + 1, i + 2), tags: [i, i + 1, i + 2] }},
        flag: i > 0
    }});
}}
func main() {{
    let _: Int = match mk(4) {{
        Option::Some(Wrapper {{ inner: Inner {{ coords: (x, .., z), tags: [_, .., _] }}, flag: _ }}) if x > {} => x + z,
        Option::None => 0,
    }};
}}"#,
                    $threshold
                );
                assert_sem_err_contains(&src, "not exhaustive");
            }
        };
    }

    macro_rules! match_literal_type_mismatch_case {
        ($name:ident, $msg:expr) => {
            #[test]
            fn $name() {
                let src = r#"enum Option<T> { None, Some(T) }
struct Inner { coords: (Int, Int, Int), tags: [Int], }
struct Wrapper { inner: Inner, flag: Bool, }
func mk(): Option<Wrapper> {
    return Option::Some(Wrapper {
        inner: Inner { coords: (1, 2, 3), tags: [1, 2, 3] },
        flag: true
    });
}
func main() {
    let _: Int = match mk() {
        Option::Some(Wrapper { inner: Inner { coords: ("oops", .., _), tags: [_, .., _] }, flag: _ }) => 1,
        Option::None => 0,
    };
}"#;
                assert_sem_err_contains(src, $msg);
            }
        };
    }

    match_nested_pass_case!(match_nested_pass_01, 1);
    match_nested_pass_case!(match_nested_pass_02, 2);
    match_nested_pass_case!(match_nested_pass_03, 3);
    match_nested_pass_case!(match_nested_pass_04, 4);
    match_nested_pass_case!(match_nested_pass_05, 5);
    match_nested_pass_case!(match_nested_pass_06, 6);
    match_nested_pass_case!(match_nested_pass_07, 7);
    match_nested_pass_case!(match_nested_pass_08, 8);
    match_nested_pass_case!(match_nested_pass_09, 9);
    match_nested_pass_case!(match_nested_pass_10, 10);
    match_nested_pass_case!(match_nested_pass_11, 11);
    match_nested_pass_case!(match_nested_pass_12, 12);
    match_nested_pass_case!(match_nested_pass_13, 13);
    match_nested_pass_case!(match_nested_pass_14, 14);
    match_nested_pass_case!(match_nested_pass_15, 15);
    match_nested_pass_case!(match_nested_pass_16, 16);
    match_nested_pass_case!(match_nested_pass_17, 17);
    match_nested_pass_case!(match_nested_pass_18, 18);
    match_nested_pass_case!(match_nested_pass_19, 19);
    match_nested_pass_case!(match_nested_pass_20, 20);

    match_nested_non_exhaustive_case!(match_nested_non_exhaustive_01, 1, 2);
    match_nested_non_exhaustive_case!(match_nested_non_exhaustive_02, 2, 3);
    match_nested_non_exhaustive_case!(match_nested_non_exhaustive_03, 3, 4);
    match_nested_non_exhaustive_case!(match_nested_non_exhaustive_04, 4, 5);
    match_nested_non_exhaustive_case!(match_nested_non_exhaustive_05, 5, 6);
    match_nested_non_exhaustive_case!(match_nested_non_exhaustive_06, 6, 7);
    match_nested_non_exhaustive_case!(match_nested_non_exhaustive_07, 7, 8);
    match_nested_non_exhaustive_case!(match_nested_non_exhaustive_08, 8, 9);
    match_nested_non_exhaustive_case!(match_nested_non_exhaustive_09, 9, 10);
    match_nested_non_exhaustive_case!(match_nested_non_exhaustive_10, 10, 11);
    match_nested_non_exhaustive_case!(match_nested_non_exhaustive_11, 11, 12);
    match_nested_non_exhaustive_case!(match_nested_non_exhaustive_12, 12, 13);
    match_nested_non_exhaustive_case!(match_nested_non_exhaustive_13, 13, 14);
    match_nested_non_exhaustive_case!(match_nested_non_exhaustive_14, 14, 15);
    match_nested_non_exhaustive_case!(match_nested_non_exhaustive_15, 15, 16);
    match_nested_non_exhaustive_case!(match_nested_non_exhaustive_16, 16, 17);
    match_nested_non_exhaustive_case!(match_nested_non_exhaustive_17, 17, 18);
    match_nested_non_exhaustive_case!(match_nested_non_exhaustive_18, 18, 19);
    match_nested_non_exhaustive_case!(match_nested_non_exhaustive_19, 19, 20);
    match_nested_non_exhaustive_case!(match_nested_non_exhaustive_20, 20, 21);

    match_guard_non_exhaustive_case!(match_guard_non_exhaustive_01, 1);
    match_guard_non_exhaustive_case!(match_guard_non_exhaustive_02, 2);
    match_guard_non_exhaustive_case!(match_guard_non_exhaustive_03, 3);
    match_guard_non_exhaustive_case!(match_guard_non_exhaustive_04, 4);
    match_guard_non_exhaustive_case!(match_guard_non_exhaustive_05, 5);
    match_guard_non_exhaustive_case!(match_guard_non_exhaustive_06, 6);
    match_guard_non_exhaustive_case!(match_guard_non_exhaustive_07, 7);
    match_guard_non_exhaustive_case!(match_guard_non_exhaustive_08, 8);
    match_guard_non_exhaustive_case!(match_guard_non_exhaustive_09, 9);
    match_guard_non_exhaustive_case!(match_guard_non_exhaustive_10, 10);
    match_guard_non_exhaustive_case!(match_guard_non_exhaustive_11, 11);
    match_guard_non_exhaustive_case!(match_guard_non_exhaustive_12, 12);
    match_guard_non_exhaustive_case!(match_guard_non_exhaustive_13, 13);
    match_guard_non_exhaustive_case!(match_guard_non_exhaustive_14, 14);
    match_guard_non_exhaustive_case!(match_guard_non_exhaustive_15, 15);
    match_guard_non_exhaustive_case!(match_guard_non_exhaustive_16, 16);
    match_guard_non_exhaustive_case!(match_guard_non_exhaustive_17, 17);
    match_guard_non_exhaustive_case!(match_guard_non_exhaustive_18, 18);
    match_guard_non_exhaustive_case!(match_guard_non_exhaustive_19, 19);
    match_guard_non_exhaustive_case!(match_guard_non_exhaustive_20, 20);

    match_literal_type_mismatch_case!(match_literal_type_mismatch_01, "string literal pattern");
    match_literal_type_mismatch_case!(match_literal_type_mismatch_02, "string literal pattern");
    match_literal_type_mismatch_case!(match_literal_type_mismatch_03, "string literal pattern");
    match_literal_type_mismatch_case!(match_literal_type_mismatch_04, "string literal pattern");
    match_literal_type_mismatch_case!(match_literal_type_mismatch_05, "string literal pattern");
    match_literal_type_mismatch_case!(match_literal_type_mismatch_06, "string literal pattern");
    match_literal_type_mismatch_case!(match_literal_type_mismatch_07, "string literal pattern");
    match_literal_type_mismatch_case!(match_literal_type_mismatch_08, "string literal pattern");
    match_literal_type_mismatch_case!(match_literal_type_mismatch_09, "string literal pattern");
    match_literal_type_mismatch_case!(match_literal_type_mismatch_10, "string literal pattern");
    match_literal_type_mismatch_case!(match_literal_type_mismatch_11, "string literal pattern");
    match_literal_type_mismatch_case!(match_literal_type_mismatch_12, "string literal pattern");
    match_literal_type_mismatch_case!(match_literal_type_mismatch_13, "string literal pattern");
    match_literal_type_mismatch_case!(match_literal_type_mismatch_14, "string literal pattern");
    match_literal_type_mismatch_case!(match_literal_type_mismatch_15, "string literal pattern");
    match_literal_type_mismatch_case!(match_literal_type_mismatch_16, "string literal pattern");
    match_literal_type_mismatch_case!(match_literal_type_mismatch_17, "string literal pattern");
    match_literal_type_mismatch_case!(match_literal_type_mismatch_18, "string literal pattern");
    match_literal_type_mismatch_case!(match_literal_type_mismatch_19, "string literal pattern");
    match_literal_type_mismatch_case!(match_literal_type_mismatch_20, "string literal pattern");

    macro_rules! if_let_nested_pass_case {
        ($name:ident, $k:expr) => {
            #[test]
            fn $name() {
                let src = format!(
                    r#"enum Option<T> {{ None, Some(T) }}
struct Point {{ x: Int, y: Int, }}
struct Wrap {{ p: Point, tag: Int, }}
func mk(i: Int): Option<Wrap> {{
    if i > 0 {{
        return Option::Some(Wrap {{ p: Point {{ x: i, y: i + 1 }}, tag: i + 2 }});
    }}
    return Option::None;
}}
func main() {{
    let out: Int = 0;
    if let Option::Some(Wrap {{ p: Point {{ x, y: _ }}, tag }}) = mk(3) {{
        out = x + tag + {};
    }} else {{
        out = 0;
    }}
    let _ = out;
}}"#,
                    $k
                );
                assert_sem_ok(&src);
            }
        };
    }

    macro_rules! if_let_tuple_array_unsupported_case {
        ($name:ident, $src:expr) => {
            #[test]
            fn $name() {
                assert_sem_err_contains($src, "tuple/array patterns are not supported yet");
            }
        };
    }

    if_let_nested_pass_case!(if_let_nested_pass_01, 1);
    if_let_nested_pass_case!(if_let_nested_pass_02, 2);
    if_let_nested_pass_case!(if_let_nested_pass_03, 3);
    if_let_nested_pass_case!(if_let_nested_pass_04, 4);
    if_let_nested_pass_case!(if_let_nested_pass_05, 5);
    if_let_nested_pass_case!(if_let_nested_pass_06, 6);
    if_let_nested_pass_case!(if_let_nested_pass_07, 7);
    if_let_nested_pass_case!(if_let_nested_pass_08, 8);
    if_let_nested_pass_case!(if_let_nested_pass_09, 9);
    if_let_nested_pass_case!(if_let_nested_pass_10, 10);

    if_let_tuple_array_unsupported_case!(
        if_let_tuple_unsupported_01,
        r#"func main() {
               if let (a, b) = (1, 2) { let _ = a + b; } else { }
           }"#
    );
    if_let_tuple_array_unsupported_case!(
        if_let_tuple_unsupported_02,
        r#"func main() {
               if let (a, .., b) = (1, 2, 3) { let _ = a + b; } else { }
           }"#
    );
    if_let_tuple_array_unsupported_case!(
        if_let_array_unsupported_01,
        r#"func main() {
               let a: [Int] = [1, 2, 3];
               if let [x, y, z] = a { let _ = x + y + z; } else { }
           }"#
    );
    if_let_tuple_array_unsupported_case!(
        if_let_array_unsupported_02,
        r#"func main() {
               let a: [Int] = [1, 2, 3, 4];
               if let [head, .., tail] = a { let _ = head + tail; } else { }
           }"#
    );
    if_let_tuple_array_unsupported_case!(
        if_let_nested_tuple_array_unsupported_01,
        r#"func main() {
               let v = ((1, 2), [3, 4, 5]);
               if let ((a, b), arr) = v { let _ = a + b; let _ = arr; } else { }
           }"#
    );
    if_let_tuple_array_unsupported_case!(
        if_let_tuple_unsupported_03,
        r#"func main() {
               if let (_, b) = (1, 2) { let _ = b; } else { }
           }"#
    );
    if_let_tuple_array_unsupported_case!(
        if_let_array_unsupported_03,
        r#"func main() {
               let a: [Int] = [1, 2];
               if let [_, y] = a { let _ = y; } else { }
           }"#
    );
    if_let_tuple_array_unsupported_case!(
        if_let_tuple_unsupported_04,
        r#"func main() {
               if let (x, (y, z)) = (1, (2, 3)) { let _ = x + y + z; } else { }
           }"#
    );
    if_let_tuple_array_unsupported_case!(
        if_let_array_unsupported_04,
        r#"func main() {
               let a: [Int] = [1, 2, 3];
               if let [x, _, z] = a { let _ = x + z; } else { }
           }"#
    );
    if_let_tuple_array_unsupported_case!(
        if_let_nested_tuple_array_unsupported_02,
        r#"func main() {
               let a = (1, [2, 3, 4]);
               if let (x, arr) = a { let _ = x; let _ = arr; } else { }
           }"#
    );

    #[test]
    fn extension_method_and_static_calls_semantic_ok() {
        assert_sem_ok(
            r#"func String::to_string(self): String { return self; }
               func Int::max(a: Int, b: Int): Int {
                   if a > b { return a; } else { return b; }
               }
               func main() {
                   let s = "hello".to_string();
                   let m = Int::max(1, 2);
                   let _: String = s;
                   let _: Int = m;
               }"#,
        );
    }

    #[test]
    fn extension_static_cannot_be_called_as_instance() {
        assert_sem_err_contains(
            r#"func Int::from_string(s: String): Int { return 1; }
               func main() {
                   let _ = 10.from_string("x");
               }"#,
            "static and cannot be called as an instance method",
        );
    }

    #[test]
    fn generic_array_extension_fallback_prefers_concrete() {
        assert_sem_ok(
            r#"func [Int]::id(self): [Int] { return self; }
               func [type T]::id<T>(self): [T] { return self; }
               func main() {
                   let a: [Int] = [1, 2];
                   let b: [Bool] = [true];
                   let _ = a.id();
                   let _ = b.id();
               }"#,
        );
    }

    #[test]
    fn struct_extension_static_and_instance_calls_semantic_ok() {
        assert_sem_ok(
            r#"struct Void {}
               func Void::foo<T>(x: T): T { return x; }
               func Void::bar<T>(self, x: T): T { return x; }
               func main() {
                   let _ = Void::foo(123);
                   let _ = Void{}.bar(456);
               }"#,
        );
    }

    #[test]
    fn generic_struct_literals_and_unit_generic_semantic_ok() {
        assert_sem_ok(
            r#"struct Generic<T> { a: T, b: T }
               struct UnitGeneric<T>;
               func main() {
                   let g = Generic { a: 7, b: 8 };
                   let _: Generic<Int> = g;
                   let g2 = Generic<Int> { a: 9, b: 10 };
                   let _: Generic<Int> = g2;
                   let u = UnitGeneric<Int>;
                   let _: UnitGeneric<Int> = u;
               }"#,
        );
    }

    #[test]
    fn unit_generic_struct_patterns_if_let_and_match_semantic_ok() {
        assert_sem_ok(
            r#"struct UnitGeneric<T>;
               func main() {
                   let ug = UnitGeneric<Int>;
                   if let UnitGeneric<Int> = ug {
                       let _: Int = 1;
                   } else {
                       let _: Int = 2;
                   }
                   let _: Int = match ug {
                       UnitGeneric<Int> => 10,
                       _ => 20,
                   };
               }"#,
        );
    }

    #[test]
    fn unit_generic_if_let_mismatch_is_type_error() {
        assert_sem_err_contains(
            r#"struct UnitGeneric<T>;
               func main() {
                   let ug = UnitGeneric<Int>;
                   let _: Int = match ug {
                       UnitGeneric<String> => 1,
                       _ => 2,
                   };
               }"#,
            "struct pattern type mismatch",
        );
    }

    #[test]
    fn deferred_local_inference_from_first_assignment_ok() {
        assert_sem_ok(
            r#"func foo() = 0;
               func main() {
                   let a;
                   a = foo();
                   let b = a;
                   let _: Int = b;
               }"#,
        );
    }

    #[test]
    fn deferred_local_inference_rejects_conflicting_reassignment() {
        assert_sem_err_contains(
            r#"func foo() = 0;
               func main() {
                   let a;
                   a = foo();
                   a = true;
               }"#,
            "type mismatch",
        );
    }

    #[test]
    fn deferred_local_inference_still_rejects_read_before_assignment() {
        assert_sem_err_contains(
            r#"func main() {
                   let a;
                   let _ = a;
               }"#,
            "may be uninitialized",
        );
    }

    #[test]
    fn lambda_local_monomorphic_call_conflict() {
        let errs = check_src(
            r#"func main() {
                let z = (x) => x;
                let _ = z(5);
                let _ = z("hello");
            }"#,
        );
        assert!(
            errs.iter().any(|e| e.message.contains("expected `Int`")),
            "{errs:?}"
        );
    }

    #[test]
    fn deep_inference_chain_resolves_from_single_breadcrumb() {
        let errs = check_src(
            r#"func next(v: Int) = v + 1;
               func main() {
                   let f = k => k;
                   let r;
                   r = f(123);
                   let _z = next(r);
               }"#,
        );
        assert!(errs.is_empty(), "{errs:?}");
    }

    #[test]
    fn deep_inference_chain_conflict_reports_error() {
        let errs = check_src(
            r#"func next(v: Int) = v + 1;
               func main() {
                   let f = k => k;
                   let r;
                   r = f("hello");
                   let _z = next(r);
               }"#,
        );
        assert!(
            errs.iter().any(|e| e.message.contains("expected `Int`") || e.message.contains("type mismatch")),
            "{errs:?}"
        );
    }

    #[test]
    fn unresolved_chain_without_breadcrumb_reports_error() {
        let errs = check_src(
            r#"func main() {
                   let f = k => k;
                   let r;
                   r = f(1);
                   let _ = r + "x";
               }"#,
        );
        assert!(!errs.is_empty(), "{errs:?}");
    }

    #[test]
    fn breadcrumb_edge_01_linear_assign_chain_ok() {
        assert_sem_ok(r#"func main() { let a; let b; let c; a = 1; b = a; c = b; let _: Int = c; }"#);
    }

    #[test]
    fn breadcrumb_edge_02_linear_assign_chain_conflict() {
        assert_sem_err_contains(
            r#"func main() { let a; let b; a = 1; b = a; a = "x"; }"#,
            "type mismatch",
        );
    }

    #[test]
    fn breadcrumb_edge_03_call_chain_ok() {
        assert_sem_ok(r#"func id(x: Int): Int { return x; } func main() { let a; a = id(7); let _: Int = a; }"#);
    }

    #[test]
    fn breadcrumb_edge_04_call_chain_conflict() {
        assert_sem_err_contains(
            r#"func id(x: Int): Int { return x; } func main() { let a; a = id("s"); }"#,
            "expected `Int`",
        );
    }

    #[test]
    fn breadcrumb_edge_05_tuple_destructure_ok() {
        assert_sem_ok(r#"func pair(x: Int): (Int, Int) { return (x, x + 1); } func main() { let (a, b) = pair(3); let _: Int = a + b; }"#);
    }

    #[test]
    fn breadcrumb_edge_06_tuple_field_chain_ok() {
        assert_sem_ok(r#"func main() { let t; t = (10,); let n = t.0; let _: Int = n; }"#);
    }

    #[test]
    fn breadcrumb_edge_07_lambda_param_arith_breadcrumb_ok() {
        assert_sem_ok(r#"func main() { let f = x => x + 1; let _: Int = f(10); }"#);
    }

    #[test]
    fn breadcrumb_edge_08_lambda_param_call_conflict() {
        assert_sem_err_contains(r#"func main() { let f = x => x + 1; let _ = f("hello"); }"#, "type mismatch");
    }

    #[test]
    fn breadcrumb_edge_09_nested_lambda_chain_ok() {
        assert_sem_ok(r#"func main() { let f = () => (() => 7); let _: Int = f()(); }"#);
    }

    #[test]
    fn breadcrumb_edge_10_invoke_chain_three_levels_ok() {
        assert_sem_ok(r#"func main() { let f = () => (() => (() => 9)); let _: Int = f()()(); }"#);
    }

    #[test]
    fn breadcrumb_edge_11_while_condition_constrains_int_ok() {
        assert_sem_ok(r#"func main() { let x = 3; let i = 0; while i < x { i += 1; } let _: Int = i; }"#);
    }

    #[test]
    fn breadcrumb_edge_12_if_branch_merge_same_type_ok() {
        assert_sem_ok(r#"func main() { let a; if true { a = 1; } else { a = 2; } let _: Int = a; }"#);
    }

    #[test]
    fn breadcrumb_edge_13_if_branch_merge_conflict() {
        assert_sem_err_contains(r#"func main() { let a; if true { a = 1; } else { a = "x"; } }"#, "type mismatch");
    }

    #[test]
    fn breadcrumb_edge_14_late_conflict_after_long_chain() {
        assert_sem_err_contains(
            r#"func main() { let a; let b; let c; let d; a = 1; b = a; c = b; d = c; b = "x"; }"#,
            "type mismatch",
        );
    }

    #[test]
    fn breadcrumb_edge_15_conflict_between_two_calls() {
        assert_sem_err_contains(
            r#"func use_int(x: Int): Int { return x; } func main() { let r; r = use_int(1); r = use_int("s"); }"#,
            "expected `Int`",
        );
    }

    #[test]
    fn breadcrumb_edge_16_uninitialized_read_still_rejected() {
        assert_sem_err_contains(r#"func main() { let a; let _ = a; }"#, "may be uninitialized");
    }

    #[test]
    fn breadcrumb_edge_17_named_arg_breadcrumb_ok() {
        assert_sem_ok(r#"func f(x: Int, y: Int = 0): Int { return x + y; } func main() { let a; a = f(y: 2, x: 3); let _: Int = a; }"#);
    }

    #[test]
    fn breadcrumb_edge_18_default_arg_breadcrumb_ok() {
        assert_sem_ok(r#"func f(x: Int, y: Int = 5): Int { return x + y; } func main() { let r; r = f(1); let _: Int = r; }"#);
    }

    #[test]
    fn breadcrumb_edge_19_compare_operator_constrains_int_ok() {
        assert_sem_ok(r#"func main() { let a; let b; a = 1; b = 2; let _: Bool = a < b; }"#);
    }

    #[test]
    fn breadcrumb_edge_20_compare_operator_conflict() {
        assert_sem_err_contains(
            r#"func main() { let a; a = "x"; let _ = a < 1; }"#,
            "require `Int` or `Float` operands",
        );
    }

    #[test]
    fn breadcrumb_edge_21_logical_operator_constrains_bool_ok() {
        assert_sem_ok(r#"func main() { let a; let b; a = true; b = false; let _: Bool = a && b; }"#);
    }

    #[test]
    fn breadcrumb_edge_22_unary_not_constrains_bool_ok() {
        assert_sem_ok(r#"func main() { let a; a = true; let _: Bool = !a; }"#);
    }

    #[test]
    fn breadcrumb_edge_23_unary_minus_constrains_int_ok() {
        assert_sem_ok(r#"func main() { let a; a = 5; let _: Int = -a; }"#);
    }

    #[test]
    fn breadcrumb_edge_24_compound_assign_constrains_int_ok() {
        assert_sem_ok(r#"func main() { let a; a = 1; a += 2; let _: Int = a; }"#);
    }

    #[test]
    fn breadcrumb_edge_25_compound_assign_conflict() {
        assert_sem_err_contains(
            r#"func main() { let a; a = "x"; a += 1; }"#,
            "requires `Int` or `Float` operands",
        );
    }

    #[test]
    fn breadcrumb_edge_26_tuple_reassign_same_type_ok() {
        assert_sem_ok(r#"func main() { let t; t = (1, 2); t = (3, 4); let _: Int = t.0 + t.1; }"#);
    }

    #[test]
    fn breadcrumb_edge_27_tuple_reassign_conflict() {
        assert_sem_err_contains(r#"func main() { let t; t = (1, 2); t = ("x", 4); }"#, "type mismatch");
    }

    #[test]
    fn breadcrumb_edge_28_lambda_monomorphic_conflict_still_enforced() {
        assert_sem_err_contains(r#"func main() { let z = x => x; let _ = z(1); let _ = z("x"); }"#, "type mismatch");
    }

    #[test]
    fn breadcrumb_edge_29_function_value_chain_ok() {
        assert_sem_ok(r#"func inc(x: Int): Int { return x + 1; } func main() { let f = inc; let r; r = f(10); let _: Int = r; }"#);
    }

    #[test]
    fn breadcrumb_edge_30_deep_multi_func_chain_ok() {
        assert_sem_ok(
            r#"func id(x: Int): Int { return x; }
               func next(x: Int): Int { return x + 1; }
               func use(x: Int): Int { return x; }
               func main() { let a; let b; a = id(10); b = next(a); let _: Int = use(b); }"#,
        );
    }

    #[test]
    fn breadcrumb_edge_31_inference_diagnostic_avoids_raw_tvars() {
        let errs = check_src(r#"func main() { let f = x => x; let _ = f(1) + f("s"); }"#);
        assert!(!errs.is_empty(), "{errs:?}");
        assert!(
            errs.iter().all(|e| !e.message.contains("_T")),
            "{errs:?}"
        );
    }

    #[test]
    fn breadcrumb_edge_32_long_chain_stress_ok() {
        assert_sem_ok(
            r#"func main() {
                   let a1; let a2; let a3; let a4; let a5; let a6; let a7; let a8; let a9; let a10;
                   let a11; let a12; let a13; let a14; let a15; let a16; let a17; let a18; let a19; let a20;
                   a1 = 1; a2 = a1; a3 = a2; a4 = a3; a5 = a4; a6 = a5; a7 = a6; a8 = a7; a9 = a8; a10 = a9;
                   a11 = a10; a12 = a11; a13 = a12; a14 = a13; a15 = a14; a16 = a15; a17 = a16; a18 = a17; a19 = a18; a20 = a19;
                   let _: Int = a20;
               }"#,
        );
    }

    #[test]
    fn type_alias_non_generic_annotation_ok() {
        assert_sem_ok(r#"type UserId = Int; func main() { let x: UserId = 7; let _: Int = x; }"#);
    }

    #[test]
    fn type_alias_generic_return_and_ctor_ok() {
        assert_sem_ok(
            r#"enum Result<T, E> { Ok(T), Err(E) }
               type Res<T> = Result<T, String>;
               func get(): Res<_> = Res::Ok(5);
               func main() { let _ = get(); }"#,
        );
    }

    #[test]
    fn type_alias_nested_generic_arg_ok() {
        assert_sem_ok(
            r#"enum Result<T, E> { Ok(T), Err(E) }
               type Res<T> = Result<T, String>;
               func main() {
                   let v: [Res<Int>] = [Result<Int, String>::Ok(1)];
                   let _ = v;
               }"#,
        );
    }

    #[test]
    fn type_alias_wrong_arity_errors() {
        assert_sem_err_contains(
            r#"enum Result<T, E> { Ok(T), Err(E) }
               type Res<T> = Result<T, String>;
               func main() { let _: Res<Int, String> = Result<Int, String>::Ok(1); }"#,
            "too many type arguments",
        );
    }

    #[test]
    fn type_alias_cycle_errors() {
        assert_sem_err_contains(
            r#"type A = B; type B = A; func main() { let _: A = 1; }"#,
            "cyclic type alias",
        );
    }

    #[test]
    fn type_alias_ctor_non_enum_target_errors() {
        assert_sem_err_contains(
            r#"type Id = Int; func main() { let _ = Id::Ok(1); }"#,
            "unknown static method",
        );
    }

    #[test]
    fn await_outside_async_rejected() {
        let errs = check_src(
            r#"struct Task<T = ()>;
               internal async func sleep(ms: Int): Task;
               func main() {
                   await sleep(0);
               }"#,
        );
        assert!(
            errs.iter().any(|e| e.message.contains("await") && e.message.contains("async")),
            "{errs:?}"
        );
    }

    #[test]
    fn async_main_task_unit_ok() {
        let errs = check_src(
            r#"struct Task<T = ()>;
               internal async func sleep(ms: Int): Task;
               async func main(): Task {
                   await sleep(0);
               }"#,
        );
        assert!(errs.is_empty(), "{errs:?}");
    }

    #[test]
    fn async_func_returns_payload_type() {
        let errs = check_src(
            r#"struct Task<T = ()>;
               async func f(): Task<Int> {
                   return 42;
               }
               async func main(): Task {
                   let _: Int = await f();
               }"#,
        );
        assert!(errs.is_empty(), "{errs:?}");
    }
}
