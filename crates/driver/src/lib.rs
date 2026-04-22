use std::collections::HashMap;
use std::path::{Path, PathBuf};

use beer_ast::{Func, ParsedFile, Program};
use beer_errors::CompileError;
use beer_lexer as lexer;
use beer_parser as parser;
use beer_source::FileTable;

pub fn load_program(
    root: &Path,
) -> Result<(Program, FileTable), (FileTable, CompileError)> {
    load_program_with(FileTable::new(), root)
}

/// Same as [`load_program`] but starts from a caller-provided [`FileTable`].
/// The LSP uses this to pre-seed editor overlays so unsaved buffers feed into
/// the compiler.
pub fn load_program_with(
    mut files: FileTable,
    root: &Path,
) -> Result<(Program, FileTable), (FileTable, CompileError)> {
    let root_id = match files.load(root, None) {
        Ok(id) => id,
        Err(e) => return Err((files, e)),
    };

    let mut processed: Vec<bool> = Vec::new();
    let mut pending: Vec<u32> = vec![root_id];
    let mut all_funcs: Vec<Func> = Vec::new();

    while let Some(id) = pending.pop() {
        while processed.len() <= id as usize {
            processed.push(false);
        }
        if processed[id as usize] {
            continue;
        }
        processed[id as usize] = true;

        let (source, dir) = {
            let loaded = files.get(id);
            let dir = loaded
                .path
                .parent()
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("."));
            (loaded.source.clone(), dir)
        };

        let tokens = match lexer::tokenize(&source, id) {
            Ok(t) => t,
            Err(e) => return Err((files, e)),
        };
        let parsed: ParsedFile = match parser::parse(tokens) {
            Ok(p) => p,
            Err(e) => return Err((files, e)),
        };

        for u in &parsed.uses {
            let resolved = dir.join(&u.path);
            match files.load(&resolved, Some(u.span)) {
                Ok(child_id) => pending.push(child_id),
                Err(e) => return Err((files, e)),
            }
        }

        all_funcs.extend(parsed.funcs);
    }

    let mut seen: HashMap<String, u32> = HashMap::new();
    for f in &all_funcs {
        if let Some(prev_file) = seen.get(&f.name).copied() {
            let prev_path = files.get(prev_file).path.display().to_string();
            let err = CompileError::at(
                f.span,
                format!(
                    "duplicate function `{}` (also defined in {})",
                    f.name, prev_path
                ),
            );
            return Err((files, err));
        }
        seen.insert(f.name.clone(), f.span.file);
    }

    Ok((Program { funcs: all_funcs }, files))
}
