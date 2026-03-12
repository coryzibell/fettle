use clap::{Parser, Subcommand};
use std::path::PathBuf;

/// strop -- the final sharpening.
/// Replaces Claude Code's constrained file tools with sharp, unrestricted alternatives.
#[derive(Parser, Debug)]
#[command(name = "strop", version, about)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Read a file with cat -n style line numbers. No size limits.
    Read {
        /// Path to the file to read
        file: PathBuf,

        /// Line number to start reading from (1-based)
        #[arg(long)]
        offset: Option<usize>,

        /// Maximum number of lines to read
        #[arg(long)]
        limit: Option<usize>,
    },

    /// Write content from stdin to a file. Creates parent directories if needed.
    Write {
        /// Path to the file to write
        file: PathBuf,
    },

    /// Install strop as Claude Code pre-tool-use hooks.
    Install,

    /// Show strop configuration and status.
    Info,
}
