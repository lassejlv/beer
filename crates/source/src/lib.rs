use std::collections::HashMap;
use std::path::{Path, PathBuf};

use beer_errors::CompileError;
use beer_span::Span;

pub struct LoadedFile {
    pub path: PathBuf,
    pub source: String,
}

#[derive(Default)]
pub struct FileTable {
    files: Vec<LoadedFile>,
    overlays: HashMap<PathBuf, String>,
}

impl FileTable {
    pub fn new() -> Self {
        Self::default()
    }

    /// Pre-seed the table with an unsaved editor buffer. Subsequent `load`
    /// calls that resolve to this path will use `source` instead of reading
    /// from disk.
    pub fn set_overlay(&mut self, path: PathBuf, source: String) {
        let canonical = std::fs::canonicalize(&path).unwrap_or(path);
        self.overlays.insert(canonical, source);
    }

    pub fn load(
        &mut self,
        path: &Path,
        reported_at: Option<Span>,
    ) -> Result<u32, CompileError> {
        let canonical = std::fs::canonicalize(path).map_err(|e| {
            let msg = format!("cannot read {}: {}", path.display(), e);
            match reported_at {
                Some(s) => CompileError::at(s, msg),
                None => CompileError::new(msg),
            }
        })?;

        if let Some(id) = self.files.iter().position(|f| f.path == canonical) {
            return Ok(id as u32);
        }

        let source = if let Some(s) = self.overlays.get(&canonical) {
            s.clone()
        } else {
            std::fs::read_to_string(&canonical).map_err(|e| {
                let msg = format!("cannot read {}: {}", canonical.display(), e);
                match reported_at {
                    Some(s) => CompileError::at(s, msg),
                    None => CompileError::new(msg),
                }
            })?
        };

        let id = self.files.len() as u32;
        self.files.push(LoadedFile { path: canonical, source });
        Ok(id)
    }

    pub fn get(&self, id: u32) -> &LoadedFile {
        &self.files[id as usize]
    }
}
