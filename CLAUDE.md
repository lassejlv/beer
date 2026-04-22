# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

`beer` is a tiny compiled language implemented in Rust on top of LLVM (via the `inkwell` crate targeting LLVM 21). Source files (`.beer`) are lexed → parsed → lowered to LLVM IR → written to an object file → linked to a native binary by invoking the system `cc`.

## Toolchain prerequisites

- Rust edition 2024 (needs rustc 1.85+).
- LLVM **21** installed and discoverable via `LLVM_SYS_210_PREFIX`. `.cargo/config.toml` hardcodes this to `/opt/homebrew/opt/llvm@21` (macOS Homebrew). If LLVM lives elsewhere, either edit that file or override the env var for the current shell. Using any LLVM version other than 21 will not work — the `inkwell` feature is pinned to `llvm21-1` in `Cargo.toml`.
- `cc` on PATH — used by `main.rs` to link the compiler-emitted `.o` into the final executable.

The `Dockerfile` (builder stage) shows the Debian equivalent: `llvm-21-dev libpolly-21-dev`, `LLVM_SYS_210_PREFIX=/usr/lib/llvm-21`. Use this as a reference if setting up a fresh Linux environment.

## Common commands

```
cargo build --release         # build the compiler
cargo build                   # debug build (slower generated code but faster to compile)
cargo check                   # type-check only; fastest feedback loop
```

Running the compiler after a release build:

```
./target/release/beer build examples/hello.beer -o hello  # produce a binary named ./hello
./target/release/beer run   examples/hello.beer           # compile to a temp binary and execute it
./target/release/beer check examples/hello.beer           # lex+parse+codegen+verify, no object file written
```

Note: the CLI uses `clap` subcommands (`build` / `run` / `check`) — the invocation shown at the top of `README.md` (`beer examples/hello.beer -o hello` with no subcommand) is stale. Prefer `beer build` / `beer run` / `beer check`.

There is no test suite. Validate changes by running the examples in `examples/` and comparing output to the expected block in `README.md`.

Docker: `.github/workflows/docker.yml` is manually triggered (`workflow_dispatch`) and publishes to `ghcr.io/<repo>`. No CI runs on push.

## Compilation pipeline

`main.rs::compile_and_link` is the canonical pipeline:

1. `lexer::tokenize` — hand-written char-by-char lexer producing `Vec<(Token, Span)>`. Single-character lookahead via `peek`, two-char via `peek2` (used to disambiguate float literals from int + `.`). Strings support `\n \t \\ \" \0` escapes.
2. `parser::parse` — recursive-descent for top-level/statements, **Pratt / precedence-climbing** for expressions (`parse_expr_bp`). `binop_info` is the precedence table (OR=1, AND=2, comparisons=3, add/sub=4, mul/div=5, `as`=6). Result is an AST rooted at `ParsedFile { uses, funcs }`.
3. `codegen::compile` — builds an LLVM `Module` through `inkwell`, runs `module.verify()`, creates a `TargetMachine` for the host triple, and writes an object file via `write_to_file(..., FileType::Object, path)`. If `obj_path` is `None` (the `check` command), verification runs but no file is emitted.
4. `cc` is invoked as a subprocess to link the object file to the final executable; the intermediate `.o` goes to `std::env::temp_dir()` and is removed after linking.

The whole `CodeGen` struct lives in one file (`src/codegen.rs`, ~800 lines). It holds `funcs` (LLVM function values), `fn_sigs` (for type-checking calls), `vars` (stack slots + their `Type` for the current function), and cached `printf` globals/format strings. Type-checking is interleaved with lowering — there is no separate typeck pass.

### Language semantics worth knowing before editing codegen

- Types: `int` → `i64`, `float` → `f64`, `bool` → `i1`, `str` → `i8*` (C string, literal only). `void` for functions with no return.
- `print(x)` is dispatched on the **static** type of `x` at codegen time to one of three `printf` format strings (`%lld\n`, `%g\n`, `%s\n`). For `bool`, it selects between two `.rodata` strings "true"/"false". There is no polymorphic runtime.
- `main()` gets special handling: its LLVM signature becomes `i32 @main()` with an implicit `ret i32 0` appended if the user wrote a void-returning `main` (`current_is_main_void`).
- `&&` and `||` short-circuit via conditional branches (see `examples/short_circuit.beer`).
- `as` is the only explicit cast operator; it's handled in the Pratt loop at precedence 6, above all binary ops.

## Error reporting

`CompileError { span: Option<Span>, msg: String }` is the single error type used across stages. `main.rs::render_error` turns it into a `rustc`-style terminal rendering (colored when stderr is a TTY) — prefer `CompileError::at(span, "...")` over `CompileError::new("...")` whenever you have a span, otherwise the caret/line display is lost.

## Current refactor in flight (important)

As of the last snapshot there is in-progress work toward multi-file support. These files have uncommitted changes and are **not yet wired through the rest of the pipeline**:

- `src/span.rs` — `Span` gained a `file: u32` field.
- `src/lexer.rs` — `tokenize` now takes `(src, file)` and tags every span with the given file id.
- `src/parser.rs` — `parse` returns `ParsedFile { uses: Vec<UseDecl>, funcs: Vec<Func> }` (not `Program`). The grammar now accepts `use "path"` declarations at the top of a file, before any `fn`.
- `src/source.rs` — new `FileTable` for loading files by path, deduplicating by canonical path, and assigning file ids.

`main.rs` and `codegen.rs` still call the old single-file API (`lexer::tokenize(src)`, `codegen::compile(&program: &Program, ...)`), so `cargo build` will currently fail until the caller side is updated or a multi-file driver is written that merges `ParsedFile`s into a `Program`. If you're landing changes here, either finish the wiring or revert the refactor — don't leave both APIs live in parallel.

## Layout

```
src/
  main.rs      CLI (clap), file I/O, pipeline orchestration, cc invocation, error rendering
  lexer.rs     source chars → Vec<(Token, Span)>
  parser.rs    tokens → AST (recursive descent + Pratt for expressions)
  ast.rs       AST types (Type, BinOp, UnOp, Expr, Stmt, Func, Program/ParsedFile, UseDecl)
  codegen.rs   AST → LLVM IR → object file (inkwell); also does type-checking
  error.rs     CompileError + Display impl
  span.rs      Span (file, line, col)
  source.rs    FileTable — multi-file loader (new, unused by main.rs yet)
examples/      *.beer programs used for manual validation
Dockerfile     two-stage: rust+llvm21 builder → debian-slim runtime with libllvm21 + gcc
```
