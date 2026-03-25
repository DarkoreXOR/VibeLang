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
- `Float` is implemented via arbitrary-precision `BigFloat` with `1024`-bit precision (rounded using `ToEven`).

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
- `itof(value: Int): Float;`
- `ftoi(value: Float): Int;`
- `stof(value: String): Float;`
- `ftos(value: Float, precision: Int): String;`
- `rand_int(to: Int): Int;`
- `rand_bigint(bits: Int): Int;`
- `int_array_len(a: [T]): Int;`
- `print_any(a: Any);`
- `print_gen<T>(value: T);`
- `clone<T>(value: T): T;`
- `sleep(ms: Int): Task` (`internal async func`) — yields `Task<()>` completed after sleeping.
- `Task<T>::wait_all<T>(params tasks: [Task<T>]): Task` (implemented via the internal `wait_all_tasks_async` helper).

## Async / `Task` (v1)

- `Task<T>` is a VM-managed handle for a cooperatively scheduled async computation.
- Calling an `async func` creates/schedules a task immediately; the callee runs until it hits an `await` (or finishes).
- `await task_expr` suspends the current task until the awaited task completes; the VM then resumes it and yields any returned payload.
- Timers (`sleep`) are non-blocking: they park a task until the deadline expires, while other ready tasks can run.
- `Task::wait_all(...)` waits for multiple tasks and completes only once all tasks have finished.

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
