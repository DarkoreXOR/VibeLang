# Diagnostics

## What it is

Compiler diagnostics report syntax/semantic/runtime issues with source spans.

## Behavior

- The compiler tries to collect multiple errors when possible.
- Errors include source spans (line/column ranges).
- CLI output points at source location for easier debugging.
- Inference diagnostics use human-friendly wording (for example, "an inferred type")
  instead of raw internal solver variable names.
- Conflict diagnostics are emitted when breadcrumb constraints disagree.
- Unresolved-type diagnostics are emitted when required concrete positions remain unknown.
- Type-alias diagnostics include unknown alias names, wrong alias arity, and alias cycle errors.

## Example

```vc
func main() {
    let x: Int = "oops";
    if 1 {}
}
```

Typical results:
- type mismatch for `x`
- non-`Bool` condition in `if`

## Guidance

- Fix syntax errors first (they can cascade).
- Then fix type errors from top to bottom.

## More examples

```vc
func main() {
    let n: Int = "x";
    let arr = [1, 2];
    let y = arr["0"];
}
```

Expected diagnostics include type mismatch and invalid index type.

## See also

- [Types](./01-types.md)
- [Implementation Notes](./17-implementation-notes.md)
