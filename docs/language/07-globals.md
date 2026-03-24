# Globals

## What it is

Globals are top-level `let` bindings shared across functions in the same linked program.

## Syntax

```vc
let answer: Int = 42;
let title = "Vibelang";
```

## Rules and constraints

- Top-level supports global `let` declarations.
- Global `let` must have an initializer.
- Top-level destructuring tuple/array patterns are rejected.
- Globals are readable inside functions.
- Reassignment of globals is only allowed inside function bodies.

## Valid examples

```vc
let counter: Int = 0;

func inc() {
    counter += 1;
}

func main() {
    inc();
}
```

## Common errors

```vc
let x: Int; // Expected compile-time error: global initializer required
```

## More examples

```vc
let base = 10;

func add_one(): Int {
    return base + 1;
}
```

## See also

- [Variables and Assignment](./02-variables-and-assignment.md)
- [Modules](./14-modules.md)
