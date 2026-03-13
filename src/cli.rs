use clap::{Parser, Subcommand};
use std::path::PathBuf;

/// fettle -- put your file tools in fine fettle.
/// Replaces Claude Code's constrained file tools with proper, unrestricted alternatives.
#[derive(Parser, Debug)]
#[command(name = "fettle", version, about)]
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

    /// Install fettle as Claude Code pre-tool-use hooks.
    Install,

    /// Show fettle configuration and status.
    Info,

    /// Apply a staged Tier 2 write.
    Confirm {
        /// The session ID to confirm
        session_id: String,
    },

    /// Discard a staged write without applying.
    Discard {
        /// The session ID to discard
        session_id: String,
    },

    /// Restore a file from a backup.
    Rollback {
        /// Backup path or filename
        backup: String,

        /// Override the restore target path
        #[arg(long)]
        to: Option<PathBuf>,
    },

    /// Show pending staged sessions and recent backups.
    Status,
}
