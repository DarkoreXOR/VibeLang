use std::collections::HashMap;

use crate::error::{SemanticError, Span};
use crate::semantic::Ty;

#[derive(Default, Debug, Clone)]
pub(crate) struct InferCtx {
    next_id: u32,
    substs: HashMap<u32, Ty>,
}

impl InferCtx {
    pub(crate) fn fresh_var(&mut self) -> Ty {
        let id = self.next_id;
        self.next_id += 1;
        Ty::InferVar(id)
    }
}

#[allow(dead_code)]
fn ty_name(ty: &Ty) -> String {
    match ty {
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
        Ty::Function { params, ret, .. } => {
            let inner: Vec<String> = params.iter().map(ty_name).collect();
            format!("({}) => {}", inner.join(", "), ty_name(ret))
        }
        Ty::InferVar(_) => "an inferred type".to_string(),
        Ty::Task(inner) => format!("Task<{}>", ty_name(inner)),
    }
}

fn occurs(var: u32, ty: &Ty, ctx: &InferCtx) -> bool {
    match resolve_ty(ty, ctx) {
        Ty::InferVar(id) => id == var,
        Ty::Array(inner) => occurs(var, inner.as_ref(), ctx),
        Ty::Tuple(parts) => parts.iter().any(|p| occurs(var, p, ctx)),
        Ty::Enum { args, .. } => args.iter().any(|a| occurs(var, a, ctx)),
        Ty::Function { params, ret, .. } => {
            params.iter().any(|p| occurs(var, p, ctx)) || occurs(var, ret.as_ref(), ctx)
        }
        Ty::Task(inner) => occurs(var, inner.as_ref(), ctx),
        _ => false,
    }
}

pub(crate) fn resolve_ty(ty: &Ty, ctx: &InferCtx) -> Ty {
    match ty {
        Ty::InferVar(id) => {
            if let Some(t) = ctx.substs.get(id) {
                resolve_ty(t, ctx)
            } else {
                ty.clone()
            }
        }
        Ty::Array(inner) => Ty::Array(Box::new(resolve_ty(inner.as_ref(), ctx))),
        Ty::Tuple(parts) => Ty::Tuple(parts.iter().map(|p| resolve_ty(p, ctx)).collect()),
        Ty::Enum { name, args } => Ty::Enum {
            name: name.clone(),
            args: args.iter().map(|a| resolve_ty(a, ctx)).collect(),
        },
        Ty::Function {
            params,
            param_names,
            param_has_default,
            ret,
        } => Ty::Function {
            params: params.iter().map(|p| resolve_ty(p, ctx)).collect(),
            param_names: param_names.clone(),
            param_has_default: param_has_default.clone(),
            ret: Box::new(resolve_ty(ret.as_ref(), ctx)),
        },
        Ty::Task(inner) => Ty::Task(Box::new(resolve_ty(inner.as_ref(), ctx))),
        _ => ty.clone(),
    }
}

