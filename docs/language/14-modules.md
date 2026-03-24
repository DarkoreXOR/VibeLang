# Modules

## What it is

Modules split code into multiple files with explicit `import`/`export`.

## Syntax

```vc
import { Option, print_gen } from "std/core";
```

Exports:

```vc
export func f() {}
export enum Option<T> { None, Some(T) }
export struct Point { x: Int, y: Int }
export internal func itos(n: Int): String;
```

## Rules and constraints

- Imports are top-level only.
- Only exported names can be imported.
- Imports are explicit: only names listed in `import { ... }` are visible from that module.
  - Exported names that are not explicitly imported are not in scope.
- Cyclic imports are rejected.
- Module resolution order:
  1. `<project-root>/<module/path>.vc`
  2. relative paths (`./`, `../`) from importing file directory

## Valid examples

```vc
// app.vc
import { Option } from "std/core";

func main() {
    let v = Option::Some(1);
}
```

## Common errors

```vc
import { hidden } from "m"; // Expected compile-time error: symbol is not exported
import { print_gen } from "std/core";
func main() { print_gen(Result::Ok(1)); } // Expected compile-time error: `Result` not imported
```

Fixed version:

```vc
import { print_gen, Result } from "std/core";
func main() { print_gen(Result::Ok(1)); }
```

## More examples

```vc
// util.vc
export func plus1(x: Int): Int = x + 1;

// main.vc
import { plus1 } from "./util";
func main() { let y = plus1(41); }
```

## See also

- [Functions](./06-functions.md)
- [Globals](./07-globals.md)
