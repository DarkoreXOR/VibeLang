use std::collections::{HashMap, HashSet};

use crate::ast::{AstNode, CallArg, MatchArm, Pattern, PatternElem, TypeExpr};
use crate::error::{SemanticWarning, Span};
use crate::parser::Parser;

#[derive(Clone)]
struct BindingInfo {
    span: Span,
    used: bool,
    kind: BindingKind,
}

#[derive(Clone, Copy)]
enum BindingKind {
    Parameter,
    Binding,
}

#[derive(Default)]
struct UseCollector {
    used_value_names: HashSet<String>,
    used_type_names: HashSet<String>,
    function_generic_uses: HashMap<String, HashSet<String>>,
}

struct FnCtx<'a> {
    warnings: &'a mut Vec<SemanticWarning>,
    collector: &'a mut UseCollector,
    fn_name: &'a str,
    function_type_params: &'a HashSet<String>,
    scopes: Vec<HashMap<String, BindingInfo>>,
}

impl<'a> FnCtx<'a> {
    fn new(
        warnings: &'a mut Vec<SemanticWarning>,
        collector: &'a mut UseCollector,
        fn_name: &'a str,
        function_type_params: &'a HashSet<String>,
    ) -> Self {
        Self {
            warnings,
            collector,
            fn_name,
            function_type_params,
            scopes: vec![HashMap::new()],
        }
    }

    fn push_scope(&mut self) {
        self.scopes.push(HashMap::new());
    }

    fn pop_scope(&mut self) {
        let Some(scope) = self.scopes.pop() else {
            return;
        };
        for (name, info) in scope {
            if !info.used && !name.starts_with('_') {
                let what = match info.kind {
                    BindingKind::Parameter => "parameter",
                    BindingKind::Binding => "binding",
                };
                self.warnings.push(SemanticWarning::new(
                    format!("unused {what} `{name}`"),
                    info.span,
                ));
            }
        }
    }

    fn declare_binding(&mut self, name: &str, span: Span, kind: BindingKind) {
        if name == "_" || name.starts_with('_') {
            return;
        }
        if let Some(scope) = self.scopes.last_mut() {
            scope.entry(name.to_string()).or_insert(BindingInfo {
                span,
                used: false,
                kind,
            });
        }
    }

    fn mark_name_use(&mut self, name: &str) {
        for scope in self.scopes.iter_mut().rev() {
            if let Some(info) = scope.get_mut(name) {
                info.used = true;
                return;
            }
        }
        self.collector.used_value_names.insert(name.to_string());
    }

    fn walk_pattern_use(&mut self, pattern: &Pattern) {
        match pattern {
            Pattern::Wildcard { .. } => {}
            Pattern::Binding { name, .. } => self.mark_name_use(name),
            Pattern::IntLiteral { .. }
            | Pattern::StringLiteral { .. }
            | Pattern::BoolLiteral { .. } => {}
            Pattern::Tuple { elements, .. } | Pattern::Array { elements, .. } => {
                for e in elements {
                    if let PatternElem::Pattern(p) = e {
                        self.walk_pattern_use(p);
                    }
                }
            }
            Pattern::Struct {
                name,
                type_args,
                fields,
                ..
            } => {
                self.collector.used_type_names.insert(name.clone());
                for ta in type_args {
                    self.walk_type_expr(ta);
                }
                for f in fields {
                    self.walk_pattern_use(&f.pattern);
                }
            }
            Pattern::EnumVariant {
                enum_name,
                type_args,
                payloads,
                ..
            } => {
                self.collector.used_type_names.insert(enum_name.clone());
                for ta in type_args {
                    self.walk_type_expr(ta);
                }
                for p in payloads {
                    self.walk_pattern_use(p);
                }
            }
        }
    }

