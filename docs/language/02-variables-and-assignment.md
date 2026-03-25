# Variables and Assignment

## What it is

`let` creates bindings. You can assign and reassign values as long as types remain valid.

## Syntax

```vc
let x: Int = 1;
let y = 2;
x = 3;
```

Pattern assignment is supported:

```vc
let (a, b) = (10, 20);
let [h, t] = [1, 2];
```

## Rules and constraints

- Core declaration form: `let pattern: Type? = expr?;`
- Inside functions, type annotation and initializer are optional, but:
  - if initializer is omitted for a simple binding (`let a;`), type is inferred from the first assignment (`a = expr;`).
  - non-binding patterns without initializer still require an explicit type.
- `=` is assignment; `==` is equality.
- Tuple field assignment (like `t.0 = 1`) is rejected semantically.
- Array element assignment is supported: `a[i] = v;`
- Struct field assignment is supported: `p.x = 1;`
- Compound assignment is supported for `Int` and `Float` variable names (arithmetic ops):
  - `+= -= *= /= %=` (Int/Float)
  - `&= |= ^= <<= >>= ~` are `Int`-only (bitwise ops)

## Valid examples

```vc
struct Point { x: Int, y: Int }

func main() {
    let a;
    a = 10; // `a` inferred as Int from first assignment

    let arr: [Int] = [1, 2, 3];
    arr[1] = 99;

    let p = Point { x: 1, y: 2 };
    p.x = 10;

    let n = 5;
    n += 3;
}
```

## Common errors

```vc
func main() {
    let t = (1, 2);
    t.0 = 9; // Expected compile-time error: unsupported assignment target

    let a;
    let b = a; // Expected compile-time error: `a` may be uninitialized
}
```

## More examples

```vc
func main() {
    let (x, y) = (3, 4);
    let arr = [10, 20, 30];
    arr[0] = x + y;
}
```

## See also

- [Types](./01-types.md)
- [Definite Assignment](./13-definite-assignment.md)
