# Types

## What it is

Vibelang is strongly typed. Values keep their declared/inferred type, and there are no implicit conversions.

## Syntax

```vc
let i: Int = 42;
let s: String = "hello";
let b: Bool = true;
let u: () = ();
let pair: (Int, String) = (1, "x");
let one: (Int,) = (7,);
let xs: [Int] = [1, 2, 3];
let anyv: Any = "text";
type UserId = Int;
type Res<T> = Result<T, String>;
```

## Rules and constraints

- `Int` is arbitrary precision (`BigInt`) at runtime.
- Literal types:
  - integer literal -> `Int`
  - string literal -> `String`
  - `true`/`false` -> `Bool`
- Arrays use `[T]`.
- Unit is `()`.
- `Any` can hold any value, but operators are not allowed on `Any`.
- No implicit casts (for example, `Int` is not auto-converted to `String`).
- `Any` is explicit: inference of locals/lambdas/call chains does not default unknowns to `Any`.
- Type aliases are compile-time only and may be generic:
  - `type UserId = Int;`
  - `type Res<T> = Result<T, String>;`
- Generic parameters may have defaults (Rust-like trailing defaults), for example `internal struct Task<T = ()>;` so bare `Task` means `Task<()>` in type positions.
- Internal nominal types: `internal struct Name<T = U, ...>;` declares a host-registered type (not a normal `Struct` value at runtime). The only such type in v1 is `Task`.

## Valid examples

```vc
func main() {
    let a = 999999999999999999999999999999999999;
    let b: [String] = ["a", "b"];
    let c: Any = b;
}
```

## Common errors

```vc
func main() {
    let x: Int = "10"; // Expected compile-time error: type mismatch
}
```

## More examples

```vc
func main() {
    let tuplev: (Int, Bool, String) = (1, true, "ok");
    let nested: [[Int]] = [[1, 2], [3, 4]];
    let anyv: Any = tuplev;
}
```

## See also

- [Variables and Assignment](./02-variables-and-assignment.md)
- [Type Inference](./12-type-inference.md)
