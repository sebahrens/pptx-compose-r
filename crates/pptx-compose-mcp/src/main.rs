#![deny(warnings)]

use std::path::PathBuf;

use clap::Parser;
use pptx_compose_mcp::{PptxServer, ServerConfig, permissions::PermissionPolicy};

#[derive(Debug, Parser)]
#[command(
    name = "pptx-compose-mcp",
    version,
    about = "pptx-compose MCP server over stdio",
    disable_help_subcommand = true
)]
struct Cli {
    #[arg(long, value_name = "DIR")]
    workspace: Option<PathBuf>,
    #[arg(long, value_name = "DIR")]
    temp_dir: Option<PathBuf>,
    #[arg(long)]
    allow_overwrite: bool,
}

impl Cli {
    fn into_server(self) -> PptxServer {
        let workspace = self
            .workspace
            .or_else(|| env_path("PPTX_COMPOSE_MCP_WORKSPACE"))
            .unwrap_or_else(default_workspace);
        let temp_dir = self
            .temp_dir
            .or_else(|| env_path("PPTX_COMPOSE_MCP_TEMP_DIR"))
            .unwrap_or_else(std::env::temp_dir);
        let config = ServerConfig::default();
        let permission_policy = PermissionPolicy::new(
            workspace,
            temp_dir,
            self.allow_overwrite || env_flag("PPTX_COMPOSE_MCP_ALLOW_OVERWRITE"),
        );

        PptxServer::with_session_store_and_permissions(
            Default::default(),
            config,
            permission_policy,
        )
    }
}

fn default_workspace() -> PathBuf {
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

fn env_path(name: &str) -> Option<PathBuf> {
    std::env::var_os(name)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

fn env_flag(name: &str) -> bool {
    std::env::var(name).is_ok_and(|value| {
        matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        )
    })
}

#[tokio::main]
async fn main() {
    let server = Cli::parse().into_server();
    if let Err(error) = pptx_compose_mcp::run_server(server).await {
        eprintln!("pptx-compose-mcp server error: {error}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use clap::Parser;

    use super::Cli;

    #[test]
    fn parses_server_config_flags() {
        let cli = Cli::try_parse_from([
            "pptx-compose-mcp",
            "--workspace",
            "/tmp/workspace",
            "--temp-dir",
            "/tmp/pptx-temp",
            "--allow-overwrite",
        ])
        .expect("mcp args parse");
        let server = cli.into_server();

        assert_eq!(
            server.permission_policy().workspace_root,
            PathBuf::from("/tmp/workspace")
        );
        assert_eq!(
            server.permission_policy().temp_dir,
            PathBuf::from("/tmp/pptx-temp")
        );
        assert!(server.permission_policy().allow_overwrite);
    }
}
