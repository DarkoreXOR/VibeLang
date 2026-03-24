//! On-demand monomorphization pass (compile-time only).
//!
//! Current scope:
//! - Instantiates generic functions on-demand for call sites with explicit type args.
//! - Rewrites those call sites to concrete synthetic function names.
//! - Leaves unsupported/inferred call sites unchanged for compatibility.

use std::collections::{HashMap, HashSet, VecDeque};

use crate::ast::{AstNode, CallArg, TypeExpr};

fn format_type_expr(te: &TypeExpr) -> Option<String> {
    match te {
        TypeExpr::Named(n) => Some(n.clone()),
        TypeExpr::EnumApp { name, args } => {
            let mut out = Vec::with_capacity(args.len());
            for a in args {
                out.push(format_type_expr(a)?);
            }
            Some(format!("{name}<{}>", out.join(", ")))
        }
        TypeExpr::Unit => Some("()".to_string()),
        TypeExpr::Tuple(parts) => {
            let mut out = Vec::with_capacity(parts.len());
            for p in parts {
                out.push(format_type_expr(p)?);
            }
            Some(format!("({})", out.join(", ")))
        }
        TypeExpr::Array(inner) => Some(format!("[{}]", format_type_expr(inner)?)),
        TypeExpr::Function { params, ret } => {
            let mut p = Vec::new();
            for part in params {
                p.push(format_type_expr(&part.ty)?);
            }
            Some(format!("({}) => {}", p.join(", "), format_type_expr(ret)?))
        }
        TypeExpr::Infer | TypeExpr::TypeParam(_) => None,
    }
}

