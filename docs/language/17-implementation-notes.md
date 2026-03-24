# Implementation Notes

## Compiler/runtime pipeline

Current crate includes:

- lexer
- parser
- AST
- semantic checker (`check_program`)
- visitor utilities
- bytecode generator
- VM runtime

The CLI performs semantic checking, then executes through the VM.

## Inference engine notes

- Semantic inference uses unification with inference variables and substitution solving.
- Constraint collection is usage-driven: assignments, calls, operators, lambda bodies, and returns
  all contribute constraints to the same local graph.
- Solving is compile-time only; runtime and VM do not evaluate generic or inference state.
- Diagnostics intentionally hide internal inference variable identifiers and report readable messages.

## Runtime details

- `Int` is arbitrary precision (`BigInt`).
- Division `/` uses truncating integer division semantics.
- Modulo `%` uses divisor-sign remainder semantics.

Examples:

```vc
5 % -3   // -1
-5 % -3  // -2
```

## Builtins (`internal func`)

Common builtins include:

- `print(s: String);`
- `print_int(v: Int);`
- `itos(n: Int): String;`
- `concat(a: String, b: String): String;`
- `input(): String;`
- `stoi(s: String): Int;`
- `rand_int(to: Int): Int;`
- `rand_bigint(bits: Int): Int;`
- `int_array_len(a: [T]): Int;`
- `print_any(a: Any);`
- `print_gen<T>(value: T);`
- `clone<T>(value: T): T;`
- `sleep(ms: Int): Task` (`internal async func`) — yields `Task<()>` completed after sleeping.

## Async / `Task` (v1)

- `Task<T>` is represented as `Value::Task` in the VM: either **deferred** (not yet run; stores callee name and arguments) or **completed** (payload).
- Calls to user or internal `async` functions compile to `MakeDeferredTask` instead of a direct `Call`; `await` emits `AwaitTask`.
- Awaiting a **deferred** user task pushes a normal call frame and runs that function to completion. Awaiting a **deferred** builtin runs the builtin immediately (for example `sleep` completes to `Task(Completed(()))` after sleeping).
- There is no separate host “driver loop” in v1 beyond this eager completion model.

## Related code and examples

- Language examples live in `examples/`.
- Standard module examples live in `std/`.

## More examples

```vc
internal func rand_bigint(bits: Int): Int;

func main() {
    let v = rand_bigint(128);
    let m = v % -3;
}
```

## See also

- [Diagnostics](./16-diagnostics.md)
- [Modules](./14-modules.md)
