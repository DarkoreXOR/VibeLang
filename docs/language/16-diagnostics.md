# Diagnostics

## What it is

Compiler diagnostics report syntax/semantic/runtime issues with source spans.

## Behavior

- The compiler tries to collect multiple errors when possible.
- Errors include source spans (line/column ranges).
- Warnings are non-fatal diagnostics and do not stop execution.
- CLI output points at source location for easier debugging.
- For unused-symbol warnings, the caret points to the exact identifier token.
- Inference diagnostics use human-friendly wording (for example, "an inferred type")
  instead of raw internal solver variable names.
- Conflict diagnostics are emitted when breadcrumb constraints disagree.
- Unresolved-type diagnostics are emitted when required concrete positions remain unknown.
- Type-alias diagnostics include unknown alias names, wrong alias arity, and alias cycle errors.
- Const diagnostics include reassignment errors (for example, `cannot assign to constant 'PI'`).

## Unused warnings

Unused diagnostics are emitted as `semantic warning` and currently include identifier-based checks
such as:

- imports
- top-level declarations (`func`, `struct`, `enum`, `type`, globals)
- function-scope bindings and parameters
- generic type parameters

Intentional unused names can be prefixed with `_` to suppress these warnings.

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

## Unused warning example

```vc
import { print_gen } from "std/core";
import { Task } from "std/async";

struct S;

func foo() {}

func bar(): Int = 4;

func g<T>(): Int {
    return 123;
}

func main() {
    let b = 0;
    print_gen(bar());
    print_gen(g<Int>());
}
```

Typical warnings:
- `unused import 'Task'`
- `unused struct 'S'`
- `unused function 'foo'`
- `unused generic type parameter 'T'`
- `unused binding 'b'`

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
