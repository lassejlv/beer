# beer

A minimal compiled language (Rust + LLVM via [inkwell](https://github.com/thedan64/inkwell)).

Syntax is a small slice of Rust/TypeScript: `let`, `fn`, `if`/`else`, `while`, `return`, and a built-in `print`. Types are `int` (i64), `bool` (i1), `str` (C string, literal only). Programs compile to native binaries via LLVM + the system linker.

## Prerequisites

- Rust 1.85+ (edition 2024)
- LLVM 21 — on macOS: `brew install llvm@21`
- A C compiler reachable as `cc` (bundled with Xcode Command Line Tools on macOS)

`.cargo/config.toml` already points `LLVM_SYS_210_PREFIX` at `/opt/homebrew/opt/llvm@21`. If your LLVM lives elsewhere, override that variable.

## Build

```
cargo build --release
```

## Compile and run a program

```
./target/release/beer examples/hello.beer -o hello
./hello
```

Expected output:
```
42
beer brewed
0
1
2
true
```

## Language

```
fn add(a: int, b: int) -> int {
    return a + b
}

fn main() {
    let x = 10
    let y = add(x, 32)
    print(y)

    let i = 0
    while i < 3 {
        print(i)
        i = i + 1
    }

    if y == 42 {
        print(true)
    } else {
        print(false)
    }
}
```

- Types: `int`, `bool`, `str`.
- Operators: `+ - * /`, `== != < <= > >=`, `&& ||`, unary `-` and `!`.
- `let x = EXPR` binds a mutable variable (type inferred). `x = EXPR` reassigns.
- Functions must declare parameter types; return type is optional (omitted = void).
- `print(x)` dispatches on the static type of `x` (int/str/bool).
- `main()` is the program entry — return type is inferred as `i32 @main()` with an implicit `return 0`.

## Layout

```
src/
  lexer.rs     # source -> tokens
  parser.rs    # tokens -> AST (recursive descent + Pratt)
  ast.rs       # AST types
  codegen.rs   # AST -> LLVM IR -> object file
  main.rs      # CLI: reads source, runs pipeline, links with `cc`
examples/
  hello.beer
```
