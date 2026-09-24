mod adapter;
mod cli;
mod engine;
mod installer;
mod model;
mod modules;
mod sink;

use std::io::{self, Read};

use clap::Parser;
use cli::{AgentKind, Cli, Commands};

use adapter::StatuslineAdapter;
use adapter::agy::AntigravityAdapter;
use adapter::claude::ClaudeAdapter;
use adapter::copilot::CopilotAdapter;
use engine::formatter::render_default;
use sink::notifier::write_ratelimit_status;

fn main() -> io::Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Render { agent, .. } => {
            let mut input = String::new();
            io::stdin().read_to_string(&mut input)?;

            if input.trim().is_empty() {
                return Ok(());
            }

            let adapter: Box<dyn StatuslineAdapter> = match agent {
                AgentKind::Agy => Box::new(AntigravityAdapter),
                AgentKind::Claude => Box::new(ClaudeAdapter),
                AgentKind::Copilot => Box::new(CopilotAdapter),
            };

            let (state, ratelimit_payload) = adapter.parse(&input);

            // Squirrel Notifier 連携 (原子的書き出し)
            if let Some(payload) = ratelimit_payload {
                let _ = write_ratelimit_status(&payload);
            }

            let term_width = terminal_size::terminal_size().map_or(80, |(w, _)| w.0 as usize);

            let output = render_default(&state, term_width);
            println!("{output}");
        }
        Commands::Install {
            agent,
            all,
            dry_run,
        } => {
            println!("Install subcommand (Phase 2): agent={agent:?}, all={all}, dry_run={dry_run}");
        }
        Commands::Uninstall { agent, all } => {
            println!("Uninstall subcommand (Phase 2): agent={agent:?}, all={all}");
        }
        Commands::Status => {
            println!("Status subcommand (Phase 2)");
        }
        Commands::Update => {
            println!("Update subcommand (Phase 4): self-update to latest release");
        }
        Commands::Init { shell } => {
            println!("Init subcommand (Phase 4): shell={shell:?}");
        }
    }

    Ok(())
}
