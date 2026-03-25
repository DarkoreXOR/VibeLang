# Comparisons and Equality

## What it is

Comparison operators return `Bool` and are limited by operand type.

## Syntax

```vc
let a = 10 < 20;
let b = 5 >= 5;
let c = "x" == "x";
let d = (1, 2) == (1, 2);
let e = [1, 2] != [2, 1];
```

## Rules and constraints

- Ordering (`< > <= >=`) works on `Int` and `Float` (same-type operands).
- Equality (`== !=`) works on:
  - `Int`, `Float`, `String`, `Bool`, `()`
  - tuples and arrays of same shape/type
- String ordering (like `"a" < "b"`) is not implemented.

## Valid examples

```vc
func main() {
    let ok1 = 100 > 50;
    let ok2 = [1, 2, 3] == [1, 2, 3];
    let ok3 = (1, (2, 3)) != (1, (2, 4));
}
```

## Common errors

```vc
func main() {
    let bad = "a" < "b"; // Expected compile-time error: ordering on String is not supported
}
```

## More examples

```vc
func main() {
    let t1 = (1, 2, 3);
    let t2 = (1, 2, 3);
    let same = t1 == t2;
}
```

## See also

- [Operators](./03-operators.md)
- [Control Flow](./05-control-flow.md)
