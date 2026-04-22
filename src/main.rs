use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use clap::{Parser, Subcommand};

use beer_codegen as codegen;
use beer_driver as driver;
use beer_errors::CompileError;
use beer_source::FileTable;

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
    /// Run the Language Server Protocol implementation on stdio.
    /// Your editor launches `beer lsp` and talks JSON-RPC over stdin/stdout.
    Lsp,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Build { input, output } => cmd_build(&input, output.as_deref()),
        Cmd::Run { input } => cmd_run(&input),
        Cmd::Check { input } => cmd_check(&input),
        Cmd::Lsp => cmd_lsp(),
    }
}

fn cmd_lsp() -> ExitCode {
    match beer_lsp::run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("beer lsp: {}", e);
            ExitCode::FAILURE
        }
    }
}

fn cmd_build(input: &Path, output: Option<&Path>) -> ExitCode {
    let out = output
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| default_output(input));
    match compile_and_link(input, &out) {
        Ok(()) => ExitCode::SUCCESS,
        Err((files, e)) => {
            render_error(input, files.as_ref(), &e);
            ExitCode::FAILURE
        }
    }
}

fn cmd_run(input: &Path) -> ExitCode {
    let bin = std::env::temp_dir().join(format!("beer-run-{}", std::process::id()));
    if let Err((files, e)) = compile_and_link(input, &bin) {
        render_error(input, files.as_ref(), &e);
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
    match run_check(input) {
        Ok(()) => ExitCode::SUCCESS,
        Err((files, e)) => {
            render_error(input, files.as_ref(), &e);
            ExitCode::FAILURE
        }
    }
}

type PipelineError = (Option<FileTable>, CompileError);

fn compile_and_link(input: &Path, output: &Path) -> Result<(), PipelineError> {
    let (program, files) = match driver::load_program(input) {
        Ok(v) => v,
        Err((files, e)) => return Err((Some(files), e)),
    };

    let obj = std::env::temp_dir().join(format!("beer-{}.o", std::process::id()));
    if let Err(e) = codegen::compile(&program, Some(&obj)) {
        return Err((Some(files), e));
    }

    let status = match Command::new("cc").arg(&obj).arg("-o").arg(output).status() {
        Ok(s) => s,
        Err(e) => {
            let _ = std::fs::remove_file(&obj);
            return Err((
                Some(files),
                CompileError::new(format!("failed to invoke cc: {}", e)),
            ));
        }
    };
    let _ = std::fs::remove_file(&obj);

    if !status.success() {
        return Err((
            Some(files),
            CompileError::new(format!("linker exited with {}", status)),
        ));
    }
    Ok(())
}

fn run_check(input: &Path) -> Result<(), PipelineError> {
    let (program, files) = match driver::load_program(input) {
        Ok(v) => v,
        Err((files, e)) => return Err((Some(files), e)),
    };
    if let Err(e) = codegen::compile(&program, None) {
        return Err((Some(files), e));
    }
    Ok(())
}

fn default_output(input: &Path) -> PathBuf {
    let stem = input
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "a.out".into());
    PathBuf::from(stem)
}

fn render_error(fallback_path: &Path, files: Option<&FileTable>, err: &CompileError) {
    let color = std::io::stderr().is_terminal();
    let red = if color { "\x1b[31;1m" } else { "" };
    let bold = if color { "\x1b[1m" } else { "" };
    let cyan = if color { "\x1b[36;1m" } else { "" };
    let reset = if color { "\x1b[0m" } else { "" };

    eprintln!("{red}error{reset}{bold}: {}{reset}", err.msg);

    let Some(s) = err.span else {
        return;
    };

    let (path_disp, source_text) = match files {
        Some(ft) => {
            let f = ft.get(s.file);
            (f.path.display().to_string(), f.source.as_str())
        }
        None => (fallback_path.display().to_string(), ""),
    };

    eprintln!(" {cyan}-->{reset} {}:{}:{}", path_disp, s.line, s.col);
    if let Some(line_text) = source_text.lines().nth((s.line - 1) as usize) {
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