    fn declare_pattern_bindings(&mut self, pattern: &Pattern) {
        match pattern {
            Pattern::Wildcard { .. } => {}
            Pattern::Binding { name, name_span } => {
                self.declare_binding(name, *name_span, BindingKind::Binding)
            }
            Pattern::IntLiteral { .. }
            | Pattern::StringLiteral { .. }
            | Pattern::BoolLiteral { .. } => {}
            Pattern::Tuple { elements, .. } | Pattern::Array { elements, .. } => {
                for e in elements {
                    if let PatternElem::Pattern(p) = e {
                        self.declare_pattern_bindings(p);
                    }
                }
            }
            Pattern::Struct {
                name,
                type_args,
                fields,
                ..
            } => {
                self.collector.used_type_names.insert(name.clone());
                for ta in type_args {
                    self.walk_type_expr(ta);
                }
                for f in fields {
                    self.declare_pattern_bindings(&f.pattern);
                }
            }
            Pattern::EnumVariant {
                enum_name,
                type_args,
                payloads,
                ..
            } => {
                self.collector.used_type_names.insert(enum_name.clone());
                for ta in type_args {
                    self.walk_type_expr(ta);
                }
                for p in payloads {
                    self.declare_pattern_bindings(p);
                }
            }
        }
    }

    fn walk_call_args(&mut self, args: &[CallArg]) {
        for arg in args {
            match arg {
                CallArg::Positional(v) => self.walk_expr(v),
                CallArg::Named { value, .. } => self.walk_expr(value),
            }
        }
    }

    fn record_generic_use(&mut self, name: &str) {
        if self.function_type_params.contains(name) {
            self.collector
                .function_generic_uses
                .entry(self.fn_name.to_string())
                .or_default()
                .insert(name.to_string());
        }
    }

    fn walk_type_expr(&mut self, ty: &TypeExpr) {
        match ty {
            TypeExpr::Named(name) => {
                self.collector.used_type_names.insert(name.clone());
            }
            TypeExpr::EnumApp { name, args } => {
                self.collector.used_type_names.insert(name.clone());
                for a in args {
                    self.walk_type_expr(a);
                }
            }
            TypeExpr::TypeParam(name) => self.record_generic_use(name),
            TypeExpr::Tuple(parts) => {
                for p in parts {
                    self.walk_type_expr(p);
                }
            }
            TypeExpr::Array(inner) => self.walk_type_expr(inner),
            TypeExpr::Function { params, ret } => {
                for p in params {
                    if let Some(n) = &p.name {
                        self.record_generic_use(n);
                    }
                    self.walk_type_expr(&p.ty);
                }
                self.walk_type_expr(ret);
            }
            TypeExpr::Infer | TypeExpr::Unit => {}
        }
    }

