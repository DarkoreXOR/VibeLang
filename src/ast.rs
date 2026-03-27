//! Abstract syntax tree for Vibelang sources.

use crate::error::Span;

#[derive(Debug, Clone, PartialEq)]
pub struct Param {
    pub name: String,
    pub name_span: Span,
    /// `true` for `_: T` parameters (name is `"_"`).
    pub is_wildcard: bool,
    /// `true` when declared as `params name: [T]`.
    pub is_params: bool,
    pub ty: TypeExpr,
    /// Optional default value expression, evaluated at call-time.
    pub default_value: Option<Box<AstNode>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtensionReceiver {
    pub ty: TypeExpr,
    pub method_name: String,
}

/// Generic parameter in declarations: `T` or `T = SomeType` (default type argument).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GenericParam {
    pub name: String,
    pub name_span: Span,
    pub default: Option<TypeExpr>,
}

impl GenericParam {
    pub fn names(params: &[GenericParam]) -> Vec<String> {
        params.iter().map(|p| p.name.clone()).collect()
    }
}

/// Type syntax: `Int`, `()`, `(Int, String)`, `[Int]`, nested combinations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TypeExpr {
    Named(String),
    /// Generic application used in types: `Option<Int>`, `Result<String, Int>`.
    EnumApp {
        name: String,
        args: Vec<TypeExpr>,
    },
    /// Underscore placeholder in type argument lists: `_`
    Infer,
    Unit,
    Tuple(Vec<TypeExpr>),
    Array(Box<TypeExpr>),
    /// First-class function type: `(A, b: B = ...) => R`.
    Function {
        params: Vec<FunctionTypeParam>,
        ret: Box<TypeExpr>,
    },
    /// Type parameter in a type position (e.g. `[type T]` in extension receivers).
    TypeParam(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FunctionTypeParam {
    pub name: Option<String>,
    pub ty: TypeExpr,
    /// True when the parameter has a default value in the type annotation.
    pub has_default: bool,
}

/// Irrefutable pattern: `_`, `x`, `(a, b, ..)`, nested.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Pattern {
    Wildcard { span: Span },
    Binding {
        name: String,
        name_span: Span,
    },
    /// Literal pattern, e.g. `x: 1` inside a struct pattern.
    IntLiteral {
        value: u64,
        original: String,
        radix: u32,
        span: Span,
    },
    StringLiteral { value: String, span: Span },
    BoolLiteral { value: bool, span: Span },
    Tuple {
        elements: Vec<PatternElem>,
        span: Span,
    },
    Array {
        elements: Vec<PatternElem>,
        span: Span,
    },
    /// `Name { x, y: z, .. }` — Rust-like struct destructuring.
    Struct {
        name: String,
        name_span: Span,
        /// Optional generic type arguments in struct patterns: `Name<T> { ... }` or `Name<T>`.
        type_args: Vec<TypeExpr>,
        fields: Vec<StructPatternField>,
        rest: Option<Span>,
        span: Span,
    },
    /// `EnumName::Variant(p1, p2, ...)` — enum variant destructuring.
    ///
    /// - Use an empty `payloads` list for zero-payload variants: `EnumName::None`.
    /// - Payload patterns may be bindings, `_`, or nested destructuring patterns.
    EnumVariant {
        enum_name: String,
        enum_name_span: Span,
        /// Optional type arguments in patterns: `EnumName<T>::Variant(...)`.
        ///
        /// - Empty means "infer from scrutinee context".
        /// - Supports `_` via `TypeExpr::Infer`.
        type_args: Vec<TypeExpr>,
        variant: String,
        payloads: Vec<Pattern>,
        span: Span,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PatternElem {
    Pattern(Pattern),
    Rest(Span),
}

/// Field declaration in a `struct Name { field: Type, ... }`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StructFieldDecl {
    pub name: String,
    pub name_span: Span,
    pub ty: TypeExpr,
    pub ty_span: Span,
}

/// One field entry in a struct literal or destructuring pattern.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StructPatternField {
    pub name: String,
    pub name_span: Span,
    pub pattern: Pattern,
}

