mod cli;
mod filetype;
mod hook;
mod info;
mod install;
mod read;
mod write;

use clap::Parser;
use std::io::{self, IsTerminal, Read};
use std::process::ExitCode;

fn main() -> ExitCode {
    // Detection strategy:
    // 1. If we have CLI args (beyond just the binary name), parse as CLI
    // 2. If stdin is not a terminal AND we have no subcommand args, try hook mode
    // 3. If called as `strop hook`, explicitly enter hook mode
    //
    // The `strop hook` subcommand is what the install script uses.
    // But we also detect bare invocation with piped stdin for robustness.

    let args: Vec<String> = std::env::args().collect();

    // Explicit hook mode: `strop hook`
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
                eprintln!("strop: {e}");
                ExitCode::FAILURE
            }
        },
        cli::Command::Write { file } => {
            let content = match write::read_stdin() {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("strop: failed to read stdin: {e}");
                    return ExitCode::FAILURE;
                }
            };
            match write::write_file(&file, &content) {
                Ok(msg) => {
                    println!("{msg}");
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("strop: {e}");
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
                eprintln!("strop: {e}");
                ExitCode::FAILURE
            }
        },
        cli::Command::Info => {
            print!("{}", info::show());
            ExitCode::SUCCESS
        }
    }
}

fn run_hook_mode() -> ExitCode {
    let mut input = String::new();
    if let Err(e) = io::stdin().read_to_string(&mut input) {
        eprintln!("strop hook: failed to read stdin: {e}");
        return ExitCode::from(hook::EXIT_ALLOW as u8);
    }

    let input = input.trim();
    if input.is_empty() {
        // No input, nothing to do — allow the tool call
        return ExitCode::from(hook::EXIT_ALLOW as u8);
    }

    let hook_input = match hook::parse_hook_input(input) {
        Ok(h) => h,
        Err(e) => {
            eprintln!("strop hook: {e}");
            // Parse failure: allow the builtin to handle it
            return ExitCode::from(hook::EXIT_ALLOW as u8);
        }
    };

    let result = hook::process(&hook_input);

    if let Some(output) = result.output {
        print!("{output}");
    }

    ExitCode::from(result.exit_code as u8)
}
