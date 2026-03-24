# Operators

## What it is

Vibelang supports arithmetic, bitwise, shift, logical, and unary operators with strict type rules.

## Syntax

```vc
let a = 1 + 2 * 3;
let b = 10 - 4;
let c = 20 / 3;
let d = 20 % -3; // divisor-sign remainder
let e = ~1;
let f = 1 << 5;
let g = 6 & 3;
let h = true && !false;
let s = "a" + "b";
```

## Rules and constraints

- `+` supports:
  - `Int + Int`
  - `String + String`
- `- * / %` are `Int`-only.
- `%` follows divisor-sign remainder semantics:
  - `5 % -3 == -1`
  - `-5 % -3 == -2`
- Bitwise operators (`& | ^ ~`) are `Int`-only.
- Shifts (`<< >>`) are `Int`-only.
- Logical operators (`&& || !`) are `Bool`-only.
- Operators are not defined for structs/enums directly.

## Valid examples

```vc
func main() {
    let x = (1 + 2) * 3;
    let y = 7 % -3;
    let ok = (x > 0) && (y < 0);
}
```

## Common errors

```vc
func main() {
    let bad = "x" - "y"; // Expected compile-time error: invalid operator for String
}
```

## More examples

```vc
func main() {
    let a = 8;
    let b = 3;
    let c = (a << 1) ^ b;
    let d = (a % -b) + (a / b);
}
```

## See also

- [Comparisons and Equality](./04-comparisons-and-equality.md)
- [Operator Precedence](./15-operator-precedence.md)
