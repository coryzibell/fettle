mod backup;
mod cli;
mod diff;
mod filetype;
mod hook;
mod info;
mod install;
mod read;
mod stage;
mod write;

use clap::Parser;
use std::io::{self, IsTerminal, Read};
use std::process::ExitCode;

fn main() -> ExitCode {
    // Detection strategy:
    // 1. If we have CLI args (beyond just the binary name), parse as CLI
    // 2. If stdin is not a terminal AND we have no subcommand args, try hook mode
    // 3. If called as `fettle hook`, explicitly enter hook mode
    //
    // The `fettle hook` subcommand is what the install script uses.
    // But we also detect bare invocation with piped stdin for robustness.

    let args: Vec<String> = std::env::args().collect();

    // Explicit hook mode: `fettle hook`
    if args.len() == 2 && args[1] == "hook" {
        return run_hook_mode();
    }

    // If we have subcommand args, parse as CLI
    if args.len() > 1 {
        return run_cli_mode();
    }

    // No args: check if stdin has data (hook invocation without explicit subcommand)
    if !io::stdin().is_terminal() {
        return run_hook_mode();
    }

    // No args, no stdin: show help
    run_cli_mode()
}

fn run_cli_mode() -> ExitCode {
    let cli = cli::Cli::parse();

    match cli.command {
        cli::Command::Read {
            file,
            offset,
            limit,
        } => match read::read_file(&file, offset, limit) {
            Ok(output) => {
                print!("{output}");
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("fettle: {e}");
                ExitCode::FAILURE
            }
        },
        cli::Command::Write { file } => {
            let content = match write::read_stdin() {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("fettle: failed to read stdin: {e}");
                    return ExitCode::FAILURE;
                }
            };
            match write::write_file(&file, &content) {
                Ok(msg) => {
                    println!("{msg}");
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("fettle: {e}");
                    ExitCode::FAILURE
                }
            }
        }
        cli::Command::Install => match install::install() {
            Ok(msg) => {
                println!("{msg}");
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("fettle: {e}");
                ExitCode::FAILURE
            }
        },
        cli::Command::Info => {
            print!("{}", info::show());
            ExitCode::SUCCESS
        }
        cli::Command::Confirm { session_id } => match stage::confirm(&session_id) {
            Ok(msg) => {
                println!("{msg}");
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("{e}");
                ExitCode::FAILURE
            }
        },
        cli::Command::Discard { session_id } => match stage::discard(&session_id) {
            Ok(msg) => {
                println!("{msg}");
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("{e}");
                ExitCode::FAILURE
            }
        },
        cli::Command::Rollback { backup, to } => match backup::rollback(&backup, to.as_deref()) {
            Ok(msg) => {
                println!("{msg}");
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("{e}");
                ExitCode::FAILURE
            }
        },
        cli::Command::Status => {
            print_status();
            ExitCode::SUCCESS
        }
        cli::Command::Hook => {
            // Normally intercepted by the early detection in main(),
            // but handle here for completeness if clap parses it.
            run_hook_mode()
        }
    }
}

fn print_status() {
    let sessions = stage::list_pending_sessions();
    let backups = backup::list_recent_backups();

    if sessions.is_empty() && backups.is_empty() {
        println!("No pending sessions or recent backups.");
        return;
    }

    if !sessions.is_empty() {
        println!("Pending staged writes:");
        for s in &sessions {
            println!(
                "  {}  {:<45} {:<10} {}",
                s.session_id, s.target_path, s.diff_summary, s.age
            );
        }
        println!();
    }

    if !backups.is_empty() {
        println!("Recent backups (last 24h):");
        for b in &backups {
            println!("  {:<40} {:<45} {}", b.backup_name, b.original_path, b.age);
        }
    }
}

fn run_hook_mode() -> ExitCode {
    let mut input = String::new();
    if let Err(e) = io::stdin().read_to_string(&mut input) {
        eprintln!("fettle hook: failed to read stdin: {e}");
        // Fail open: exit 0 with no stdout = allow
        return ExitCode::SUCCESS;
    }

    let input = input.trim();
    if input.is_empty() {
        // No input, nothing to do -- allow the tool call
        return ExitCode::SUCCESS;
    }

    let hook_input = match hook::parse_hook_input(input) {
        Ok(h) => h,
        Err(e) => {
            eprintln!("fettle hook: {e}");
            // Parse failure: allow the builtin to handle it
            return ExitCode::SUCCESS;
        }
    };

    let result = hook::process(&hook_input);

    if let Some(reason) = result.deny_reason {
        // Deny: emit the JSON envelope on stdout
        // The deny tells Claude Code not to also run the builtin Write tool.
        // Fettle has already performed the write -- the "deny" IS the success.
        // Claude sees the deny reason as an "error" but the message confirms
        // the write landed (e.g. "fettle: Wrote /path (+3 -1)").
        let output = serde_json::json!({
            "hookSpecificOutput": {
                "hookEventName": "PreToolUse",
                "permissionDecision": "deny",
                "permissionDecisionReason": reason
            }
        });
        print!("{output}");
    }
    // Allow: print nothing

    // All hook responses exit 0
    ExitCode::SUCCESS
}
