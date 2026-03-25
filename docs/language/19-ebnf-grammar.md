# EBNF Grammar (v1)

## Notes

- This grammar describes the concrete syntax accepted by the Vibelang parser.
- Terminals are written as quoted strings (keywords/operators) or as token names (e.g. `Identifier`, `IntegerLiteral`, `FloatLiteral`, `StringLiteral`).
- Operator precedence is encoded via separate expression nonterminals (`OrExpr`, `AndExpr`, …).

## Lexical tokens

### Identifiers

- `Identifier` — a non-empty sequence matching the lexer’s identifier rules (e.g. `main`, `print_gen`, `Float`, `x`, `self`).

### Literals

- `IntegerLiteral` — integer digits with optional underscores, optionally with radix prefix (`0b`, `0o`, `0x`).
- `FloatLiteral` — a decimal float with:
  - a mantissa containing digits and optional underscores
  - an optional fractional part (`.` + digits)
  - an optional exponent part (`e` or `E` + optional sign + digits), with underscores allowed in mantissa/exponent
- `StringLiteral` — double-quoted string with escapes (as supported by the lexer).
- `True` / `False` — boolean literals.

## Source grammar

```ebnf
Program              = { TopLevelItem } ;

TopLevelItem         = ImportDecl
                      | ExportItem
                      | LetStmtTopLevel
                      | StructDef
                      | EnumDef
                      | TypeAliasDef
                      | InternalDecl
                      | FuncDef
                      | AsyncFuncDef
                      | CallStmtTopLevel
                      | Comment ;

Comment               = SingleLineComment | MultiLineComment ;  (* ignored by the parser *)

(* -------- Modules -------- *)
ImportDecl            = "import" "{" ImportBindingList "}" "from" StringLiteral ";" ;
ImportBindingList    = Identifier { "," Identifier } ;
CallStmtTopLevel     = Identifier GenericArgsOpt "(" CallArgListOpt ")" ";" ;

ExportItem            = "export" ( ExportDecl | ExportAlias ) ;
ExportAlias           = Identifier "as" Identifier ";" ;

ExportDecl            = StructDef
                      | EnumDef
                      | TypeAliasDef
                      | FuncDef
                      | AsyncFuncDef
                      | InternalDecl ;

(* -------- Globals -------- *)
LetStmtTopLevel       = "let" Pattern TypeAnnOpt InitAnnOpt ";" ;
TypeAnnOpt            = [ ":" TypeExpr ] ;
InitAnnOpt            = [ "=" Expr ] ;  (* required by the parser for top-level lets *)

(* -------- Types: structs / enums / aliases -------- *)
StructDef             = "struct" Identifier TypeParamsOpt StructBodyOrUnit ;
StructBodyOrUnit      = ( ";" )                      (* unit struct *) 
                      | ( "{" [ StructField { "," StructField } ] "}" ) ;
StructField           = Identifier ":" TypeExpr ;

EnumDef               = "enum" Identifier TypeParamsOpt "{" EnumVariant { "," EnumVariant } [ "," ] "}" ;
EnumVariant           = Identifier [ "(" TypeExpr { "," TypeExpr } ")" ] ;

TypeAliasDef          = "type" Identifier TypeParamsOpt "=" TypeExpr ";" ;

(* -------- internal declarations -------- *)
InternalDecl          = "internal" ( InternalStructDef | InternalFuncDecl | InternalAsyncFuncDecl ) ;
InternalStructDef     = "struct" Identifier TypeParamsOpt ";" ;

InternalFuncDecl      = "func" FuncNameWithExtReceiver TypeParamsOpt "(" ParameterList ")" ReturnTypeOpt ";" ;
InternalAsyncFuncDecl = "async" "func" FuncNameWithExtReceiver TypeParamsOpt "(" ParameterList ")" ReturnTypeOpt ";" ;

(* -------- Functions / async functions -------- *)
FuncDef               = "func" FuncNameWithExtReceiver TypeParamsOpt "(" ParameterList ")" ReturnTypeOpt FuncBody ;
AsyncFuncDef          = "async" "func" FuncNameWithExtReceiver TypeParamsOpt "(" ParameterList ")" ReturnTypeOpt FuncBody ;
FuncNameWithExtReceiver
                      = Identifier
                      | TypeExpr "::" Identifier ;  (* extension method syntax *)

TypeParamsOpt         = [ "<" TypeParam { "," TypeParam } ">" ] ;
TypeParam             = Identifier [ "=" TypeExpr ] ;

FuncBody              = "{" { BlockItem } "}"
                      | "=" Expr ";"  ;  (* shorthand *)

ReturnTypeOpt         = [ ":" TypeExpr ] ;

(* -------- let / if / while / match / return -------- *)
BlockItem             = LetStmtInBlock
                      | IfStmt
                      | WhileStmt
                      | MatchExprStmt
                      | "break" ";"
                      | "continue" ";"
                      | ReturnStmt
                      | BlockStmt
                      | AwaitExprStmt
                      | AssignStmt ;

LetStmtInBlock       = "let" Pattern TypeAnnOpt InitAnnOpt ";" ;

IfStmt               = "if" ( "let" Pattern "=" Expr BlockBodyElseOpt
                             | Expr BlockBodyElseOpt ) ;
BlockBodyElseOpt     = "{" { BlockItem } "}" [ "else" ( "if" IfStmt | "{" { BlockItem } "}" ) ] ;

WhileStmt            = "while" Expr "{" { BlockItem } "}" ;

MatchExprStmt        = "match" Expr "{" MatchArm { "," MatchArm } [ "," ] "}" [";"] ;
MatchArm             = PatternOr [ "if" Expr ] "=>" MatchBody ;
PatternOr            = Pattern { "|" Pattern } ;
MatchBody            = BlockStmt | Expr ;

BlockStmt            = "{" { BlockItem } "}" ;

ReturnStmt           = "return" [ Expr ] ";" ;

AwaitExprStmt        = "await" Expr ";" ;

(* -------- Assignment and call statements -------- *)
AssignStmt           = "(" Pattern ")" "=" Expr ";"            (* pattern assignment *)
                      | AssignmentTarget ( CompoundOp Expr ";" | "=" Expr ";" ) ;

AssignmentTarget     = Expr ;  (* restricted in the implementation to postfix forms starting from identifiers *)
CompoundOp           = "+=" | "-=" | "*=" | "/=" | "%="
                      | "&=" | "|=" | "^="
                      | "<<=" | ">>=" ;

(* -------- Patterns -------- *)
Pattern              = "_"                                (* wildcard *)
                      | Identifier                           (* binding *)
                      | IntegerLiteral
                      | StringLiteral
                      | "true" | "false"
                      | TuplePattern
                      | ArrayPattern
                      | StructPattern
                      | EnumVariantPattern ;

TuplePattern          = "(" [ TuplePatternElem { "," TuplePatternElem } [ "," ] ] ")" ;
TuplePatternElem      = Pattern | "..." ;
ArrayPattern          = "[" [ ArrayPatternElem { "," ArrayPatternElem } [ "," ] ] "]" ;
ArrayPatternElem      = Pattern | "..." ;

StructPattern         = Identifier [ TypeArgsOpt ] "{" [ StructPatItem { "," StructPatItem } [ "," ] ] "}"
                      | Identifier TypeArgsReq ;  (* typed unit-struct pattern: `Name<T>` *)
StructPatItem        = StructPatField | ".." ;
StructPatField       = Identifier [ ":" ( "_" | Pattern ) ] ;  (* shorthand `field` means `field: field` *)

EnumVariantPattern   = Identifier [ TypeArgsOpt ] "::" Identifier [ "(" [ Pattern { "," Pattern } ] ")" ] ;

TypeArgsReq         = "<" TypeExpr { "," TypeExpr } ">" ;
TypeArgsOpt         = [ TypeArgsReq ] ;

(* -------- Expressions with precedence -------- *)
Expr                 = LambdaExpr | OrExpr ;

(* Lambda shorthand *)
LambdaExpr          = Identifier "=>" LambdaBody
                    | "(" LambdaParamList ")" "=>" LambdaBody ;
LambdaParamList    = Identifier { "," Identifier } ;
LambdaBody         = BlockStmt | Expr ;

OrExpr              = AndExpr { "||" AndExpr } ;
AndExpr             = BitOrExpr { "&&" BitOrExpr } ;
BitOrExpr           = BitXorExpr { "|" BitXorExpr } ;
BitXorExpr          = BitAndExpr { "^" BitAndExpr } ;
BitAndExpr          = EqualityExpr { "&" EqualityExpr } ;
EqualityExpr       = RelExpr { ( "==" | "!=" ) RelExpr } ;
RelExpr             = ShiftExpr { ( "<" | ">" | "<=" | ">=" ) ShiftExpr } ;
ShiftExpr           = AddExpr { ( "<<" | ">>" ) AddExpr } ;
AddExpr             = MulExpr { ( "+" | "-" ) MulExpr } ;
MulExpr             = UnaryExpr { ( "*" | "/" | "%" ) UnaryExpr } ;

UnaryExpr           = ( "!" | "~" | "+" | "-" ) UnaryExpr
                    | "await" UnaryExpr
                    | PostfixExpr ;

PostfixExpr         = PrimaryExpr PostfixSuffix* ;
PostfixSuffix       = "." ( IntegerLiteral | Identifier )
                      | "[" Expr "]"
                      | "(" CallArgListOpt ")" ;

PrimaryExpr         = "match" Expr "{" MatchArm { "," MatchArm } [ "," ] "}"
                      | IntegerLiteral
                      | FloatLiteral
                      | StringLiteral
                      | "true" | "false"
                      | Identifier GenericCallOrValueTailOpt
                      | "(" ")"                          (* unit literal *)
                      | "(" Expr ")"                     (* grouping *)
                      | "(" Expr "," [ Expr { "," Expr } ] [ "," ] ")"  (* tuple literal, 1+ elements *)
                      | "[" "]"                          (* empty array literal *)
                      | "[" Expr { "," Expr } [ "," ] "]" (* array literal *)
                      | StructLiteral
                      | DictLiteral
                      | UnitStructTypeValue ;

(* Calls, enum constructors, struct literals, unit-struct type values.
   Implemented as tails after `Identifier` (including optional `<...>`). *)
GenericCallOrValueTailOpt
                      = /* empty */
                      | TypeArgsOpt "::" Identifier "(" [ ExprList ] ")"
                      | TypeArgsOpt "(" CallArgListOpt ")"  (* normal generic call *)
                      | TypeArgsOpt "{" StructFieldInitListOpt "}"  (* generic struct literal *)
                      | TypeArgsOpt  ;  (* type-value expression *)

StructLiteral        = Identifier [ TypeArgsOpt ] "{" StructFieldInitListOpt [ "," ".." Expr ] "}" ;
StructFieldInitListOpt = [ StructFieldInit { "," StructFieldInit } [ "," ] ] ;
StructFieldInit      = Identifier ":" Expr ;

(* Dict literal: `{ key: value, ... }` *)
DictLiteral          = "{" [ DictEntry { "," DictEntry } [ "," ] ] "}" ;
DictEntry            = Expr ":" Expr ;

UnitStructTypeValue = Identifier TypeArgsOpt ;  (* parsed as a type-value expression *)

CallArgListOpt       = [ CallArgList ] ;
CallArgList          = CallArg { "," CallArg } [ "," ] ;
CallArg              = Expr | NamedArg ;
NamedArg             = Identifier ":" Expr ;

ExprList             = Expr { "," Expr } ;
GenericArgsOpt       = [ "<" TypeExpr { "," TypeExpr } ">" ] ;
```

## Coverage / limitations

- The parser has a few context-sensitive restrictions (e.g. what can start a block item, top-level `let` requiring an initializer, and some assignment target restrictions). Those are documented as prose in other pages (for example, `18-async.md` and `17-implementation-notes.md`) rather than fully expressed in EBNF.

