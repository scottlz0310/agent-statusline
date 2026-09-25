mod adapter;
mod cli;
mod engine;
mod installer;
mod model;
mod modules;
mod sink;
mod updater;

use std::io::{self, Read};

use clap::Parser;
use cli::{AgentKind, Cli, Commands};

use adapter::StatuslineAdapter;
use adapter::agy::AntigravityAdapter;
use adapter::claude::ClaudeAdapter;
use adapter::copilot::CopilotAdapter;
use engine::style::{BOLD, CYAN, DIMMED, GREEN, RED, RESET, YELLOW};
use installer::{
    AgentConfigTarget, AgentStatusReport, PatchAction, PatchResult, StatusKind, diagnose_agent,
    install_to_agent, uninstall_from_agent,
};
use sink::notifier::write_ratelimit_status;

fn main() -> io::Result<()> {
    if let Ok(exe) = std::env::current_exe() {
        updater::clean_old_executable(&exe);
    }

    let cli = Cli::parse();

    match cli.command {
        Commands::Render {
            agent,
            config,
            bench,
        } => cmd_render(agent, config.as_deref(), bench),
        Commands::Install {
            agent,
            all,
            dry_run,
        } => cmd_install(agent, all, dry_run),
        Commands::Uninstall {
            agent,
            all,
            dry_run,
        } => cmd_uninstall(agent, all, dry_run),
        Commands::Status => {
            cmd_status();
            Ok(())
        }
        Commands::Update {
            check,
            force,
            background,
        } => updater::run_update(check, force, background),
    }
}

fn cmd_render(
    agent: AgentKind,
    config_path: Option<&std::path::Path>,
    bench: bool,
) -> io::Result<()> {
    let start = std::time::Instant::now();

    // 24時間非同期バックグラウンド更新チェック（キャッシュ確認 0ms）
    updater::check_update_background_if_needed();

    let mut input = String::new();
    io::stdin().read_to_string(&mut input)?;

    if input.trim().is_empty() {
        return Ok(());
    }

    let config = engine::config::load_config(config_path);

    let adapter: Box<dyn StatuslineAdapter> = match agent {
        AgentKind::Agy => Box::new(AntigravityAdapter),
        AgentKind::Claude => Box::new(ClaudeAdapter),
        AgentKind::Copilot => Box::new(CopilotAdapter),
    };

    let (state, ratelimit_payload) = adapter.parse(&input);

    // Squirrel Notifier 連携 (原子的書き出し)
    if config.integrations.squirrel_notifier.enabled
        && let Some(payload) = ratelimit_payload
    {
        if let Some(custom_dir) = &config.integrations.squirrel_notifier.output_dir {
            let expanded_path = sink::notifier::expand_env_path(custom_dir);
            let _ = sink::notifier::write_ratelimit_status_to_dir(&payload, &expanded_path);
        } else {
            let _ = write_ratelimit_status(&payload);
        }
    }

    let term_width = terminal_size::terminal_size().map_or(80, |(w, _)| w.0 as usize);

    let output = engine::formatter::render_template(&config, &state, term_width);
    println!("{output}");

    if bench {
        let elapsed = start.elapsed();
        eprintln!(
            "[agent-statusline] Render completed in {:.3}ms",
            elapsed.as_secs_f64() * 1000.0
        );
    }

    Ok(())
}

fn cmd_install(agents: Vec<AgentKind>, all: bool, dry_run: bool) -> io::Result<()> {
    let targets = resolve_agents(agents, all)?;
    for a in targets {
        let Some(target) = AgentConfigTarget::resolve(a) else {
            eprintln!("{YELLOW}⚠ Could not resolve home directory for {a}{RESET}");
            continue;
        };

        let res: PatchResult = install_to_agent(&target, dry_run)?;
        match res.action {
            PatchAction::Installed => {
                let prefix = if dry_run {
                    "[DRY-RUN] Would configure"
                } else {
                    "✓ Configured"
                };
                println!(
                    "{GREEN}{prefix} statusLine for {BOLD}{}{RESET}{GREEN}:{RESET} {}",
                    res.agent.display_name(),
                    res.path.display()
                );
                if let Some(bak) = res.backup_path {
                    println!("  {DIMMED}Backup saved to {}{RESET}", bak.display());
                }
                if let Some(preview) = res.preview {
                    println!("{CYAN}Preview JSON:{RESET}\n{preview}");
                }
            }
            PatchAction::AlreadyInstalled => {
                println!(
                    "{GREEN}✓ statusLine is already up to date for {BOLD}{}{RESET}{GREEN}:{RESET} {}",
                    res.agent.display_name(),
                    res.path.display()
                );
            }
            _ => {}
        }
    }
    Ok(())
}

