# Type Inference

## What it is

The compiler infers types from initializers and contextual type information.

## Syntax

```vc
let x = 10;          // Int
let y = "hello";     // String
let z: [Int] = [];   // contextual typing
```

## Rules and constraints

- If `let` has no annotation, type comes from initializer.
- Float literals (e.g. `1.5`, `1e-3`, `12_34.5`) infer `Float`.
- Local bindings may defer inference to first assignment:
  - `let a; a = foo();` infers `a` from `foo()` result type.
- Inference is constraint-based across usage chains ("breadcrumbs"):
  - one concrete usage can resolve an entire connected chain of unknowns.
  - example: `r = f(123); next(r);` resolves both `r` and `f` to `Int`-compatible shapes.
- Empty array literal `[]` requires contextual type.
- Assignment to an annotated variable must match annotation.
- Types are not inferred from future reads.
- Generic struct literals can infer type args from field initializers:
  - `Generic { a: 7, b: 8 }` -> `Generic<Int>`
- Explicit generic struct literals are also supported:
  - `Generic<Int> { a: 7, b: 8 }`
- Generic unit structs use type-value expressions with explicit args:
  - `UnitGeneric<Int>`
- Generic inference is resolved at compile time; emitted bytecode uses concrete instantiations only.
- Inference does not silently fall back to `Any` for unknown locals/lambdas/call chains.
- Local function/lambda bindings remain monomorphic after first constraining use.
- Alias type arguments also participate in inference:
  - `type Res<T> = Result<T, String>;`
  - `func get(): Res<_> = Res::Ok(5);` resolves to `Result<Int, String>`.

## Valid examples

```vc
func main() {
    let a = 1;
    let x;
    x = a + 2;      // `x` inferred as Int from first assignment
    let b: [String] = [];
    let c: Int = x;
}
```

## Common errors

```vc
func main() {
    let xs = []; // Expected compile-time error: no context for element type
}
```

```vc
func main() {
    let z = x => x;
    let _ = z(1);
    let _ = z("x"); // Expected compile-time error: conflicting inferred types
}
```

## More examples

```vc
func main() {
    let n = 10;
    let words: [String] = [];
    let pair = (n, "ok");
}
```

## See also

- [Types](./01-types.md)
- [Definite Assignment](./13-definite-assignment.md)
