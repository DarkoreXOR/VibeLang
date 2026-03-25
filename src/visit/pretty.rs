//! Debug-style pretty printer; useful for tests and quick CLI inspection.

use crate::ast::{AstNode, BinaryOp, CallArg, CompoundOp, TypeExpr, UnaryOp};
use crate::visit::Visit;

pub fn pretty_print_ast(node: &AstNode) -> String {
    let mut p = PrettyPrinter { out: String::new(), indent: 0 };
    p.visit_ast_node(node);
    p.out
}

struct PrettyPrinter {
    out: String,
    indent: usize,
}

impl PrettyPrinter {
    fn line(&mut self, s: &str) {
        for _ in 0..self.indent {
            self.out.push_str("  ");
        }
        self.out.push_str(s);
        self.out.push('\n');
    }

    fn fmt_binary_op(op: BinaryOp) -> &'static str {
        match op {
            BinaryOp::Add => "Add",
            BinaryOp::Sub => "Sub",
            BinaryOp::Mul => "Mul",
            BinaryOp::Div => "Div",
            BinaryOp::Mod => "Mod",
            BinaryOp::Eq => "Eq",
            BinaryOp::Ne => "Ne",
            BinaryOp::Lt => "Lt",
            BinaryOp::Gt => "Gt",
            BinaryOp::Le => "Le",
            BinaryOp::Ge => "Ge",
            BinaryOp::BitAnd => "BitAnd",
            BinaryOp::BitXor => "BitXor",
            BinaryOp::BitOr => "BitOr",
            BinaryOp::ShiftLeft => "ShiftLeft",
            BinaryOp::ShiftRight => "ShiftRight",
            BinaryOp::And => "And",
            BinaryOp::Or => "Or",
        }
    }

    fn fmt_compound_op(op: CompoundOp) -> &'static str {
        match op {
            CompoundOp::Add => "Add",
            CompoundOp::Sub => "Sub",
            CompoundOp::Mul => "Mul",
            CompoundOp::Div => "Div",
            CompoundOp::Mod => "Mod",
            CompoundOp::BitAnd => "BitAnd",
            CompoundOp::BitXor => "BitXor",
            CompoundOp::BitOr => "BitOr",
            CompoundOp::ShiftLeft => "ShiftLeft",
            CompoundOp::ShiftRight => "ShiftRight",
        }
    }

    fn fmt_unary_op(op: UnaryOp) -> &'static str {
        match op {
            UnaryOp::Plus => "Plus",
            UnaryOp::Minus => "Minus",
            UnaryOp::BitNot => "BitNot",
            UnaryOp::Not => "Not",
        }
    }

    fn fmt_type_expr(te: &TypeExpr) -> String {
        match te {
            TypeExpr::Named(s) => format!("{s:?}"),
            TypeExpr::EnumApp { name, args } => {
                let inner: Vec<String> = args.iter().map(|p| Self::fmt_type_expr(p)).collect();
                format!("{name:?}<{}>", inner.join(", "))
            }
            TypeExpr::Infer => "_".to_string(),
            TypeExpr::Unit => "()".to_string(),
            TypeExpr::Tuple(parts) => {
                let inner: Vec<String> = parts.iter().map(|p| Self::fmt_type_expr(p)).collect();
                format!("({})", inner.join(", "))
            }
            TypeExpr::Array(elem) => match elem.as_ref() {
                TypeExpr::TypeParam(n) => format!("[type {n}]"),
                e => format!("[{}]", Self::fmt_type_expr(e)),
            },
            TypeExpr::TypeParam(name) => name.clone(),
            TypeExpr::Function { params, ret } => {
                let mut p = Vec::new();
                for part in params {
                    let ty = Self::fmt_type_expr(&part.ty);
                    if let Some(n) = &part.name {
                        p.push(format!("{n}: {ty}"));
                    } else {
                        p.push(ty);
                    }
                }
                format!("({}) => {}", p.join(", "), Self::fmt_type_expr(ret))
            }
        }
    }
}