    fn walk_expr(&mut self, node: &AstNode) {
        match node {
            AstNode::Identifier { name, .. } => self.mark_name_use(name),
            AstNode::Call {
                callee,
                type_args,
                arguments,
                ..
            } => {
                self.mark_name_use(callee);
                for ta in type_args {
                    self.walk_type_expr(ta);
                }
                self.walk_call_args(arguments);
            }
            AstNode::Invoke {
                callee, arguments, ..
            } => {
                self.walk_expr(callee);
                self.walk_call_args(arguments);
            }
            AstNode::MethodCall {
                receiver,
                arguments,
                ..
            } => {
                self.walk_expr(receiver);
                self.walk_call_args(arguments);
            }
            AstNode::TypeMethodCall {
                type_name,
                arguments,
                ..
            } => {
                if let Ok(te) = Parser::parse_type_expr_from_source(type_name) {
                    self.walk_type_expr(&te);
                } else {
                    self.collector.used_type_names.insert(type_name.clone());
                }
                self.walk_call_args(arguments);
            }
            AstNode::TypeValue { type_name, .. } => {
                if let Ok(te) = Parser::parse_type_expr_from_source(type_name) {
                    self.walk_type_expr(&te);
                } else {
                    self.collector.used_type_names.insert(type_name.clone());
                }
            }
            AstNode::StructLiteral {
                name,
                type_args,
                fields,
                update,
                ..
            } => {
                self.collector.used_type_names.insert(name.clone());
                for ta in type_args {
                    self.walk_type_expr(ta);
                }
                for (_, v) in fields {
                    self.walk_expr(v);
                }
                if let Some(u) = update {
                    self.walk_expr(u);
                }
            }
            AstNode::EnumVariantCtor {
                enum_name,
                type_args,
                payloads,
                ..
            } => {
                self.collector.used_type_names.insert(enum_name.clone());
                for ta in type_args {
                    self.walk_type_expr(ta);
                }
                for p in payloads {
                    self.walk_expr(p);
                }
            }
            AstNode::Let {
                pattern,
                type_annotation,
                initializer,
                ..
            } => {
                if let Some(ann) = type_annotation {
                    self.walk_type_expr(ann);
                }
                if let Some(init) = initializer {
                    self.walk_expr(init);
                }
                self.declare_pattern_bindings(pattern);
            }
            AstNode::Assign { pattern, value, .. } => {
                self.walk_pattern_use(pattern);
                self.walk_expr(value);
            }
            AstNode::AssignExpr { lhs, rhs, .. }
            | AstNode::CompoundAssign { lhs, rhs, .. } => {
                self.walk_expr(lhs);
                self.walk_expr(rhs);
            }
            AstNode::Return { value, .. } => {
                if let Some(v) = value {
                    self.walk_expr(v);
                }
            }
            AstNode::If {
                condition,
                then_body,
                else_body,
                ..
            } => {
                self.walk_expr(condition);
                self.push_scope();
                for s in then_body {
                    self.walk_expr(s);
                }
                self.pop_scope();
                if let Some(else_items) = else_body {
                    self.push_scope();
                    for s in else_items {
                        self.walk_expr(s);
                    }
                    self.pop_scope();
                }
            }
            AstNode::IfLet {
                pattern,
                value,
                then_body,
                else_body,
                ..
            } => {
                self.walk_expr(value);
                self.push_scope();
                self.declare_pattern_bindings(pattern);
                for s in then_body {
                    self.walk_expr(s);
                }
                self.pop_scope();
                if let Some(else_items) = else_body {
                    self.push_scope();
                    for s in else_items {
                        self.walk_expr(s);
                    }
                    self.pop_scope();
                }
            }
            AstNode::While {
                condition, body, ..
            } => {
                self.walk_expr(condition);
                self.push_scope();
                for s in body {
                    self.walk_expr(s);
                }
                self.pop_scope();
            }
            AstNode::Match { scrutinee, arms, .. } => {
                self.walk_expr(scrutinee);
                for arm in arms {
                    self.walk_match_arm(arm);
                }
            }
            AstNode::Block { body, .. } => {
                self.push_scope();
                for s in body {
                    self.walk_expr(s);
                }
                self.pop_scope();
            }
            AstNode::Lambda { params, body, .. } => {
                self.push_scope();
                for p in params {
                    self.declare_binding(&p.name, p.name_span, BindingKind::Parameter);
                }
                match body.as_ref() {
                    crate::ast::LambdaBody::Expr(e) => self.walk_expr(e),
                    crate::ast::LambdaBody::Block(items) => {
                        for s in items {
                            self.walk_expr(s);
                        }
                    }
                }
                self.pop_scope();
            }
            AstNode::TupleLiteral { elements, .. } | AstNode::ArrayLiteral { elements, .. } => {
                for e in elements {
                    self.walk_expr(e);
                }
            }
            AstNode::DictLiteral { entries, .. } => {
                for (k, v) in entries {
                    self.walk_expr(k);
                    self.walk_expr(v);
                }
            }
            AstNode::TupleField { base, .. } => self.walk_expr(base),
            AstNode::ArrayIndex { base, index, .. } => {
                self.walk_expr(base);
                self.walk_expr(index);
            }
            AstNode::BinaryOp { left, right, .. } => {
                self.walk_expr(left);
                self.walk_expr(right);
            }
            AstNode::UnaryOp { operand, .. } | AstNode::Await { expr: operand, .. } => {
                self.walk_expr(operand)
            }
            AstNode::FieldAccess { base, .. } => self.walk_expr(base),
            AstNode::Program(items) => {
                for it in items {
                    self.walk_expr(it);
                }
            }
            AstNode::InternalFunction { .. }
            | AstNode::Function { .. }
            | AstNode::Import { .. }
            | AstNode::ExportAlias { .. }
            | AstNode::StructDef { .. }
            | AstNode::EnumDef { .. }
            | AstNode::TypeAlias { .. }
            | AstNode::SingleLineComment(_)
            | AstNode::MultiLineComment(_)
            | AstNode::IntegerLiteral { .. }
            | AstNode::FloatLiteral { .. }
            | AstNode::StringLiteral { .. }
            | AstNode::BoolLiteral { .. }
            | AstNode::UnitLiteral { .. }
            | AstNode::Break { .. }
            | AstNode::Continue { .. } => {}
        }
    }

