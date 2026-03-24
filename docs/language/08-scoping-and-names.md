# Scoping and Names

## What it is

Scopes define where names are visible and where redeclaration is allowed.

## Syntax

```vc
{
    let x = 1;
}
```

## Rules and constraints

- `{ ... }` creates a block scope.
- Inner scope bindings are not visible outside the block.
- Same-scope redeclaration is an error.
- Inner scopes may shadow outer bindings.
- At file scope, names are unique across:
  - `func`
  - `internal func`
  - top-level `let`
- User `func` cannot shadow `internal func` names.

## Valid examples

```vc
func main() {
    let x = 1;
    {
        let x = 2; // shadowing is allowed
        let y = x;
    }
    let z = x;
}
```

## Common errors

```vc
func main() {
    let x = 1;
    let x = 2; // Expected compile-time error: redeclaration in same scope
}
```

## More examples

```vc
func main() {
    let v = 1;
    if true {
        let v = 2;
        let w = v;
    }
    let z = v;
}
```

## See also

- [Globals](./07-globals.md)
- [Functions](./06-functions.md)