fn cmd_uninstall(agents: Vec<AgentKind>, all: bool, dry_run: bool) -> io::Result<()> {
    let targets = resolve_agents(agents, all)?;
    for a in targets {
        let Some(target) = AgentConfigTarget::resolve(a) else {
            eprintln!("{YELLOW}⚠ Could not resolve home directory for {a}{RESET}");
            continue;
        };

        let res: PatchResult = uninstall_from_agent(&target, dry_run)?;
        match res.action {
            PatchAction::Uninstalled => {
                let prefix = if dry_run {
                    "[DRY-RUN] Would remove"
                } else {
                    "✓ Removed"
                };
                println!(
                    "{GREEN}{prefix} statusLine from {BOLD}{}{RESET}{GREEN}:{RESET} {}",
                    res.agent.display_name(),
                    res.path.display()
                );
                if let Some(bak) = res.backup_path {
                    println!("  {DIMMED}Backup saved to {}{RESET}", bak.display());
                }
                if let Some(preview) = res.preview {
                    println!("{CYAN}Preview JSON:{RESET}\n{preview}");
                }
            }
            PatchAction::RestoredBackup => {
                let prefix = if dry_run {
                    "[DRY-RUN] Would restore original custom"
                } else {
                    "✓ Restored original custom"
                };
                println!(
                    "{GREEN}{prefix} statusLine for {BOLD}{}{RESET}{GREEN}:{RESET} {}",
                    res.agent.display_name(),
                    res.path.display()
                );
                if let Some(bak) = res.backup_path {
                    println!("  {DIMMED}Backup saved to {}{RESET}", bak.display());
                }
                if let Some(preview) = res.preview {
                    println!("{CYAN}Preview JSON:{RESET}\n{preview}");
                }
            }
            PatchAction::SkippedCustom => {
                println!(
                    "{YELLOW}⚠ Skipped uninstall for {BOLD}{}{RESET}{YELLOW}: custom statusLine preserved{RESET}",
                    res.agent.display_name()
                );
            }
            PatchAction::NotInstalled => {
                println!(
                    "{DIMMED}- statusLine was not configured for {}{RESET}",
                    res.agent.display_name()
                );
            }
            _ => {}
        }
    }
    Ok(())
}

fn cmd_status() {
    let current_exe = std::env::current_exe().map_or_else(
        |_| "agent-statusline".to_string(),
        |p| p.display().to_string(),
    );

    println!("{BOLD}agent-statusline Client Status{RESET}");
    println!("{DIMMED}Binary location: {current_exe}{RESET}\n");

    for a in AgentKind::ALL {
        let Some(target) = AgentConfigTarget::resolve(a) else {
            println!(
                "  {YELLOW}? {BOLD}{}{RESET}: Unable to resolve home directory",
                a.display_name()
            );
            continue;
        };

        let report: AgentStatusReport = diagnose_agent(&target);
        match report.status {
            StatusKind::Installed => {
                println!(
                    "  {GREEN}●{RESET} {BOLD}{}{RESET} ({GREEN}Installed{RESET})",
                    report.agent.display_name()
                );
                println!("    Path:    {}", report.path.display());
                if let Some(cmd) = report.command {
                    println!("    Command: {cmd}");
                }
                if let Some(en) = report.enabled {
                    println!("    Enabled: {en}");
                }
            }
            StatusKind::Custom(cmd) => {
                println!(
                    "  {YELLOW}▲{RESET} {BOLD}{}{RESET} ({YELLOW}Custom Statusline{RESET})",
                    report.agent.display_name()
                );
                println!("    Path:    {}", report.path.display());
                println!("    Command: {cmd}");
            }
            StatusKind::NotConfigured => {
                println!(
                    "  {DIMMED}○{RESET} {BOLD}{}{RESET} ({DIMMED}Not Configured{RESET})",
                    report.agent.display_name()
                );
                println!("    Path:    {}", report.path.display());
            }
            StatusKind::FileNotFound => {
                println!(
                    "  {DIMMED}○{RESET} {BOLD}{}{RESET} ({DIMMED}File Missing{RESET})",
                    report.agent.display_name()
                );
                println!("    Path:    {}", report.path.display());
            }
            StatusKind::InvalidJson(err) => {
                println!(
                    "  {RED}✕{RESET} {BOLD}{}{RESET} ({RED}Invalid JSON{RESET})",
                    report.agent.display_name()
                );
                println!("    Path:    {}", report.path.display());
                println!("    Error:   {err}");
            }
        }
        println!();
    }
}

fn resolve_agents(agents: Vec<AgentKind>, all: bool) -> io::Result<Vec<AgentKind>> {
    if all {
        Ok(AgentKind::ALL.to_vec())
    } else if !agents.is_empty() {
        Ok(agents)
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "No agent specified. Use '--all' or '--agent <agy,claude,copilot>'",
        ))
    }
}