    fn walk_match_arm(&mut self, arm: &MatchArm) {
        self.push_scope();
        for p in &arm.patterns {
            self.declare_pattern_bindings(p);
        }
        if let Some(g) = arm.guard.as_ref() {
            self.walk_expr(g);
        }
        self.walk_expr(arm.body.as_ref());
        self.pop_scope();
    }
}

pub fn collect_unused_warnings(ast: &AstNode) -> Vec<SemanticWarning> {
    let AstNode::Program(items) = ast else {
        return Vec::new();
    };

    let mut warnings = Vec::new();
    let mut collector = UseCollector::default();

    let mut import_bindings: HashMap<String, Span> = HashMap::new();
    let mut top_functions: HashMap<String, Span> = HashMap::new();
    let mut top_structs: HashMap<String, Span> = HashMap::new();
    let mut top_enums: HashMap<String, Span> = HashMap::new();
    let mut top_aliases: HashMap<String, Span> = HashMap::new();
    let mut top_globals: HashMap<String, Span> = HashMap::new();
    let mut function_type_params: HashMap<String, Vec<(String, Span)>> = HashMap::new();

    for item in items {
        match item {
            AstNode::Import { bindings, .. } => {
                for b in bindings {
                    import_bindings
                        .entry(b.local_name.clone())
                        .or_insert(b.local_span);
                }
            }
            AstNode::InternalFunction {
                name,
                name_span,
                type_params,
                params,
                return_type,
                ..
            }
            | AstNode::Function {
                name,
                name_span,
                type_params,
                params,
                return_type,
                ..
            } => {
                top_functions.insert(name.clone(), *name_span);
                let tps = type_params
                    .iter()
                    .map(|p| (p.name.clone(), p.name_span))
                    .collect::<Vec<_>>();
                function_type_params.insert(name.clone(), tps);

                let tp_set = type_params
                    .iter()
                    .map(|p| p.name.clone())
                    .collect::<HashSet<_>>();
                let mut ctx = FnCtx::new(&mut warnings, &mut collector, name, &tp_set);
                for p in params {
                    ctx.declare_binding(&p.name, p.name_span, BindingKind::Parameter);
                    ctx.walk_type_expr(&p.ty);
                    if let Some(def) = p.default_value.as_ref() {
                        ctx.walk_expr(def.as_ref());
                    }
                }
                if let Some(ret) = return_type {
                    ctx.walk_type_expr(ret);
                }
                if let AstNode::Function {
                    extension_receiver,
                    body,
                    ..
                } = item
                {
                    if let Some(ext) = extension_receiver {
                        ctx.walk_type_expr(&ext.ty);
                    }
                    for stmt in body {
                        ctx.walk_expr(stmt);
                    }
                }
                ctx.pop_scope();
            }
            AstNode::StructDef {
                name,
                name_span,
                type_params: _,
                ..
            } => {
                top_structs.insert(name.clone(), *name_span);
            }
            AstNode::EnumDef {
                name,
                name_span,
                type_params: _,
                ..
            } => {
                top_enums.insert(name.clone(), *name_span);
            }
            AstNode::TypeAlias {
                name,
                name_span,
                target,
                ..
            } => {
                top_aliases.insert(name.clone(), *name_span);
                // Track aliases referenced in alias RHS types.
                let empty: HashSet<String> = HashSet::new();
                let mut ctx = FnCtx::new(&mut warnings, &mut collector, "<type>", &empty);
                ctx.walk_type_expr(target);
            }
            AstNode::Let { pattern, .. } => {
                collect_top_pattern_bindings(pattern, &mut top_globals);
            }
            _ => {}
        }
    }

    for item in items {
        if let AstNode::Let {
            type_annotation,
            initializer,
            ..
        } = item
        {
            let empty: HashSet<String> = HashSet::new();
            let mut ctx = FnCtx::new(&mut warnings, &mut collector, "<top>", &empty);
            if let Some(ann) = type_annotation {
                ctx.walk_type_expr(ann);
            }
            if let Some(init) = initializer {
                ctx.walk_expr(init);
            }
            ctx.pop_scope();
        }
    }

    for (name, span) in import_bindings {
        if !collector.used_value_names.contains(&name) && !name.starts_with('_') {
            warnings.push(SemanticWarning::new(format!("unused import `{name}`"), span));
        }
    }
    for (name, span) in top_functions {
        if name != "main" && !collector.used_value_names.contains(&name) && !name.starts_with('_') {
            warnings.push(SemanticWarning::new(
                format!("unused function `{name}`"),
                span,
            ));
        }
    }
    for (name, span) in top_structs {
        if !collector.used_type_names.contains(&name) && !name.starts_with('_') {
            warnings.push(SemanticWarning::new(format!("unused struct `{name}`"), span));
        }
    }
    for (name, span) in top_enums {
        if !collector.used_type_names.contains(&name) && !name.starts_with('_') {
            warnings.push(SemanticWarning::new(format!("unused enum `{name}`"), span));
        }
    }
    for (name, span) in top_aliases {
        if !collector.used_type_names.contains(&name) && !name.starts_with('_') {
            warnings.push(SemanticWarning::new(format!("unused type alias `{name}`"), span));
        }
    }
    for (name, span) in top_globals {
        if !collector.used_value_names.contains(&name) && !name.starts_with('_') {
            warnings.push(SemanticWarning::new(format!("unused global `{name}`"), span));
        }
    }
    for (fn_name, params) in function_type_params {
        let used = collector
            .function_generic_uses
            .get(&fn_name)
            .cloned()
            .unwrap_or_default();
        for (tp, span) in params {
            if !used.contains(&tp) && !tp.starts_with('_') {
                warnings.push(SemanticWarning::new(
                    format!("unused generic type parameter `{tp}`"),
                    span,
                ));
            }
        }
    }

    warnings.sort_by(|a, b| {
        a.span
            .line
            .cmp(&b.span.line)
            .then_with(|| a.span.column.cmp(&b.span.column))
            .then_with(|| a.message.cmp(&b.message))
    });
    warnings
}

fn collect_top_pattern_bindings(pattern: &Pattern, out: &mut HashMap<String, Span>) {
    match pattern {
        Pattern::Wildcard { .. } => {}
        Pattern::Binding { name, name_span } => {
            out.entry(name.clone()).or_insert(*name_span);
        }
        Pattern::IntLiteral { .. } | Pattern::StringLiteral { .. } | Pattern::BoolLiteral { .. } => {}
        Pattern::Tuple { elements, .. } | Pattern::Array { elements, .. } => {
            for e in elements {
                if let PatternElem::Pattern(p) = e {
                    collect_top_pattern_bindings(p, out);
                }
            }
        }
        Pattern::Struct { fields, .. } => {
            for f in fields {
                collect_top_pattern_bindings(&f.pattern, out);
            }
        }
        Pattern::EnumVariant { payloads, .. } => {
            for p in payloads {
                collect_top_pattern_bindings(p, out);
            }
        }
    }
}
