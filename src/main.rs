mod cli;
mod filetype;
mod hook;
mod info;
mod install;
mod read;
mod write;

use clap::Parser;
use std::io::{self, IsTerminal, Read};
use std::process;

fn main() {
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
        run_hook_mode();
        return;
    }

    // If we have subcommand args, parse as CLI
    if args.len() > 1 {
        run_cli_mode();
        return;
    }

    // No args: check if stdin has data (hook invocation without explicit subcommand)
    if !io::stdin().is_terminal() {
        run_hook_mode();
        return;
    }

    // No args, no stdin: show help
    run_cli_mode();
}

fn run_cli_mode() {
    let cli = cli::Cli::parse();

    match cli.command {
        cli::Command::Read {
            file,
            offset,
            limit,
        } => {
            match read::read_file(&file, offset, limit) {
                Ok(output) => print!("{output}"),
                Err(e) => {
                    eprintln!("strop: {e}");
                    process::exit(1);
                }
            }
        }
        cli::Command::Write { file } => {
            let content = match write::read_stdin() {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("strop: failed to read stdin: {e}");
                    process::exit(1);
                }
            };
            match write::write_file(&file, &content) {
                Ok(msg) => println!("{msg}"),
                Err(e) => {
                    eprintln!("strop: {e}");
                    process::exit(1);
                }
            }
        }
        cli::Command::Install => {
            match install::install() {
                Ok(msg) => println!("{msg}"),
                Err(e) => {
                    eprintln!("strop: {e}");
                    process::exit(1);
                }
            }
        }
        cli::Command::Info => {
            print!("{}", info::show());
        }
    }
}

fn run_hook_mode() {
    let mut input = String::new();
    if let Err(e) = io::stdin().read_to_string(&mut input) {
        eprintln!("strop hook: failed to read stdin: {e}");
        process::exit(hook::EXIT_ALLOW);
    }

    let input = input.trim();
    if input.is_empty() {
        // No input, nothing to do — allow the tool call
        process::exit(hook::EXIT_ALLOW);
    }

    let hook_input = match hook::parse_hook_input(input) {
        Ok(h) => h,
        Err(e) => {
            eprintln!("strop hook: {e}");
            // Parse failure: allow the builtin to handle it
            process::exit(hook::EXIT_ALLOW);
        }
    };

    let result = hook::process(&hook_input);

    if let Some(output) = result.output {
        print!("{output}");
    }

    process::exit(result.exit_code);
}
