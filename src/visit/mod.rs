//! Immutable AST traversal (`Visit` + `walk_*`), similar in spirit to `syn::visit`.

mod pretty;

pub use pretty::pretty_print_ast;

use crate::ast::{AstNode, BinaryOp, CallArg, Param, TypeExpr, UnaryOp};

/// Visitor over [`AstNode`]. Override specific `visit_*` methods; defaults recurse in a
/// predictable order. For custom `visit_*` bodies, call the corresponding `walk_*`
/// function to retain default child traversal.
pub trait Visit {
    fn visit_ast_node(&mut self, node: &AstNode) {
        walk_ast_node(self, node);
    }

    fn visit_program(&mut self, items: &[AstNode]) {
        walk_program(self, items);
    }

    fn visit_single_line_comment(&mut self, _text: &str) {}

    fn visit_multi_line_comment(&mut self, _text: &str) {}

    fn visit_integer_literal(&mut self, _value: u64, _original: &str, _radix: u32) {}

    fn visit_float_literal(
        &mut self,
        _original: &str,
        _cleaned: &str,
    ) {
    }

    fn visit_string_literal(&mut self, _value: &str, _original: &str) {}

    fn visit_identifier(&mut self, _name: &str) {}

    fn visit_binary_op(&mut self, left: &AstNode, _op: BinaryOp, right: &AstNode) {
        walk_binary_op_children(self, left, right);
    }

    fn visit_unary_op(&mut self, _op: UnaryOp, operand: &AstNode) {
        walk_unary_op_operand(self, operand);
    }

    fn visit_internal_function(
        &mut self,
        _name: &str,
        _params: &[Param],
        _return_type: Option<&TypeExpr>,
    ) {
    }

    fn visit_function(
        &mut self,
        _name: &str,
        _params: &[Param],
        _return_type: Option<&TypeExpr>,
        body: &[AstNode],
    ) {
        walk_function_body(self, body);
    }

    fn visit_call(&mut self, _callee: &str, arguments: &[CallArg]) {
        walk_call_arguments(self, arguments);
    }

    fn visit_return(&mut self, value: Option<&AstNode>) {
        walk_return_value(self, value);
    }

    fn visit_let(
        &mut self,
        _type_annotation: Option<&TypeExpr>,
        initializer: Option<&AstNode>,
    ) {
        walk_optional_initializer(self, initializer);
    }

    fn visit_assign(&mut self, value: &AstNode) {
        self.visit_ast_node(value);
    }

    fn visit_block(&mut self, body: &[AstNode]) {
        walk_block_body(self, body);
    }
}