pub(crate) fn unify_types(
    lhs: &Ty,
    rhs: &Ty,
    ctx: &mut InferCtx,
    errors: &mut Vec<SemanticError>,
    span: Span,
) -> bool {
    let l = resolve_ty(lhs, ctx);
    let r = resolve_ty(rhs, ctx);
    match (&l, &r) {
        (Ty::InferVar(a), Ty::InferVar(b)) if a == b => true,
        (Ty::InferVar(a), t) => {
            if occurs(*a, t, ctx) {
                errors.push(SemanticError::new(
                    format!("recursive inferred type `{}`", ty_name(&l)),
                    span,
                ));
                return false;
            }
            ctx.substs.insert(*a, t.clone());
            true
        }
        (t, Ty::InferVar(b)) => {
            if occurs(*b, t, ctx) {
                errors.push(SemanticError::new(
                    format!("recursive inferred type `{}`", ty_name(&r)),
                    span,
                ));
                return false;
            }
            ctx.substs.insert(*b, t.clone());
            true
        }
        (Ty::Array(a), Ty::Array(b)) => unify_types(a, b, ctx, errors, span),
        (Ty::Tuple(a), Ty::Tuple(b)) => {
            if a.len() != b.len() {
                errors.push(SemanticError::new(
                    format!(
                        "type mismatch: expected `{}`, found `{}`",
                        ty_name(&l),
                        ty_name(&r)
                    ),
                    span,
                ));
                return false;
            }
            a.iter()
                .zip(b.iter())
                .all(|(x, y)| unify_types(x, y, ctx, errors, span))
        }
        (
            Ty::Enum {
                name: n1,
                args: a1,
            },
            Ty::Enum {
                name: n2,
                args: a2,
            },
        ) => {
            if n1 != n2 || a1.len() != a2.len() {
                errors.push(SemanticError::new(
                    format!(
                        "type mismatch: expected `{}`, found `{}`",
                        ty_name(&l),
                        ty_name(&r)
                    ),
                    span,
                ));
                return false;
            }
            a1.iter()
                .zip(a2.iter())
                .all(|(x, y)| unify_types(x, y, ctx, errors, span))
        }
        (
            Ty::Function {
                params: p1,
                ret: r1,
                ..
            },
            Ty::Function {
                params: p2,
                ret: r2,
                ..
            },
        ) => {
            if p1.len() != p2.len() {
                errors.push(SemanticError::new(
                    format!(
                        "type mismatch: expected `{}`, found `{}`",
                        ty_name(&l),
                        ty_name(&r)
                    ),
                    span,
                ));
                return false;
            }
            p1.iter()
                .zip(p2.iter())
                .all(|(x, y)| unify_types(x, y, ctx, errors, span))
                && unify_types(r1, r2, ctx, errors, span)
        }
        (Ty::Task(a), Ty::Task(b)) => unify_types(a, b, ctx, errors, span),
        _ if l == r => true,
        _ => {
            errors.push(SemanticError::new(
                format!(
                    "type mismatch: expected `{}`, found `{}`",
                    ty_name(&l),
                    ty_name(&r)
                ),
                span,
            ));
            false
        }
    }
}

#[allow(dead_code)]
#[allow(dead_code)]
pub(crate) fn instantiate_ty(template: &Ty, substs: &HashMap<String, Ty>) -> Ty {
    match template {
        Ty::TypeParam(name) => substs
            .get(name)
            .cloned()
            .unwrap_or_else(|| Ty::TypeParam(name.clone())),
        Ty::Array(inner) => Ty::Array(Box::new(instantiate_ty(inner, substs))),
        Ty::Tuple(parts) => Ty::Tuple(parts.iter().map(|p| instantiate_ty(p, substs)).collect()),
        Ty::Enum { name, args } => Ty::Enum {
            name: name.clone(),
            args: args.iter().map(|a| instantiate_ty(a, substs)).collect(),
        },
        Ty::Function {
            params,
            param_names,
            param_has_default,
            ret,
        } => Ty::Function {
            params: params.iter().map(|p| instantiate_ty(p, substs)).collect(),
            param_names: param_names.clone(),
            param_has_default: param_has_default.clone(),
            ret: Box::new(instantiate_ty(ret, substs)),
        },
        Ty::InferVar(id) => Ty::InferVar(*id),
        Ty::Task(inner) => Ty::Task(Box::new(instantiate_ty(inner, substs))),
        Ty::Struct(name) => {
            // `Ty::Struct` stores the full nominal instance name as a string, e.g. `Dict<K, V>`.
            // For calls that infer type parameters from receiver types, we must also substitute
            // these generic args inside the instance-name string.
            let Some((base, rest)) = name.split_once('<') else {
                return Ty::Struct(name.clone());
            };
            let inner = match rest.strip_suffix('>') {
                Some(v) => v,
                None => return Ty::Struct(name.clone()),
            };

            // Split on top-level commas (do not split inside nested `<...>`).
            let mut args = Vec::<String>::new();
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
                            args.push(part.to_string());
                        }
                        buf.clear();
                    }
                    _ => buf.push(ch),
                }
            }
            let part = buf.trim();
            if !part.is_empty() {
                args.push(part.to_string());
            }

            // Substitute only direct type-parameter tokens (e.g. `K` -> `String`).
            let mut subst_args = Vec::<String>::new();
            for a in args {
                if let Some(ty) = substs.get(a.trim()) {
                    subst_args.push(ty_name(ty));
                } else {
                    subst_args.push(a.trim().to_string());
                }
            }

            Ty::Struct(format!("{}<{}>", base.trim(), subst_args.join(", ")))
        }
        other => other.clone(),
    }
}

