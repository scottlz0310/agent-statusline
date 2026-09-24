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

        /// Path to custom config file
        #[arg(short, long)]
        config: Option<std::path::PathBuf>,

        /// Output execution time breakdown to stderr
        #[arg(long)]
        bench: bool,
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

        /// Display changes without modifying files
        #[arg(long)]
        dry_run: bool,
    },

    /// Diagnose statusline installation status across all agents
    Status,

    /// Self-update binary to the latest GitHub Release
    Update,

    /// Output shell integration script
    Init {
        #[arg(value_enum)]
        shell: ShellKind,
    },
}

#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AgentKind {
    Agy,
    Claude,
    Copilot,
}

impl AgentKind {
    pub const ALL: [Self; 3] = [Self::Agy, Self::Claude, Self::Copilot];

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Agy => "agy",
            Self::Claude => "claude",
            Self::Copilot => "copilot",
        }
    }

    #[must_use]
    pub const fn display_name(self) -> &'static str {
        match self {
            Self::Agy => "Antigravity CLI (agy)",
            Self::Claude => "Claude Code (claude)",
            Self::Copilot => "GitHub Copilot CLI (copilot)",
        }
    }
}

impl std::fmt::Display for AgentKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShellKind {
    Pwsh,
    Bash,
}