pub fn walk_ast_node<V: Visit + ?Sized>(visitor: &mut V, node: &AstNode) {
    match node {
        AstNode::Import { .. } | AstNode::ExportAlias { .. } => {}
        AstNode::Program(items) => visitor.visit_program(items),
        AstNode::SingleLineComment(text) => visitor.visit_single_line_comment(text),
        AstNode::MultiLineComment(text) => visitor.visit_multi_line_comment(text),
        AstNode::IntegerLiteral {
            value,
            original,
            radix,
            ..
        } => visitor.visit_integer_literal(*value, original, *radix),
        AstNode::FloatLiteral {
            original,
            cleaned,
            ..
        } => visitor.visit_float_literal(original, cleaned),
        AstNode::StringLiteral { value, original, .. } => {
            visitor.visit_string_literal(value, original);
        }
        AstNode::BoolLiteral { .. } => {}
        AstNode::Identifier { name, .. } => visitor.visit_identifier(name),
        AstNode::UnitLiteral { .. } => {}
        AstNode::TupleLiteral { elements, .. } => {
            for e in elements {
                visitor.visit_ast_node(e);
            }
        }
        AstNode::ArrayLiteral { elements, .. } => {
            for e in elements {
                visitor.visit_ast_node(e);
            }
        }
        AstNode::DictLiteral { entries, .. } => {
            for (k, v) in entries {
                visitor.visit_ast_node(k);
                visitor.visit_ast_node(v);
            }
        }
        AstNode::TupleField { base, .. } => visitor.visit_ast_node(base),
        AstNode::ArrayIndex { base, index, .. } => {
            visitor.visit_ast_node(base);
            visitor.visit_ast_node(index);
        }
        AstNode::BinaryOp { left, op, right, .. } => visitor.visit_binary_op(left, *op, right),
        AstNode::UnaryOp { op, operand, .. } => visitor.visit_unary_op(*op, operand),
        AstNode::Await { expr, .. } => visitor.visit_ast_node(expr.as_ref()),
        AstNode::InternalFunction {
            name,
            params,
            return_type,
            ..
        } => {
            for p in params {
                if let Some(def) = p.default_value.as_ref() {
                    visitor.visit_ast_node(def.as_ref());
                }
            }
            visitor.visit_internal_function(name, params, return_type.as_ref())
        }
        AstNode::Function {
            name,
            params,
            return_type,
            body,
            ..
        } => {
            for p in params {
                if let Some(def) = p.default_value.as_ref() {
                    visitor.visit_ast_node(def.as_ref());
                }
            }
            visitor.visit_function(name, params, return_type.as_ref(), body)
        }
        AstNode::Call { callee, arguments, .. } => visitor.visit_call(callee, arguments),
        AstNode::Invoke {
            callee, arguments, ..
        } => {
            visitor.visit_ast_node(callee.as_ref());
            walk_call_arguments(visitor, arguments);
        }
        AstNode::MethodCall {
            receiver,
            arguments,
            ..
        } => {
            visitor.visit_ast_node(receiver.as_ref());
            walk_call_arguments(visitor, arguments);
        }
        AstNode::TypeMethodCall { arguments, .. } => walk_call_arguments(visitor, arguments),
        AstNode::TypeValue { .. } => {}
        AstNode::StructDef { .. } => {}
        AstNode::StructLiteral { fields, update, .. } => {
            for (_, v) in fields {
                visitor.visit_ast_node(v);
            }
            if let Some(u) = update {
                visitor.visit_ast_node(u.as_ref());
            }
        }
        AstNode::FieldAccess { base, .. } => visitor.visit_ast_node(base),
        AstNode::EnumDef { .. } => {}
        AstNode::TypeAlias { .. } => {}
        AstNode::EnumVariantCtor { payloads, .. } => {
            for p in payloads {
                visitor.visit_ast_node(p);
            }
        }
        AstNode::Return { value, .. } => {
            let opt = value.as_ref().map(|b| b.as_ref());
            visitor.visit_return(opt);
        }
        AstNode::Let {
            type_annotation,
            initializer,
            ..
        } => visitor.visit_let(type_annotation.as_ref(), initializer.as_deref()),
        AstNode::Assign { value, .. } => visitor.visit_assign(value.as_ref()),
        AstNode::Block { body, .. } => visitor.visit_block(body),
        AstNode::If {
            condition,
            then_body,
            else_body,
            ..
        } => {
            visitor.visit_ast_node(condition.as_ref());
            for s in then_body {
                visitor.visit_ast_node(s);
            }
            if let Some(else_b) = else_body {
                for s in else_b {
                    visitor.visit_ast_node(s);
                }
            }
        }
        AstNode::IfLet {
            value,
            then_body,
            else_body,
            ..
        } => {
            visitor.visit_ast_node(value.as_ref());
            for s in then_body {
                visitor.visit_ast_node(s);
            }
            if let Some(else_b) = else_body {
                for s in else_b {
                    visitor.visit_ast_node(s);
                }
            }
        }
        AstNode::Match { scrutinee, arms, .. } => {
            visitor.visit_ast_node(scrutinee.as_ref());
            for arm in arms {
                for p in &arm.patterns {
                    // Patterns don't contain nested expressions, so nothing to walk here.
                    let _ = p;
                }
                if let Some(g) = arm.guard.as_ref() {
                    visitor.visit_ast_node(g.as_ref());
                }
                visitor.visit_ast_node(arm.body.as_ref());
            }
        }
        AstNode::AssignExpr { lhs, rhs, .. } => {
            visitor.visit_ast_node(lhs.as_ref());
            visitor.visit_ast_node(rhs.as_ref());
        }
        AstNode::CompoundAssign { lhs, rhs, .. } => {
            visitor.visit_ast_node(lhs.as_ref());
            visitor.visit_ast_node(rhs.as_ref());
        }
        AstNode::While { condition, body, .. } => {
            visitor.visit_ast_node(condition.as_ref());
            for s in body {
                visitor.visit_ast_node(s);
            }
        }
        AstNode::Lambda { body, .. } => match body.as_ref() {
            crate::ast::LambdaBody::Expr(expr) => visitor.visit_ast_node(expr),
            crate::ast::LambdaBody::Block(items) => {
                for s in items {
                    visitor.visit_ast_node(s);
                }
            }
        },
        AstNode::Break { .. } | AstNode::Continue { .. } => {}
    }
}

