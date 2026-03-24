# Definite Assignment

## What it is

A variable must be assigned on all required control-flow paths before it is read.

## Syntax

```vc
let x;
x = 1;
let y = x;
```

## Rules and constraints

- Reads before assignment are errors.
- For `if/else`, a variable is readable after the statement only if assigned on all branches.
- For `while`, analysis follows current Rust-like conservative/widening behavior:
  - non-literal-true loop condition -> conservative merge
  - literal `true` loop -> widening with `break` flow merge

## Valid examples

```vc
func main() {
    let x;
    if true {
        x = 1;
    } else {
        x = 2;
    }
    let y = x;
}
```

## Common errors

```vc
func main() {
    let x;
    if true {
        x = 1;
    }
    let y = x; // Expected compile-time error: possibly unassigned
}
```

## More examples

```vc
func main() {
    let out: Int;
    if true { out = 1; } else { out = 2; }
    let y = out;
}
```

## See also

- [Variables and Assignment](./02-variables-and-assignment.md)
- [Control Flow](./05-control-flow.md)
