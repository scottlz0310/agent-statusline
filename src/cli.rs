use clap::{Parser, Subcommand, ValueEnum};

#[derive(Parser, Debug)]
#[command(
    name = "agent-statusline",
    author = "scottlz0310",
    version,
    about = "Ultra-fast zero-fork statusline for AI coding agents"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Render statusline from agent stdin JSON
    Render {
        /// Agent type (agy, claude, copilot)
        #[arg(short, long, value_enum, default_value = "agy")]
        agent: AgentKind,

        /// Shell type for ANSI escaping (pwsh, bash)
        #[arg(short, long, value_enum, default_value = "pwsh")]
        shell: ShellKind,
    },

    /// Automatically install statusline into agent settings
    Install {
        /// Target agents to install (comma-separated: agy, claude, copilot)
        #[arg(short, long, value_delimiter = ',')]
        agent: Vec<AgentKind>,

        /// Install to all supported agents
        #[arg(long)]
        all: bool,

        /// Display changes without modifying files
        #[arg(long)]
        dry_run: bool,
    },

    /// Uninstall statusline from agent settings
    Uninstall {
        /// Target agents to uninstall
        #[arg(short, long, value_delimiter = ',')]
        agent: Vec<AgentKind>,

        /// Uninstall from all supported agents
        #[arg(long)]
        all: bool,
    },

    /// Diagnose statusline installation status across all agents
    Status,

    /// Output shell integration script
    Init {
        #[arg(value_enum)]
        shell: ShellKind,
    },
}

#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum AgentKind {
    Agy,
    Claude,
    Copilot,
}

#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShellKind {
    Pwsh,
    Bash,
}