pub fn walk_program<V: Visit + ?Sized>(visitor: &mut V, items: &[AstNode]) {
    for item in items {
        visitor.visit_ast_node(item);
    }
}

pub fn walk_function_body<V: Visit + ?Sized>(visitor: &mut V, body: &[AstNode]) {
    for stmt in body {
        visitor.visit_ast_node(stmt);
    }
}

pub fn walk_binary_op_children<V: Visit + ?Sized>(visitor: &mut V, left: &AstNode, right: &AstNode) {
    visitor.visit_ast_node(left);
    visitor.visit_ast_node(right);
}

pub fn walk_unary_op_operand<V: Visit + ?Sized>(visitor: &mut V, operand: &AstNode) {
    visitor.visit_ast_node(operand);
}

pub fn walk_call_arguments<V: Visit + ?Sized>(visitor: &mut V, arguments: &[CallArg]) {
    for arg in arguments {
        match arg {
            CallArg::Positional(v) => visitor.visit_ast_node(v),
            CallArg::Named { value, .. } => visitor.visit_ast_node(value),
        }
    }
}

pub fn walk_return_value<V: Visit + ?Sized>(visitor: &mut V, value: Option<&AstNode>) {
    if let Some(v) = value {
        visitor.visit_ast_node(v);
    }
}

pub fn walk_optional_initializer<V: Visit + ?Sized>(visitor: &mut V, init: Option<&AstNode>) {
    if let Some(e) = init {
        visitor.visit_ast_node(e);
    }
}

pub fn walk_block_body<V: Visit + ?Sized>(visitor: &mut V, body: &[AstNode]) {
    for stmt in body {
        visitor.visit_ast_node(stmt);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::Lexer;
    use crate::parser::Parser;

    struct CountVisit {
        nodes: usize,
        identifiers: usize,
        calls: usize,
    }

    impl Visit for CountVisit {
        fn visit_ast_node(&mut self, node: &AstNode) {
            self.nodes += 1;
            walk_ast_node(self, node);
        }

        fn visit_identifier(&mut self, _name: &str) {
            self.identifiers += 1;
        }

        fn visit_call(&mut self, callee: &str, arguments: &[CallArg]) {
            let _ = callee;
            self.calls += 1;
            walk_call_arguments(self, arguments);
        }
    }

    fn parse_example4() -> AstNode {
        let src = include_str!("../../examples/example4.vc");
        let mut lexer = Lexer::new(src);
        let tokens = lexer.tokenize().expect("lex");
        let mut parser = Parser::new(tokens);
        parser.parse().expect("parse")
    }

    #[test]
    fn walk_visits_all_nodes_at_least_once() {
        let ast = parse_example4();
        let mut v = CountVisit {
            nodes: 0,
            identifiers: 0,
            calls: 0,
        };
        v.visit_ast_node(&ast);
        assert!(v.nodes >= 50, "nodes={}", v.nodes);
        assert!(v.identifiers >= 10, "idents={}", v.identifiers);
        assert!(v.calls >= 10, "calls={}", v.calls);
    }

    #[test]
    fn pretty_print_example4_is_stable_nonempty() {
        let ast = parse_example4();
        let s = pretty_print_ast(&ast);
        assert!(s.contains("Function") && s.contains("\"add\""));
        assert!(s.contains("BinaryOp"));
        assert!(s.len() > 200);
    }
}
