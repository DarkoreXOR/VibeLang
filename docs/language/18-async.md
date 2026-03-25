# Async and Tasks

## What it is

Vibelang’s async model is a single-threaded cooperative scheduler built around `Task<T>`, `async func`, and `await`.

An `async func` returns a `Task<Payload>` immediately, and runs cooperatively until it `await`s (or completes).

## Syntax

### Declaring an async function

```vc
async func fetch(): Task<String> {
    // ...
    return "ok"; // returns the payload type (String)
}
```

### Awaiting

```vc
async func main(): Task<()> {
    let v: String = await fetch();
    return ();
}
```

### Waiting for multiple tasks

`Task::wait_all` is a variadic-style async helper:

```vc
internal async func sleep(ms: Int): Task;

async func main(): Task {
    await Task::wait_all(
        sleep(5000),
        sleep(7000),
    );
}
```

### Non-blocking sleep

```vc
async func main(): Task {
    print_gen("Waiting...");
    await sleep(1000);
    print_gen("Done");
}
```

## Rules and constraints

- `await` is only valid inside `async func` bodies.
- Calling an `async func` creates/schedules a `Task` immediately.
- `await task_expr` suspends the current task until `task_expr` is completed, letting other ready tasks run.
- `async func name(...): Task<Payload>`:
  - `return expr;` returns `expr`’s type as the task payload type `Payload`
  - the `Task<...>` wrapper is only in the function signature
- `Task::wait_all(...)` only accepts tasks with the same payload type `T`:
  - `Task::wait_all([Task<Int>...])` is valid
  - mixing `Task<Int>` with `Task<Float>` is a type error
- `sleep(ms)` is non-blocking: it parks the current task until the timer expires.

## Valid examples

This is the pattern used by `examples/example28.vc`:

```vc
async func t1(): Task {
    print_gen("Waiting for 5 seconds...");
    await sleep(5000);
    print_gen("Finished 5 seconds");
}

async func t2(): Task {
    print_gen("Waiting for 7 seconds...");
    await sleep(7000);
    print_gen("Finished 7 seconds");
}

async func main(): Task {
    await Task::wait_all(t1(), t2());
}
```

## Common errors

- `await` outside `async` functions is rejected at compile time.
- Comparing/operating on `Task<T>` directly with arithmetic or ordering operators is rejected (you must `await` to get the payload).

## See also

- [Functions](./06-functions.md)
- [Types](./01-types.md)