fn mangle_function_instance_name(base: &str, concrete_args: &[String]) -> String {
    if concrete_args.is_empty() {
        return base.to_string();
    }
    let suffix = concrete_args
        .iter()
        .map(|a| {
            a.chars()
                .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("__");
    format!("{base}$mono${suffix}")
}

fn rewrite_expr(
    expr: &mut AstNode,
    templates: &HashMap<String, AstNode>,
    wanted: &mut VecDeque<(String, Vec<String>)>,
    seen: &mut HashSet<String>,
) {
    match expr {
        AstNode::Call {
            callee, type_args, ..
        } => {
            if let Some(tpl) = templates.get(callee) {
                if matches!(tpl, AstNode::Function { type_params, .. } if !type_params.is_empty()) {
                    let mut concrete = Vec::new();
                    for a in type_args.iter() {
                        if let Some(s) = format_type_expr(a) {
                            concrete.push(s);
                        } else {
                            concrete.clear();
                            break;
                        }
                    }
                    if !concrete.is_empty() {
                        let mono_name = mangle_function_instance_name(callee, &concrete);
                        *callee = mono_name.clone();
                        type_args.clear();
                        if seen.insert(mono_name.clone()) {
                            wanted.push_back((mono_name, concrete));
                        }
                    }
                }
            }
        }
        AstNode::MethodCall {
            receiver,
            arguments,
            ..
        } => {
            rewrite_expr(receiver, templates, wanted, seen);
            for a in arguments {
                if let CallArg::Positional(v) = a {
                    rewrite_expr(v, templates, wanted, seen);
                } else if let CallArg::Named { value, .. } = a {
                    rewrite_expr(value, templates, wanted, seen);
                }
            }
        }
        AstNode::TypeMethodCall { arguments, .. } => {
            for a in arguments {
                if let CallArg::Positional(v) = a {
                    rewrite_expr(v, templates, wanted, seen);
                } else if let CallArg::Named { value, .. } = a {
                    rewrite_expr(value, templates, wanted, seen);
                }
            }
        }
        AstNode::StructLiteral { fields, update, .. } => {
            for (_, v) in fields {
                rewrite_expr(v, templates, wanted, seen);
            }
            if let Some(u) = update {
                rewrite_expr(u, templates, wanted, seen);
            }
        }
        AstNode::EnumVariantCtor { payloads, .. } => {
            for p in payloads {
                rewrite_expr(p, templates, wanted, seen);
            }
        }
        AstNode::ArrayLiteral { elements, .. } | AstNode::TupleLiteral { elements, .. } => {
            for e in elements {
                rewrite_expr(e, templates, wanted, seen);
            }
        }
        AstNode::UnaryOp { operand, .. } => rewrite_expr(operand, templates, wanted, seen),
        AstNode::BinaryOp { left, right, .. } => {
            rewrite_expr(left, templates, wanted, seen);
            rewrite_expr(right, templates, wanted, seen);
        }
        AstNode::FieldAccess { base, .. } => rewrite_expr(base, templates, wanted, seen),
        AstNode::ArrayIndex { base, index, .. } => {
            rewrite_expr(base, templates, wanted, seen);
            rewrite_expr(index, templates, wanted, seen);
        }
        AstNode::TupleField { base, .. } => rewrite_expr(base, templates, wanted, seen),
        AstNode::AssignExpr { lhs, rhs, .. } => {
            rewrite_expr(lhs, templates, wanted, seen);
            rewrite_expr(rhs, templates, wanted, seen);
        }
        AstNode::CompoundAssign { lhs, rhs, .. } => {
            rewrite_expr(lhs, templates, wanted, seen);
            rewrite_expr(rhs, templates, wanted, seen);
        }
        AstNode::Assign { value, .. } => rewrite_expr(value, templates, wanted, seen),
        AstNode::Let {
            initializer: Some(init),
            ..
        } => rewrite_expr(init, templates, wanted, seen),
        AstNode::Return { value: Some(v), .. } => rewrite_expr(v, templates, wanted, seen),
        AstNode::If {
            condition,
            then_body,
            else_body,
            ..
        } => {
            rewrite_expr(condition, templates, wanted, seen);
            for s in then_body {
                rewrite_expr(s, templates, wanted, seen);
            }
            if let Some(else_body) = else_body {
                for s in else_body {
                    rewrite_expr(s, templates, wanted, seen);
                }
            }
        }
        AstNode::IfLet {
            value,
            then_body,
            else_body,
            ..
        } => {
            rewrite_expr(value, templates, wanted, seen);
            for s in then_body {
                rewrite_expr(s, templates, wanted, seen);
            }
            if let Some(else_body) = else_body {
                for s in else_body {
                    rewrite_expr(s, templates, wanted, seen);
                }
            }
        }
        AstNode::While {
            condition, body, ..
        } => {
            rewrite_expr(condition, templates, wanted, seen);
            for s in body {
                rewrite_expr(s, templates, wanted, seen);
            }
        }
        AstNode::Match {
            scrutinee, arms, ..
        } => {
            rewrite_expr(scrutinee, templates, wanted, seen);
            for arm in arms {
                if let Some(g) = arm.guard.as_mut() {
                    rewrite_expr(g, templates, wanted, seen);
                }
                rewrite_expr(arm.body.as_mut(), templates, wanted, seen);
            }
        }
        AstNode::Block { body, .. } | AstNode::Program(body) => {
            for s in body {
                rewrite_expr(s, templates, wanted, seen);
            }
        }
        _ => {}
    }
}

fn instantiate_function_template(
    template: &AstNode,
    concrete_name: String,
) -> Option<AstNode> {
    let AstNode::Function {
        extension_receiver,
        params,
        return_type,
        body,
        name_span,
        closing_span,
        is_exported,
        is_async,
        ..
    } = template
    else {
        return None;
    };
    Some(AstNode::Function {
        name: concrete_name,
        extension_receiver: extension_receiver.clone(),
        type_params: Vec::new(),
        params: params.clone(),
        return_type: return_type.clone(),
        body: body.clone(),
        name_span: *name_span,
        closing_span: *closing_span,
        is_exported: *is_exported,
        is_async: *is_async,
    })
}

fn collect_concrete_type_uses_from_type_expr(te: &TypeExpr, out: &mut HashSet<String>) {
    match te {
        TypeExpr::EnumApp { name, args } => {
            let mut parts = Vec::with_capacity(args.len());
            let mut all_concrete = true;
            for a in args {
                if let Some(s) = format_type_expr(a) {
                    parts.push(s);
                } else {
                    all_concrete = false;
                }
                collect_concrete_type_uses_from_type_expr(a, out);
            }
            if all_concrete {
                out.insert(format!("{name}<{}>", parts.join(", ")));
            }
        }
        TypeExpr::Tuple(parts) => {
            for p in parts {
                collect_concrete_type_uses_from_type_expr(p, out);
            }
        }
        TypeExpr::Array(inner) => collect_concrete_type_uses_from_type_expr(inner, out),
        _ => {}
    }
}

fn collect_concrete_type_uses_from_node(node: &AstNode, out: &mut HashSet<String>) {
    match node {
        AstNode::StructLiteral {
            name,
            type_args,
            fields,
            update,
            ..
        } => {
            if !type_args.is_empty() {
                let mut parts = Vec::new();
                let mut ok = true;
                for a in type_args {
                    if let Some(s) = format_type_expr(a) {
                        parts.push(s);
                    } else {
                        ok = false;
                    }
                }
                if ok {
                    out.insert(format!("{name}<{}>", parts.join(", ")));
                }
            }
            for (_, v) in fields {
                collect_concrete_type_uses_from_node(v, out);
            }
            if let Some(u) = update {
                collect_concrete_type_uses_from_node(u, out);
            }
        }
        AstNode::TypeValue { type_name, .. } => {
            if type_name.contains('<') {
                out.insert(type_name.clone());
            }
        }
        AstNode::Call {
            type_args,
            arguments,
            ..
        } => {
            for a in type_args {
                collect_concrete_type_uses_from_type_expr(a, out);
            }
            for arg in arguments {
                match arg {
                    CallArg::Positional(v) => collect_concrete_type_uses_from_node(v, out),
                    CallArg::Named { value, .. } => collect_concrete_type_uses_from_node(value, out),
                }
            }
        }
        AstNode::EnumVariantCtor {
            enum_name,
            type_args,
            payloads,
            ..
        } => {
            if !type_args.is_empty() {
                let mut parts = Vec::new();
                let mut ok = true;
                for a in type_args {
                    if let Some(s) = format_type_expr(a) {
                        parts.push(s);
                    } else {
                        ok = false;
                    }
                }
                if ok {
                    out.insert(format!("{enum_name}<{}>", parts.join(", ")));
                }
            }
            for p in payloads {
                collect_concrete_type_uses_from_node(p, out);
            }
        }
        AstNode::MethodCall {
            receiver,
            arguments,
            ..
        } => {
            collect_concrete_type_uses_from_node(receiver, out);
            for arg in arguments {
                match arg {
                    CallArg::Positional(v) => collect_concrete_type_uses_from_node(v, out),
                    CallArg::Named { value, .. } => collect_concrete_type_uses_from_node(value, out),
                }
            }
        }
        AstNode::TypeMethodCall { arguments, .. } => {
            for arg in arguments {
                match arg {
                    CallArg::Positional(v) => collect_concrete_type_uses_from_node(v, out),
                    CallArg::Named { value, .. } => collect_concrete_type_uses_from_node(value, out),
                }
            }
        }
        AstNode::Let { initializer, .. } => {
            if let Some(i) = initializer {
                collect_concrete_type_uses_from_node(i, out);
            }
        }
        AstNode::Assign { value, .. } => collect_concrete_type_uses_from_node(value, out),
        AstNode::AssignExpr { lhs, rhs, .. } | AstNode::CompoundAssign { lhs, rhs, .. } => {
            collect_concrete_type_uses_from_node(lhs, out);
            collect_concrete_type_uses_from_node(rhs, out);
        }
        AstNode::If {
            condition,
            then_body,
            else_body,
            ..
        } => {
            collect_concrete_type_uses_from_node(condition, out);
            for s in then_body {
                collect_concrete_type_uses_from_node(s, out);
            }
            if let Some(es) = else_body {
                for s in es {
                    collect_concrete_type_uses_from_node(s, out);
                }
            }
        }
        AstNode::IfLet {
            value,
            then_body,
            else_body,
            ..
        } => {
            collect_concrete_type_uses_from_node(value, out);
            for s in then_body {
                collect_concrete_type_uses_from_node(s, out);
            }
            if let Some(es) = else_body {
                for s in es {
                    collect_concrete_type_uses_from_node(s, out);
                }
            }
        }
        AstNode::While {
            condition, body, ..
        } => {
            collect_concrete_type_uses_from_node(condition, out);
            for s in body {
                collect_concrete_type_uses_from_node(s, out);
            }
        }
        AstNode::Match {
            scrutinee, arms, ..
        } => {
            collect_concrete_type_uses_from_node(scrutinee, out);
            for arm in arms {
                if let Some(g) = arm.guard.as_ref() {
                    collect_concrete_type_uses_from_node(g, out);
                }
                collect_concrete_type_uses_from_node(arm.body.as_ref(), out);
            }
        }
        AstNode::Block { body, .. } | AstNode::Program(body) => {
            for s in body {
                collect_concrete_type_uses_from_node(s, out);
            }
        }
        AstNode::ArrayLiteral { elements, .. } | AstNode::TupleLiteral { elements, .. } => {
            for e in elements {
                collect_concrete_type_uses_from_node(e, out);
            }
        }
        AstNode::UnaryOp { operand, .. } => collect_concrete_type_uses_from_node(operand, out),
        AstNode::BinaryOp { left, right, .. } => {
            collect_concrete_type_uses_from_node(left, out);
            collect_concrete_type_uses_from_node(right, out);
        }
        AstNode::FieldAccess { base, .. }
        | AstNode::TupleField { base, .. }
        | AstNode::Return { value: Some(base), .. } => collect_concrete_type_uses_from_node(base, out),
        AstNode::ArrayIndex { base, index, .. } => {
            collect_concrete_type_uses_from_node(base, out);
            collect_concrete_type_uses_from_node(index, out);
        }
        _ => {}
    }
}

pub fn monomorphize_program(ast: &AstNode) -> AstNode {
    let AstNode::Program(items) = ast else {
        return ast.clone();
    };

    let mut out_items = items.clone();
    let mut templates: HashMap<String, AstNode> = HashMap::new();
    for item in items {
        if let AstNode::Function {
            name, type_params, ..
        } = item
        {
            if !type_params.is_empty() {
                templates.insert(name.clone(), item.clone());
            }
        }
    }

    let mut wanted: VecDeque<(String, Vec<String>)> = VecDeque::new();
    let mut seen: HashSet<String> = HashSet::new();
    for item in &mut out_items {
        rewrite_expr(item, &templates, &mut wanted, &mut seen);
    }

    while let Some((mono_name, concrete_args)) = wanted.pop_front() {
        let Some((base_name, _)) = mono_name.split_once("$mono$") else {
            continue;
        };
        let Some(template) = templates.get(base_name) else {
            continue;
        };
        if let Some(mut inst) = instantiate_function_template(template, mono_name.clone()) {
            rewrite_expr(&mut inst, &templates, &mut wanted, &mut seen);
            out_items.push(inst);
        }
        let _ = concrete_args;
    }

    // Materialize concrete generic ADT declarations that are used by this program.
    let mut used_concrete_types = HashSet::new();
    for item in &out_items {
        collect_concrete_type_uses_from_node(item, &mut used_concrete_types);
    }
    let mut generic_struct_templates: HashMap<String, AstNode> = HashMap::new();
    let mut generic_enum_templates: HashMap<String, AstNode> = HashMap::new();
    for item in items {
        match item {
            AstNode::StructDef {
                name,
                type_params,
                is_internal,
                ..
            } if !type_params.is_empty() && !is_internal => {
                generic_struct_templates.insert(name.clone(), item.clone());
            }
            AstNode::EnumDef {
                name, type_params, ..
            } if !type_params.is_empty() => {
                generic_enum_templates.insert(name.clone(), item.clone());
            }
            _ => {}
        }
    }
    let existing_type_defs: HashSet<String> = out_items
        .iter()
        .filter_map(|n| match n {
            AstNode::StructDef { name, .. } | AstNode::EnumDef { name, .. } => Some(name.clone()),
            _ => None,
        })
        .collect();
    for concrete in used_concrete_types {
        if existing_type_defs.contains(&concrete) {
            continue;
        }
        let Some((base, _)) = concrete.split_once('<') else {
            continue;
        };
        if let Some(AstNode::StructDef {
            fields,
            is_unit,
            is_internal,
            name_span,
            span,
            is_exported,
            ..
        }) = generic_struct_templates.get(base)
        {
            out_items.push(AstNode::StructDef {
                name: concrete.clone(),
                type_params: Vec::new(),
                fields: fields.clone(),
                is_unit: *is_unit,
                is_internal: *is_internal,
                name_span: *name_span,
                span: *span,
                is_exported: *is_exported,
            });
            continue;
        }
        if let Some(AstNode::EnumDef {
            variants,
            name_span,
            span,
            is_exported,
            ..
        }) = generic_enum_templates.get(base)
        {
            out_items.push(AstNode::EnumDef {
                name: concrete,
                type_params: Vec::new(),
                variants: variants.clone(),
                name_span: *name_span,
                span: *span,
                is_exported: *is_exported,
            });
        }
    }

    AstNode::Program(out_items)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::Lexer;
    use crate::parser::Parser;

    fn parse(src: &str) -> AstNode {
        let mut lexer = Lexer::new(src);
        let tokens = lexer.tokenize().expect("lex");
        let mut parser = Parser::new(tokens);
        parser.parse().expect("parse")
    }

    #[test]
    fn keeps_program_shape_for_non_explicit_generic_calls() {
        let ast = parse(
            r#"func id<T>(x: T): T { return x; }
               func main() { let _ = id(1); }"#,
        );
        let mono = monomorphize_program(&ast);
        let AstNode::Program(items) = mono else {
            panic!("program");
        };
        let names: Vec<String> = items
            .iter()
            .filter_map(|n| match n {
                AstNode::Function { name, .. } => Some(name.clone()),
                _ => None,
            })
            .collect();
        assert!(names.contains(&"id".to_string()));
        assert!(names.contains(&"main".to_string()));
    }
}