#[derive(Debug, Clone, PartialEq)]
pub enum CallArg {
    Positional(AstNode),
    Named {
        name: String,
        name_span: Span,
        value: AstNode,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnumVariantDecl {
    pub name: String,
    pub name_span: Span,
    pub payload_types: Vec<TypeExpr>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinaryOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    BitAnd,
    BitXor,
    BitOr,
    ShiftLeft,
    ShiftRight,
    Eq,
    Ne,
    Lt,
    Gt,
    Le,
    Ge,
    And,
    Or,
}

/// Compound assignment operator (`+=`, …); `Int` only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompoundOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    BitAnd,
    BitXor,
    BitOr,
    ShiftLeft,
    ShiftRight,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOp {
    Plus,
    Minus,
    BitNot,
    /// Logical not (`!`), `Bool` only
    Not,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ImportBinding {
    pub export_name: String,
    pub local_name: String,
    pub local_span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum AstNode {
    /// `import { A, B as C } from "path";` — each pair is (exported name from module, local name).
    Import {
        bindings: Vec<ImportBinding>,
        module_path: String,
        span: Span,
    },
    /// `export from_name as to_name;` — re-export `from_name` under public name `to_name`.
    ExportAlias {
        from: String,
        to: String,
        span: Span,
    },
    /// `export Name;` — export a previously declared symbol by its local name.
    ExportName {
        name: String,
        name_span: Span,
        span: Span,
    },
    SingleLineComment(String),
    MultiLineComment(String),
    IntegerLiteral {
        value: u64,
        original: String,
        radix: u32,
        span: Span,
    },
    FloatLiteral {
        /// Original lexeme, including underscores.
        original: String,
        /// Cleansed lexeme for numeric parsing (underscores removed).
        cleaned: String,
        span: Span,
    },
    StringLiteral {
        value: String,
        original: String,
        span: Span,
    },
    BoolLiteral {
        value: bool,
        span: Span,
    },
    /// Local or parameter reference: `a`, `b`
    Identifier {
        name: String,
        span: Span,
    },
    /// `()` unit value
    UnitLiteral {
        span: Span,
    },
    /// Tuple literal `(a, b, …)` or `(x,)`
    TupleLiteral {
        elements: Vec<AstNode>,
        span: Span,
    },
    /// Array literal `[a, b, ...]` (length is dynamic at runtime).
    ArrayLiteral {
        elements: Vec<AstNode>,
        span: Span,
    },
    /// Dict/map literal: `{ key: value, key2: value2, ... }`.
    DictLiteral {
        entries: Vec<(AstNode, AstNode)>,
        span: Span,
    },
    /// `expr.0`, `expr.0.1` — one segment per node (left-associative chain in parser).
    TupleField {
        base: Box<AstNode>,
        index: u32,
        span: Span,
    },
    /// `expr[idx]` — one segment per node (left-associative chain in parser).
    ArrayIndex {
        base: Box<AstNode>,
        index: Box<AstNode>,
        span: Span,
    },
    /// Lambda expression: `x => x + 1`, `(x, y) => { ... }`, `() => 1`.
    Lambda {
        params: Vec<LambdaParam>,
        body: Box<LambdaBody>,
        span: Span,
    },
    BinaryOp {
        left: Box<AstNode>,
        op: BinaryOp,
        right: Box<AstNode>,
        span: Span,
    },
    UnaryOp {
        op: UnaryOp,
        operand: Box<AstNode>,
        span: Span,
    },
    /// `internal func name(args);` or `internal async func name(args): Type;`
    InternalFunction {
        name: String,
        type_params: Vec<GenericParam>,
        params: Vec<Param>,
        return_type: Option<TypeExpr>,
        name_span: Span,
        is_exported: bool,
        is_async: bool,
    },
    /// `func name(args) { body }` or `async func name(args): Type { body }`
    Function {
        name: String,
        extension_receiver: Option<ExtensionReceiver>,
        type_params: Vec<GenericParam>,
        params: Vec<Param>,
        return_type: Option<TypeExpr>,
        body: Vec<AstNode>,
        name_span: Span,
        closing_span: Span,
        is_exported: bool,
        is_async: bool,
    },
    /// Expression or statement call: `callee(args)` / `callee(args);`
    Call {
        callee: String,
        type_args: Vec<TypeExpr>,
        arguments: Vec<CallArg>,
        span: Span,
    },
    /// Call where callee itself is an expression: `f()(x)`, `(x => x)(1)`.
    Invoke {
        callee: Box<AstNode>,
        arguments: Vec<CallArg>,
        span: Span,
    },
    /// `value.method(args...)`
    MethodCall {
        receiver: Box<AstNode>,
        method: String,
        arguments: Vec<CallArg>,
        span: Span,
    },
    /// `Type::method(args...)`
    TypeMethodCall {
        type_name: String,
        method: String,
        arguments: Vec<CallArg>,
        span: Span,
    },
    /// Type-as-value expression for unit structs (for example `None` or `None<Int>`).
    TypeValue {
        type_name: String,
        span: Span,
    },
    /// `await expr` — only valid inside `async` functions.
    Await {
        expr: Box<AstNode>,
        span: Span,
    },
    /// `struct Name { field: Type, ... }` or `internal struct Name<T = U>;` (top-level item)
    StructDef {
        name: String,
        type_params: Vec<GenericParam>,
        fields: Vec<StructFieldDecl>,
        /// `true` when declared as a unit struct (`struct Name;`).
        is_unit: bool,
        /// Host-defined nominal type (e.g. `Task`); not a normal `Struct` runtime value.
        is_internal: bool,
        name_span: Span,
        span: Span,
        is_exported: bool,
    },
    /// `Name { x: expr, y: expr, ..base? }` (expression)
    StructLiteral {
        name: String,
        type_args: Vec<TypeExpr>,
        fields: Vec<(String, AstNode)>,
        update: Option<Box<AstNode>>,
        span: Span,
    },
    /// `base.field` (expression or lvalue)
    FieldAccess {
        base: Box<AstNode>,
        field: String,
        span: Span,
    },
    /// `enum Name<T, ...> { Variant, Variant(T), ... }` (top-level item)
    EnumDef {
        name: String,
        type_params: Vec<GenericParam>,
        variants: Vec<EnumVariantDecl>,
        /// Host-defined/module-private enum declaration.
        is_internal: bool,
        name_span: Span,
        span: Span,
        is_exported: bool,
    },
    /// `type Name<T, ...> = SomeTypeExpr;` (top-level item)
    TypeAlias {
        name: String,
        type_params: Vec<GenericParam>,
        target: TypeExpr,
        name_span: Span,
        span: Span,
        is_exported: bool,
    },
    /// `EnumTypeExpr::Variant(payloads?)` expression, where `EnumTypeExpr` may include
    /// explicit type arguments like `Option<Int>::None` or `Result<_, String>::Ok(true)`.
    EnumVariantCtor {
        enum_name: String,
        type_args: Vec<TypeExpr>,
        variant: String,
        payloads: Vec<AstNode>,
        span: Span,
    },
    /// `return;` or `return expr;`
    Return {
        value: Option<Box<AstNode>>,
        span: Span,
    },
    /// `let pattern: Type? = expr?;` — top level requires initializer; tuple patterns only inside functions.
    Let {
        pattern: Pattern,
        type_annotation: Option<TypeExpr>,
        initializer: Option<Box<AstNode>>,
        /// `true` for `const` declarations.
        is_const: bool,
        /// `true` for `export const ...;` declarations.
        is_exported: bool,
        span: Span,
    },
    /// `pattern = expr;` or `(a, b) = t;` — irrefutable pattern on the left.
    Assign {
        pattern: Pattern,
        value: Box<AstNode>,
        span: Span,
    },
    /// `lhs = rhs;` when `lhs` is an lvalue expression (`t.0`, …). Semantics may reject (e.g. tuple fields).
    AssignExpr {
        lhs: Box<AstNode>,
        rhs: Box<AstNode>,
        span: Span,
    },
    /// `lhs += rhs;` etc. — `lhs` is parsed as an lvalue expression; semantics allow `Identifier` only.
    CompoundAssign {
        lhs: Box<AstNode>,
        op: CompoundOp,
        rhs: Box<AstNode>,
        span: Span,
    },
    Block {
        body: Vec<AstNode>,
        closing_span: Span,
    },
    /// `if cond { stmts }` or `if cond { stmts } else { stmts }`
    If {
        condition: Box<AstNode>,
        then_body: Vec<AstNode>,
        else_body: Option<Vec<AstNode>>,
        span: Span,
    },
    /// `if let <pattern> = <expr> { stmts } else { stmts }`
    ///
    /// Pattern-bound names are only visible in the `then` branch.
    IfLet {
        pattern: Pattern,
        value: Box<AstNode>,
        then_body: Vec<AstNode>,
        else_body: Option<Vec<AstNode>>,
        span: Span,
    },
    /// `while cond { body }` — condition must be `Bool`.
    While {
        condition: Box<AstNode>,
        body: Vec<AstNode>,
        span: Span,
    },
    /// `match <scrutinee> { <arms> }` — Rust-like match expression.
    Match {
        scrutinee: Box<AstNode>,
        arms: Vec<MatchArm>,
        span: Span,
    },
    /// `break;` — innermost loop only.
    Break {
        span: Span,
    },
    /// `continue;` — innermost loop only.
    Continue {
        span: Span,
    },
    Program(Vec<AstNode>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct LambdaParam {
    pub name: String,
    pub name_span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum LambdaBody {
    Expr(AstNode),
    Block(Vec<AstNode>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct MatchArm {
    /// `pat1 | pat2 | ...`
    pub patterns: Vec<Pattern>,
    pub guard: Option<Box<AstNode>>,
    pub body: Box<AstNode>,
    pub span: Span,
}
