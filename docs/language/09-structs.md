# Structs

## What it is

Structs are nominal user-defined types. They can be record structs (`struct S { ... }`) or unit structs (`struct S;`), and both forms support generics.

## Syntax

```vc
struct Point {
    x: Int,
    y: Int,
}
struct UnitGeneric<T>;
struct Pair<T> { a: T, b: T }
```

Struct literals and updates:

```vc
struct Void {}
let p1 = Point { x: 1, y: 2 };
let p2 = Point { x: 10, ..p1 };
let z1 = Void{};
let z2 = Void {};
let ug = UnitGeneric<Int>;
let p3 = Pair { a: 1, b: 2 };          // inferred as Pair<Int>
let p4 = Pair<Int> { a: 3, b: 4 };     // explicit type args
```

## Rules and constraints

- Declared at top level.
- Field separator is `,`; trailing comma is optional.
- Struct assignment is alias/reference style (`p2 = p1` aliases same instance).
- Field access: `p.x`, chained: `p.a.b`.
- Field assignment is supported.
- Struct update `..base` must be last and same struct type.
- Zero-field struct literals are valid in both forms: `Void{}` and `Void {}`.
- Zero-field struct literals can be used directly as method-call receivers: `Void{}.method(...)`.
- Unit structs are declared with `;` and used as values by type name:
  - `struct None;`
  - value: `None`
- Generic structs:
  - declaration: `struct Box<T> { value: T }`, `struct Token<T>;`
  - record literals support both inferred and explicit type args:
    - `Box { value: 1 }` (inferred)
    - `Box<Int> { value: 1 }` (explicit)
  - generic unit structs use a type value form:
    - `Token<Int>`
- Generics are compile-time only:
  - concrete instantiations are treated as distinct nominal types (`Box<Int>` and `Box<Bool>`).
  - VM/runtime operate on concrete instantiated type names and do not perform generic matching.
- Operators are not defined for struct values directly.

## Valid examples

```vc
struct User { name: String, age: Int }

func main() {
    let u = User { name: "A", age: 20 };
    u.age = 21;
    let older = User { age: 30, ..u };
}
```

## Common errors

```vc
struct User { name: String, age: Int }
func main() {
    let u = User { name: "A" }; // Expected compile-time error: missing field age
}
```

## More examples

```vc
struct Point { x: Int, y: Int }

func main() {
    let p1 = Point { x: 1, y: 2 };
    let p2 = Point { ..p1 };
    p2.x = 10;
}
```

## See also

- [Patterns](./11-patterns.md)
- [Enums](./10-enums.md)
