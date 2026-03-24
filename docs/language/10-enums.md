# Enums

## What it is

Enums define tagged variants, optionally with payloads, including generic enums.

## Syntax

```vc
enum Option<T> {
    None,
    Some(T),
}
```

Constructors:

```vc
let a = Option::None;
let b = Option::Some(10);
let c = Option<Int>::Some(20);
```

## Rules and constraints

- Enum declarations are top-level.
- Variants may be unit-like or payload-bearing.
- Trailing comma in variant list is optional.
- Generic type args may be explicit or inferred.
- `_` placeholders in generic args are allowed in supported contexts.
- Enum constructor paths also work through type aliases:
  - `type Res<T> = Result<T, String>;`
  - `Res::Ok(5)` / `Res<_>::Err("bad")`
- For generic extension receiver declarations, enum type parameters use `type`:
  - `func Result<type T, type E>::method<T, E>(self): ...`
  - `func [Result<type T, type E>]::method<T, E>(self): ...`
- Operators are not defined directly on enum values.

## Valid examples

```vc
enum Result<T, E> { Ok(T), Err(E) }

func main() {
    let r1 = Result::Ok("done");
    let r2 = Result<String, Int>::Err(404);
}
```

## Common errors

```vc
enum Option<T> { None, Some(T) }
func main() {
    let x = Option::Some(); // Expected compile-time error: missing payload
}
```

## More examples

```vc
enum Result<T, E> { Ok(T), Err(E) }

func main() {
    let a = Result::Ok(1);
    let b = Result<Int, String>::Err("bad");
}
```

## See also

- [Patterns](./11-patterns.md)
- [Modules](./14-modules.md)
