mod ast;
mod codegen;
mod error;
mod lexer;
mod parser;
mod span;

use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use clap::{Parser, Subcommand};

use error::CompileError;

#[derive(Parser)]
#[command(name = "beer", version, about = "A minimal compiled language", long_about = None)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Compile a source file to a native binary
    Build {
        /// Source file (.beer)
        input: PathBuf,
        /// Output binary path (default: input with extension stripped)
        #[arg(short, long, value_name = "FILE")]
        output: Option<PathBuf>,
    },
    /// Compile and immediately run a source file
    Run {
        /// Source file (.beer)
        input: PathBuf,
    },
    /// Type-check a source file without producing a binary
    Check {
        /// Source file (.beer)
        input: PathBuf,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Build { input, output } => cmd_build(&input, output.as_deref()),
        Cmd::Run { input } => cmd_run(&input),
        Cmd::Check { input } => cmd_check(&input),
    }
}

fn cmd_build(input: &Path, output: Option<&Path>) -> ExitCode {
    let src = match read_source(input) {
        Ok(s) => s,
        Err(code) => return code,
    };
    let out = output
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| default_output(input));
    match compile_and_link(&src, &out) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            render_error(input, &src, &e);
            ExitCode::FAILURE
        }
    }
}

fn cmd_run(input: &Path) -> ExitCode {
    let src = match read_source(input) {
        Ok(s) => s,
        Err(code) => return code,
    };
    let bin = std::env::temp_dir().join(format!("beer-run-{}", std::process::id()));
    if let Err(e) = compile_and_link(&src, &bin) {
        render_error(input, &src, &e);
        let _ = std::fs::remove_file(&bin);
        return ExitCode::FAILURE;
    }
    let status = Command::new(&bin).status();
    let _ = std::fs::remove_file(&bin);
    match status {
        Ok(s) => s
            .code()
            .map(|c| ExitCode::from(c as u8))
            .unwrap_or(ExitCode::FAILURE),
        Err(e) => {
            eprintln!("beer: failed to execute compiled binary: {}", e);
            ExitCode::FAILURE
        }
    }
}

fn cmd_check(input: &Path) -> ExitCode {
    let src = match read_source(input) {
        Ok(s) => s,
        Err(code) => return code,
    };
    match run_check(&src) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            render_error(input, &src, &e);
            ExitCode::FAILURE
        }
    }
}

fn read_source(input: &Path) -> Result<String, ExitCode> {
    std::fs::read_to_string(input).map_err(|e| {
        eprintln!("beer: cannot read {}: {}", input.display(), e);
        ExitCode::FAILURE
    })
}

fn compile_and_link(src: &str, output: &Path) -> Result<(), CompileError> {
    let tokens = lexer::tokenize(src)?;
    let program = parser::parse(tokens)?;

    let obj = std::env::temp_dir().join(format!("beer-{}.o", std::process::id()));
    codegen::compile(&program, Some(&obj))?;

    let status = Command::new("cc")
        .arg(&obj)
        .arg("-o")
        .arg(output)
        .status()
        .map_err(|e| CompileError::new(format!("failed to invoke cc: {}", e)))?;
    let _ = std::fs::remove_file(&obj);

    if !status.success() {
        return Err(CompileError::new(format!("linker exited with {}", status)));
    }
    Ok(())
}

fn run_check(src: &str) -> Result<(), CompileError> {
    let tokens = lexer::tokenize(src)?;
    let program = parser::parse(tokens)?;
    codegen::compile(&program, None)?;
    Ok(())
}

fn default_output(input: &Path) -> PathBuf {
    let stem = input
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "a.out".into());
    PathBuf::from(stem)
}

fn render_error(path: &Path, src: &str, err: &CompileError) {
    let color = std::io::stderr().is_terminal();
    let red = if color { "\x1b[31;1m" } else { "" };
    let bold = if color { "\x1b[1m" } else { "" };
    let cyan = if color { "\x1b[36;1m" } else { "" };
    let reset = if color { "\x1b[0m" } else { "" };

    eprintln!("{red}error{reset}{bold}: {}{reset}", err.msg);

    if let Some(s) = err.span {
        eprintln!(" {cyan}-->{reset} {}:{}:{}", path.display(), s.line, s.col);
        if let Some(line_text) = src.lines().nth((s.line - 1) as usize) {
            let line_num = s.line.to_string();
            let pad = " ".repeat(line_num.len());
            eprintln!("  {pad}{cyan}|{reset}");
            eprintln!("  {cyan}{line_num} |{reset} {line_text}");
            eprintln!(
                "  {pad}{cyan}|{reset} {}{red}^{reset}",
                " ".repeat((s.col as usize).saturating_sub(1))
            );
        }
    }
}
