# Control Flow

## What it is

Control flow includes `if`, `while`, `break`, `continue`, and `match`.

## Syntax

```vc
if cond { ... } else { ... }
while cond { ... }
break;
continue;
match expr { pat => expr, _ => expr2 }
```

## Rules and constraints

- `if` and `while` conditions must be `Bool`.
- `while` condition can be any boolean expression, including function calls.
- `break`/`continue` are only valid inside `while`.
- `match` is an expression.
- `match` arms support:
  - `pattern => expr_or_block`
  - pattern alternatives with `|`
  - optional guards (`if ...`)
  - optional trailing comma on final arm
- `match` exhaustiveness is checked at compile time.

## Valid examples

```vc
internal func int_array_len(a: [Int]): Int;

func main() {
    let xs = [1, 2, 3];
    let i = 0;
    while i < int_array_len(xs) {
        if xs[i] == 2 {
            i += 1;
            continue;
        }
        i += 1;
    }

    let v = match i {
        0 => "zero",
        _ => "non-zero",
    };
}
```

## Common errors

```vc
func main() {
    if 1 { } // Expected compile-time error: condition is not Bool
}
```

## More examples

```vc
enum Option<T> { None, Some(T) }

func main() {
    let v = Option::Some(10);
    let out = match v {
        Option::Some(x) if x > 5 => "big",
        Option::Some(_) => "small",
        Option::None => "none",
    };
}
```

## See also

- [Patterns](./11-patterns.md)
- [Functions](./06-functions.md)
