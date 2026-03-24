# Operator Precedence

## What it is

Expression operators follow a Rust/C-like precedence ladder (highest to lowest binding).

## Order (high -> low)

1. Primary (`literal`, `identifier`, `(expr)`, calls, tuple field access)
2. Unary (`+ - ~ !`)
3. Multiplicative (`* / %`)
4. Additive (`+ -`)
5. Shift (`<< >>`)
6. Relational/equality (`< > <= >= == !=`)
7. Bitwise AND (`&`)
8. Bitwise XOR (`^`)
9. Bitwise OR (`|`)
10. Logical AND (`&&`)
11. Logical OR (`||`)

## Syntax examples

```vc
let a = 1 + 2 * 3;          // 1 + (2 * 3)
let b = false || true && true; // false || (true && true)
let c = 1 | 2 & 3;          // 1 | (2 & 3)
let d = ~-1;                // ~( -1 )
```

## Notes

- Unary operators are prefix.
- Most binary operators are left-associative.
- `&&` / `||` short-circuit at runtime.
- Compound assignments (`+=`, `<<=`, etc.) are statement-level tokens.

## More examples

```vc
let x = 1 + 2 * 3 + 4;      // 1 + (2 * 3) + 4
let y = 1 | 2 ^ 3 & 4;      // 1 | (2 ^ (3 & 4))
let z = !false || false;    // (!false) || false
```

## See also

- [Operators](./03-operators.md)
- [Comparisons and Equality](./04-comparisons-and-equality.md)
