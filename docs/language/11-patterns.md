# Patterns

## What it is

Patterns are used in `let`, assignment, `if let`, and `match`.

## Syntax

```vc
let (a, b) = (1, 2);
let [x, .., z] = [1, 2, 3, 4];
let Point { x, y, .. } = p;
```

`match` patterns:

```vc
match v {
    Option::Some(x) => x,
    Option::None => 0,
}
```

## Rules and constraints

- Supported forms:
  - wildcard: `_`
  - binding: `name`
  - literals: int/string/bool literals
  - tuple and array patterns with at most one `..`
  - struct patterns
  - enum variant patterns
  - alternatives with `|`
  - match arm guards with `if`
- `match` is exhaustiveness-checked.
- `if let` supports nested patterns for enums/structs.
- Generic-instantiated pattern types are strict at compile time:
  - mismatched concrete generic instances are type errors (for example `UnitGeneric<String>` on `UnitGeneric<Int>`).

## Valid examples

```vc
enum Option<T> { None, Some(T) }

func main() {
    let v = Option::Some((1, 2));
    if let Option::Some((a, b)) = v {
        let sum = a + b;
    } else {}
}
```

## Common errors

```vc
enum Option<T> { None, Some(T) }
func main() {
    struct UnitGeneric<T>;
    let ug = UnitGeneric<Int>;
    if let UnitGeneric<String> = ug { } else { }
    // Expected compile-time error: struct pattern type mismatch (`UnitGeneric<Int>` vs `UnitGeneric<String>`)
}
```

## More examples

```vc
enum Option<T> { None, Some(T) }
struct Pair { a: Int, b: Int }

func main() {
    let x = Option::Some(Pair { a: 1, b: 2 });
    match x {
        Option::Some(Pair { a, .. }) => a,
        Option::None => 0,
    };
}
```

## See also

- [Control Flow](./05-control-flow.md)
- [Structs](./09-structs.md)
