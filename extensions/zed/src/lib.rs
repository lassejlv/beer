use zed_extension_api::{self as zed, Command, LanguageServerId, Result, Worktree};

struct BeerExtension;

impl zed::Extension for BeerExtension {
    fn new() -> Self {
        Self
    }

    fn language_server_command(
        &mut self,
        _language_server_id: &LanguageServerId,
        worktree: &Worktree,
    ) -> Result<Command> {
        let binary = worktree.which("beer").ok_or_else(|| {
            "`beer` not found on $PATH — install it from \
             https://github.com/lassejlv/beer and try again"
                .to_string()
        })?;
        Ok(Command {
            command: binary,
            args: vec!["lsp".to_string()],
            env: Default::default(),
        })
    }
}

zed::register_extension!(BeerExtension);