impl Visit for PrettyPrinter {
    fn visit_ast_node(&mut self, node: &AstNode) {
        match node {
            AstNode::Import {
                bindings,
                module_path,
                ..
            } => {
                let s = bindings
                    .iter()
                    .map(|(e, l)| {
                        if e == l {
                            e.clone()
                        } else {
                            format!("{e} as {l}")
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                self.line(&format!("Import {{ {s}, from: {:?} }}", module_path));
            }
            AstNode::ExportAlias { from, to, .. } => {
                self.line(&format!("ExportAlias {{ {from} as {to} }}"));
            }
            AstNode::Program(items) => {
                self.line("Program");
                self.indent += 1;
                for item in items {
                    self.visit_ast_node(item);
                }
                self.indent -= 1;
            }
            AstNode::SingleLineComment(text) => {
                self.line(&format!("SingleLineComment({text:?})"));
            }
            AstNode::MultiLineComment(text) => {
                self.line(&format!("MultiLineComment({text:?})"));
            }
            AstNode::IntegerLiteral {
                value,
                original,
                radix,
                ..
            } => {
                self.line(&format!(
                    "IntegerLiteral {{ value: {value}, original: {original:?}, radix: {radix} }}"
                ));
            }
            AstNode::FloatLiteral {
                original,
                cleaned,
                ..
            } => {
                self.line(&format!(
                    "FloatLiteral {{ original: {original:?}, cleaned: {cleaned:?} }}"
                ));
            }
            AstNode::StringLiteral { value, original, .. } => {
                self.line(&format!(
                    "StringLiteral {{ value: {value:?}, original: {original:?} }}"
                ));
            }
            AstNode::BoolLiteral { value, .. } => {
                self.line(&format!("BoolLiteral({value})"));
            }
            AstNode::Identifier { name, .. } => {
                self.line(&format!("Identifier({name:?})"));
            }
            AstNode::UnitLiteral { .. } => {
                self.line("UnitLiteral");
            }
            AstNode::TupleLiteral { elements, .. } => {
                self.line("TupleLiteral");
                self.indent += 1;
                for e in elements {
                    self.visit_ast_node(e);
                }
                self.indent -= 1;
            }
            AstNode::TupleField { base, index, .. } => {
                self.line(&format!("TupleField .{index}"));
                self.indent += 1;
                self.visit_ast_node(base);
                self.indent -= 1;
            }
            AstNode::ArrayLiteral { elements, .. } => {
                self.line("ArrayLiteral");
                self.indent += 1;
                for e in elements {
                    self.visit_ast_node(e);
                }
                self.indent -= 1;
            }
            AstNode::DictLiteral { entries, .. } => {
                self.line("DictLiteral");
                self.indent += 1;
                for (k, v) in entries {
                    self.line("Entry");
                    self.indent += 1;
                    self.visit_ast_node(k);
                    self.visit_ast_node(v);
                    self.indent -= 1;
                }
                self.indent -= 1;
            }
            AstNode::ArrayIndex { base, index, .. } => {
                self.line("ArrayIndex");
                self.indent += 1;
                self.visit_ast_node(base);
                self.visit_ast_node(index);
                self.indent -= 1;
            }
            AstNode::Lambda { params, body, .. } => {
                self.line(&format!(
                    "Lambda(params: {})",
                    params
                        .iter()
                        .map(|p| p.name.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
                self.indent += 1;
                match body.as_ref() {
                    crate::ast::LambdaBody::Expr(expr) => self.visit_ast_node(expr),
                    crate::ast::LambdaBody::Block(items) => {
                        for item in items {
                            self.visit_ast_node(item);
                        }
                    }
                }
                self.indent -= 1;
            }
            AstNode::BinaryOp { left, op, right, .. } => {
                self.line(&format!("BinaryOp {}", Self::fmt_binary_op(*op)));
                self.indent += 1;
                self.visit_ast_node(left);
                self.visit_ast_node(right);
                self.indent -= 1;
            }
            AstNode::UnaryOp { op, operand, .. } => {
                self.line(&format!("UnaryOp {}", Self::fmt_unary_op(*op)));
                self.indent += 1;
                self.visit_ast_node(operand);
                self.indent -= 1;
            }
            AstNode::Await { expr, .. } => {
                self.line("Await");
                self.indent += 1;
                self.visit_ast_node(expr);
                self.indent -= 1;
            }
            AstNode::InternalFunction {
                name,
                params,
                return_type,
                ..
            } => {
                let rt = return_type
                    .as_ref()
                    .map(|t| format!("Some({})", Self::fmt_type_expr(t)))
                    .unwrap_or_else(|| "None".to_string());
                self.line(&format!(
                    "InternalFunction {{ name: {name:?}, return_type: {rt} }}"
                ));
                self.indent += 1;
                for p in params {
                    let ty = Self::fmt_type_expr(&p.ty);
                    if let Some(def) = p.default_value.as_ref() {
                        self.line(&format!("param {}: {} =", p.name, ty));
                        self.indent += 1;
                        self.visit_ast_node(def.as_ref());
                        self.indent -= 1;
                    } else {
                        self.line(&format!("param {}: {}", p.name, ty));
                    }
                }
                self.indent -= 1;
            }
            AstNode::Function {
                name,
                params,
                return_type,
                body,
                ..
            } => {
                let rt = return_type
                    .as_ref()
                    .map(|t| format!("Some({})", Self::fmt_type_expr(t)))
                    .unwrap_or_else(|| "None".to_string());
                self.line(&format!("Function {{ name: {name:?}, return_type: {rt} }}"));
                self.indent += 1;
                for p in params {
                    let ty = Self::fmt_type_expr(&p.ty);
                    if let Some(def) = p.default_value.as_ref() {
                        self.line(&format!("param {}: {} =", p.name, ty));
                        self.indent += 1;
                        self.visit_ast_node(def.as_ref());
                        self.indent -= 1;
                    } else {
                        self.line(&format!("param {}: {}", p.name, ty));
                    }
                }
                self.line("body:");
                self.indent += 1;
                for stmt in body {
                    self.visit_ast_node(stmt);
                }
                self.indent -= 2;
            }
            AstNode::Call { callee, arguments, .. } => {
                self.line(&format!("Call({callee:?})"));
                self.indent += 1;
                for arg in arguments {
                    match arg {
                        CallArg::Positional(v) => self.visit_ast_node(v),
                        CallArg::Named { name, value, .. } => {
                            self.line(&format!("named_arg {name}"));
                            self.indent += 1;
                            self.visit_ast_node(value);
                            self.indent -= 1;
                        }
                    }
                }
                self.indent -= 1;
            }
            AstNode::Invoke {
                callee, arguments, ..
            } => {
                self.line("Invoke");
                self.indent += 1;
                self.visit_ast_node(callee.as_ref());
                for arg in arguments {
                    match arg {
                        CallArg::Positional(v) => self.visit_ast_node(v),
                        CallArg::Named { name, value, .. } => {
                            self.line(&format!("named_arg {name}"));
                            self.indent += 1;
                            self.visit_ast_node(value);
                            self.indent -= 1;
                        }
                    }
                }
                self.indent -= 1;
            }
            AstNode::MethodCall {
                receiver,
                method,
                arguments,
                ..
            } => {
                self.line(&format!("MethodCall(.{method})"));
                self.indent += 1;
                self.visit_ast_node(receiver.as_ref());
                for arg in arguments {
                    match arg {
                        CallArg::Positional(v) => self.visit_ast_node(v),
                        CallArg::Named { name, value, .. } => {
                            self.line(&format!("named_arg {name}"));
                            self.indent += 1;
                            self.visit_ast_node(value);
                            self.indent -= 1;
                        }
                    }
                }
                self.indent -= 1;
            }
            AstNode::TypeMethodCall {
                type_name,
                method,
                arguments,
                ..
            } => {
                self.line(&format!("TypeMethodCall({type_name}::{method})"));
                self.indent += 1;
                for arg in arguments {
                    match arg {
                        CallArg::Positional(v) => self.visit_ast_node(v),
                        CallArg::Named { name, value, .. } => {
                            self.line(&format!("named_arg {name}"));
                            self.indent += 1;
                            self.visit_ast_node(value);
                            self.indent -= 1;
                        }
                    }
                }
                self.indent -= 1;
            }
            AstNode::TypeValue { type_name, .. } => {
                self.line(&format!("TypeValue({type_name})"));
            }
            AstNode::StructDef { name, fields, .. } => {
                self.line(&format!("StructDef({name:?})"));
                self.indent += 1;
                for f in fields {
                    let ty = Self::fmt_type_expr(&f.ty);
                    self.line(&format!("field {}: {}", f.name, ty));
                }
                self.indent -= 1;
            }
            AstNode::StructLiteral {
                name,
                fields,
                update,
                ..
            } => {
                self.line(&format!("StructLiteral({name:?})"));
                self.indent += 1;
                for (_, v) in fields {
                    self.visit_ast_node(v);
                }
                if let Some(u) = update {
                    self.line("update:");
                    self.indent += 1;
                    self.visit_ast_node(u.as_ref());
                    self.indent -= 1;
                }
                self.indent -= 1;
            }
            AstNode::FieldAccess { field, base, .. } => {
                self.line(&format!("FieldAccess({field:?})"));
                self.indent += 1;
                self.visit_ast_node(base);
                self.indent -= 1;
            }
            AstNode::EnumDef {
                name, type_params, variants, ..
            } => {
                self.line(&format!(
                    "EnumDef({name:?}, params={})",
                    type_params.len()
                ));
                self.indent += 1;
                for v in variants {
                    self.line(&format!("variant {}()", v.name));
                    if !v.payload_types.is_empty() {
                        self.line(&format!(
                            "payloads: {}",
                            v.payload_types.len()
                        ));
                    }
                }
                self.indent -= 1;
            }
            AstNode::TypeAlias {
                name,
                type_params,
                target,
                ..
            } => {
                self.line(&format!(
                    "TypeAlias({name:?}, params={}, target={})",
                    type_params.len(),
                    Self::fmt_type_expr(target)
                ));
            }
            AstNode::EnumVariantCtor {
                enum_name,
                variant,
                payloads,
                ..
            } => {
                self.line(&format!(
                    "EnumVariantCtor({enum_name:?}::{variant:?})"
                ));
                self.indent += 1;
                for p in payloads {
                    self.visit_ast_node(p);
                }
                self.indent -= 1;
            }
            AstNode::Return { value, .. } => {
                if let Some(v) = value {
                    self.line("Return");
                    self.indent += 1;
                    self.visit_ast_node(v);
                    self.indent -= 1;
                } else {
                    self.line("Return ()");
                }
            }
            AstNode::Let {
                type_annotation,
                initializer,
                ..
            } => {
                let tn = type_annotation
                    .as_ref()
                    .map(|t| format!("Some({})", Self::fmt_type_expr(t)))
                    .unwrap_or_else(|| "None".to_string());
                self.line(&format!("Let {{ type: {tn} }}"));
                if let Some(v) = initializer {
                    self.indent += 1;
                    self.visit_ast_node(v.as_ref());
                    self.indent -= 1;
                }
            }
            AstNode::Assign { value, .. } => {
                self.line("Assign");
                self.indent += 1;
                self.visit_ast_node(value.as_ref());
                self.indent -= 1;
            }
            AstNode::AssignExpr { lhs, rhs, .. } => {
                self.line("AssignExpr");
                self.indent += 1;
                self.visit_ast_node(lhs.as_ref());
                self.visit_ast_node(rhs.as_ref());
                self.indent -= 1;
            }
            AstNode::CompoundAssign { lhs, op, rhs, .. } => {
                self.line(&format!(
                    "CompoundAssign {}",
                    Self::fmt_compound_op(*op)
                ));
                self.indent += 1;
                self.visit_ast_node(lhs.as_ref());
                self.visit_ast_node(rhs.as_ref());
                self.indent -= 1;
            }
            AstNode::Block { body, .. } => {
                self.line("Block");
                self.indent += 1;
                for stmt in body {
                    self.visit_ast_node(stmt);
                }
                self.indent -= 1;
            }
            AstNode::If {
                condition,
                then_body,
                else_body,
                ..
            } => {
                self.line("If");
                self.indent += 1;
                self.visit_ast_node(condition.as_ref());
                self.line("then:");
                self.indent += 1;
                for stmt in then_body {
                    self.visit_ast_node(stmt);
                }
                self.indent -= 1;
                if let Some(else_b) = else_body {
                    self.line("else:");
                    self.indent += 1;
                    for stmt in else_b {
                        self.visit_ast_node(stmt);
                    }
                    self.indent -= 1;
                }
                self.indent -= 1;
            }
            AstNode::IfLet {
                value,
                then_body,
                else_body,
                ..
            } => {
                self.line("IfLet");
                self.indent += 1;
                self.visit_ast_node(value.as_ref());
                self.line("then:");
                self.indent += 1;
                for stmt in then_body {
                    self.visit_ast_node(stmt);
                }
                self.indent -= 1;
                if let Some(else_b) = else_body {
                    self.line("else:");
                    self.indent += 1;
                    for stmt in else_b {
                        self.visit_ast_node(stmt);
                    }
                    self.indent -= 1;
                }
                self.indent -= 1;
            }
            AstNode::Match {
                scrutinee,
                arms,
                ..
            } => {
                self.line("Match");
                self.indent += 1;
                self.visit_ast_node(scrutinee.as_ref());
                for (i, arm) in arms.iter().enumerate() {
                    self.line(&format!("arm[{i}]"));
                    self.indent += 1;
                    if let Some(g) = arm.guard.as_ref() {
                        self.line("guard");
                        self.indent += 1;
                        self.visit_ast_node(g.as_ref());
                        self.indent -= 1;
                    }
                    self.line("body:");
                    self.indent += 1;
                    self.visit_ast_node(arm.body.as_ref());
                    self.indent -= 1;
                    self.indent -= 1;
                }
                self.indent -= 1;
            }
            AstNode::While {
                condition,
                body,
                ..
            } => {
                self.line("While");
                self.indent += 1;
                self.visit_ast_node(condition.as_ref());
                self.line("body:");
                self.indent += 1;
                for stmt in body {
                    self.visit_ast_node(stmt);
                }
                self.indent -= 2;
            }
            AstNode::Break { .. } => {
                self.line("Break");
            }
            AstNode::Continue { .. } => {
                self.line("Continue");
            }
        }
    }
}