#[allow(dead_code)]
#[allow(dead_code)]
pub(crate) fn infer_generic_from_template(
    template: &Ty,
    got: &Ty,
    substs: &mut HashMap<String, Ty>,
    errors: &mut Vec<SemanticError>,
    span: Span,
) -> bool {
    match template {
        Ty::Any => true,
        Ty::TypeParam(name) => match substs.get(name) {
            None => {
                substs.insert(name.clone(), got.clone());
                true
            }
            Some(prev) => {
                if prev == got {
                    true
                } else {
                    errors.push(SemanticError::new(
                        format!(
                            "inconsistent type inference for generic parameter `{}` (expected `{}`, found `{}`)",
                            name,
                            ty_name(prev),
                            ty_name(got)
                        ),
                        span,
                    ));
                    false
                }
            }
        },
        Ty::Array(exp_inner) => match got {
            Ty::Array(got_inner) => infer_generic_from_template(
                exp_inner.as_ref(),
                got_inner.as_ref(),
                substs,
                errors,
                span,
            ),
            other => {
                errors.push(SemanticError::new(
                    format!(
                        "argument type mismatch during generic inference (found `{}`, expected `{}`)",
                        ty_name(other),
                        ty_name(template)
                    ),
                    span,
                ));
                false
            }
        },
        Ty::Tuple(exp_parts) => match got {
            Ty::Tuple(got_parts) => {
                if exp_parts.len() != got_parts.len() {
                    errors.push(SemanticError::new(
                        format!(
                            "tuple length mismatch during generic inference (expected {}, found {})",
                            exp_parts.len(),
                            got_parts.len()
                        ),
                        span,
                    ));
                    return false;
                }
                for (e, g) in exp_parts.iter().zip(got_parts.iter()) {
                    if !infer_generic_from_template(e, g, substs, errors, span) {
                        return false;
                    }
                }
                true
            }
            other => {
                errors.push(SemanticError::new(
                    format!(
                        "argument type mismatch during generic inference (found `{}`, expected `{}`)",
                        ty_name(other),
                        ty_name(template)
                    ),
                    span,
                ));
                false
            }
        },
        Ty::Enum {
            name: exp_name,
            args: exp_args,
        } => match got {
            Ty::Enum {
                name: got_name,
                args: got_args,
            } => {
                if exp_name != got_name {
                    errors.push(SemanticError::new(
                        format!(
                            "argument type mismatch during generic inference: expected enum `{}` but found enum `{}`",
                            exp_name, got_name
                        ),
                        span,
                    ));
                    return false;
                }
                if exp_args.len() != got_args.len() {
                    errors.push(SemanticError::new(
                        format!(
                            "enum generic argument count mismatch: expected {}, found {}",
                            exp_args.len(),
                            got_args.len()
                        ),
                        span,
                    ));
                    return false;
                }
                for (e, g) in exp_args.iter().zip(got_args.iter()) {
                    if !infer_generic_from_template(e, g, substs, errors, span) {
                        return false;
                    }
                }
                true
            }
            other => {
                errors.push(SemanticError::new(
                    format!(
                        "argument type mismatch during generic inference (found `{}`, expected `{}`)",
                        ty_name(other),
                        ty_name(template)
                    ),
                    span,
                ));
                false
            }
        },
        Ty::Function {
            params: exp_params,
            ret: exp_ret,
            ..
        } => match got {
            Ty::Function {
                params: got_params,
                ret: got_ret,
                ..
            } => {
                if exp_params.len() != got_params.len() {
                    errors.push(SemanticError::new(
                        "function arity mismatch during generic inference",
                        span,
                    ));
                    return false;
                }
                for (e, g) in exp_params.iter().zip(got_params.iter()) {
                    if !infer_generic_from_template(e, g, substs, errors, span) {
                        return false;
                    }
                }
                infer_generic_from_template(exp_ret, got_ret, substs, errors, span)
            }
            other => {
                errors.push(SemanticError::new(
                    format!(
                        "argument type mismatch during generic inference (found `{}`, expected `{}`)",
                        ty_name(other),
                        ty_name(template)
                    ),
                    span,
                ));
                false
            }
        },
        Ty::Task(exp_inner) => match got {
            Ty::Task(got_inner) => infer_generic_from_template(
                exp_inner.as_ref(),
                got_inner.as_ref(),
                substs,
                errors,
                span,
            ),
            other => {
                errors.push(SemanticError::new(
                    format!(
                        "argument type mismatch during generic inference (found `{}`, expected `{}`)",
                        ty_name(other),
                        ty_name(template)
                    ),
                    span,
                ));
                false
            }
        },
        Ty::Struct(exp_inst) => match got {
            Ty::Struct(got_inst) => {
                // Best-effort inference for struct instance templates, based on inst-name
                // strings such as `Dict<K, V>` vs `Dict<String, Float>`.
                let exp_base = exp_inst
                    .split_once('<')
                    .map(|(b, _)| b)
                    .unwrap_or(exp_inst.as_str());
                let got_base = got_inst
                    .split_once('<')
                    .map(|(b, _)| b)
                    .unwrap_or(got_inst.as_str());
                if exp_base != got_base {
                    errors.push(SemanticError::new(
                        format!(
                            "argument type mismatch during generic inference (found `{}`, expected `{}`)",
                            ty_name(got),
                            ty_name(template)
                        ),
                        span,
                    ));
                    return false;
                }

                let split_inst_args = |inst: &str| -> Option<Vec<String>> {
                    let Some((_, rest)) = inst.split_once('<') else {
                        return Some(vec![]);
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

                let exp_args = match split_inst_args(exp_inst) {
                    Some(v) => v,
                    None => {
                        errors.push(SemanticError::new(
                            format!(
                                "argument type mismatch during generic inference (found `{}`, expected `{}`)",
                                ty_name(got),
                                ty_name(template)
                            ),
                            span,
                        ));
                        return false;
                    }
                };
                let got_args = match split_inst_args(got_inst) {
                    Some(v) => v,
                    None => {
                        errors.push(SemanticError::new(
                            format!(
                                "argument type mismatch during generic inference (found `{}`, expected `{}`)",
                                ty_name(got),
                                ty_name(template)
                            ),
                            span,
                        ));
                        return false;
                    }
                };

                if exp_args.len() != got_args.len() {
                    errors.push(SemanticError::new(
                        format!(
                            "argument type mismatch during generic inference (found `{}`, expected `{}`)",
                            ty_name(got),
                            ty_name(template)
                        ),
                        span,
                    ));
                    return false;
                }

                fn parse_ty_arg(arg: &str) -> Ty {
                    let n = arg.trim();
                    match n {
                        "Int" => Ty::Int,
                        "Float" => Ty::Float,
                        "String" => Ty::String,
                        "Bool" => Ty::Bool,
                        "Any" => Ty::Any,
                        "()" => Ty::Unit,
                        _ => {
                            if let Some((base, rest)) = n.split_once('<') {
                                if let Some(inner) = rest.strip_suffix('>') {
                                    // Split args at top-level commas.
                                    let mut args = Vec::new();
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
                                                    args.push(part.to_string());
                                                }
                                                buf.clear();
                                            }
                                            _ => buf.push(ch),
                                        }
                                    }
                                    let part = buf.trim();
                                    if !part.is_empty() {
                                        args.push(part.to_string());
                                    }
                                    if base == "Option" && args.len() == 1 {
                                        return Ty::Enum {
                                            name: "Option".to_string(),
                                            args: vec![parse_ty_arg(&args[0])],
                                        };
                                    }
                                    if base == "Result" && args.len() == 2 {
                                        return Ty::Enum {
                                            name: "Result".to_string(),
                                            args: vec![
                                                parse_ty_arg(&args[0]),
                                                parse_ty_arg(&args[1]),
                                            ],
                                        };
                                    }
                                }
                            }
                            Ty::Struct(n.to_string())
                        }
                    }
                }

                for (exp_arg, got_arg) in exp_args.iter().zip(got_args.iter()) {
                    let got_ty = parse_ty_arg(got_arg);
                    if let Some(prev) = substs.get(exp_arg.as_str()) {
                        if prev != &got_ty {
                            errors.push(SemanticError::new(
                                format!(
                                    "inconsistent type inference for generic parameter `{}` (expected `{}`, found `{}`)",
                                    exp_arg,
                                    ty_name(prev),
                                    ty_name(&got_ty)
                                ),
                                span,
                            ));
                            return false;
                        }
                    } else {
                        substs.insert(exp_arg.clone(), got_ty);
                    }
                }

                true
            }
            other => {
                errors.push(SemanticError::new(
                    format!(
                        "argument type mismatch during generic inference (found `{}`, expected `{}`)",
                        ty_name(other),
                        ty_name(template)
                    ),
                    span,
                ));
                false
            }
        },
        other => {
            if other == got {
                true
            } else {
                errors.push(SemanticError::new(
                    format!(
                        "argument type mismatch during generic inference (found `{}`, expected `{}`)",
                        ty_name(got),
                        ty_name(template)
                    ),
                    span,
                ));
                false
            }
        }
    }
}

