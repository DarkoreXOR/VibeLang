# Functions

## What it is

Functions are top-level declarations with typed parameters, optional defaults, named arguments, variadic `params`, and extension methods.

## Syntax

```vc
func add(a: Int, b: Int): Int { return a + b; }
func id<T>(x: T): T { return x; }
func twice(x: Int) = x * 2;
type Res<T> = Result<T, String>;
func get(): Res<_> = Res::Ok(5);
```

## Rules and constraints

- Functions are top-level only (no nested function declarations).
- Entry point is `func main()` (or `func main(): ()`), no parameters, or `async func main(): Task` / `Task<()>` (no parameters).
- `internal func` can declare host/builtin functions.
- Async (v1):
  - `async func name(...): Task<Payload> { ... }` — body uses `await` on `Task` values; `return expr` returns the **payload** type `Payload` (the `Task<...>` wrapper is only in the signature).
  - `internal async func` is reserved for builtins such as `sleep` that return `Task`.
  - `await expr` is only valid inside `async` function bodies.
  - Calls to `async` functions produce a `Task` immediately; the runtime schedules tasks cooperatively (single-threaded) and progress happens when tasks `await` or yield.
  - `await` suspends the current task until the awaited task is completed.
  - `Task::wait_all(...)` waits for multiple tasks to finish.
  - `async` on extension methods and lambdas is rejected for now.
- Default parameters:
  - `name: Type = expr`
  - must be trailing
  - evaluated at call-time
  - not allowed on `internal func`
- Named arguments:
  - can be reordered
  - positional args cannot follow named args
  - unknown/duplicate names are compile-time errors
- Variadic parameter:
  - `params name: [T]`
  - must be last
  - packs trailing positionals
  - can be passed explicitly by name as an array
- Extension methods:
  - declaration: `func Type::method(...)`
  - instance call: `value.method(...)`
  - static call: `Type::method(...)`
  - struct receivers support both static and instance forms:
    - `func Void::foo<T>(x: T): T { ... }`
    - `func Void::bar<T>(self, x: T): T { ... }`
- Array extension methods:
  - concrete element type: `func [Int]::len(self): Int { ... }`
  - generic default (element type inferred from `self`): `func [type T]::len<T>(self): Int { ... }`
    - use the keyword `type` before the type parameter name in the receiver (`[type T]`, not plain `[T]`).
  - enum-constrained generic receiver is also supported: `func [Result<type T, type E>]::method<T, E>(self): ...`
    - use `type` for enum type parameters inside receiver type arguments.
  - resolution order for arrays:
    1. exact concrete receiver (for example `[Int]::len`)
    2. more specific generic receiver (for example `[Result<type T, type E>]::len`)
    3. generic fallback receiver (`[type T]::len`)
- Generic enum extension methods:
  - declaration: `func Result<type T, type E>::is_ok<T, E>(self): Bool { ... }`
  - both instance and static call resolution support generic fallback from concrete enum types.
- Generic struct-receiver extensions:
  - declaration with generic receiver type parameter: `func T::m<T>(self): T { ... }`
  - for struct instance calls, resolution is concrete first, then generic fallback candidates.
- Operator overloading via extension methods:
  - custom operator implementations are declared as extension methods on the receiver type.
  - operator methods may be overloaded by parameter type (same method name, different typed params).
  - supported operator method names:
    - binary: `binary_add`, `binary_sub`, `binary_mul`, `binary_div`, `binary_mod`, `binary_bitwise_and`, `binary_bitwise_or`, `binary_bitwise_xor`, `binary_left_shift`, `binary_right_shift`, `compare_less`, `compare_less_or_equal`, `compare_greater`, `compare_greater_or_equal`, `compare_equal`, `compare_not_equal`, `binary_and`, `binary_or`
    - unary: `unary_plus`, `unary_minus`, `unary_not`, `unary_bitwise_not`
  - examples:
    - `func Foo::binary_add(self, rhs: Int): Foo { ... }` for `Foo + Int`
    - `func Foo::compare_greater(self, rhs: Float): Bool { ... }` for `Foo > Float`
    - `func Foo::compare_greater(self, rhs: Int): Bool { ... }` for `Foo > Int` (overload)

## Valid examples

```vc
func greet(name: String = "world"): String = "hello, " + name;
enum Result<T, E> { Ok(T), Err(E) }

func sum(params xs: [Int]): Int {
    let i = 0;
    let out = 0;
    while i < int_array_len(xs) {
        out += xs[i];
        i += 1;
    }
    return out;
}

func String::to_string(self): String { return self; }
func Result<type T, type E>::is_ok<T, E>(self): Bool { return true; }

func main() {
    let a = greet();
    let b = greet(name: "vibe");
    let s = "x".to_string();
    let t = sum(1, 2, 3);
    let r = Result<_, String>::Ok(123).is_ok();
}
```

## Common errors

```vc
func f(a: Int = 1, b: Int) {} // Expected compile-time error: non-default after default
```

## More examples

```vc
func join(a: String, b: String = "!"): String = a + b;

func Int::double(self): Int = self * 2;

func main() {
    let a = join(a: "ok");
    let b = 12.double();
    let c = Int::double(20);
}
```

## See also

- [Control Flow](./05-control-flow.md)
- [Async and Tasks](./18-async.md)
- [Modules](./14-modules.md)
