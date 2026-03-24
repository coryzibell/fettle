use clap::{Parser, Subcommand};
use std::path::PathBuf;

/// fettle -- put your file tools in fine fettle.
/// Replaces Claude Code's constrained file tools with proper, unrestricted alternatives.
#[derive(Parser, Debug)]
#[command(
    name = "fettle",
    version,
    about = "Put Claude Code's file tools in fine fettle",
    long_about = "Put Claude Code's file tools in fine fettle.\n\n\
        Installs as a Claude Code PreToolUse hook to intercept Read and Write tool calls.\n\
        Reads: bypasses size limits on large text files (>48KB).\n\
        Writes: removes the read-before-write gate, adds backup and diff-based staging.\n\n\
        Quick start:\n  \
          cargo install fettle\n  \
          fettle install\n  \
          fettle info"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Read a file with cat -n style line numbers. No size limits.
    #[command(
        long_about = "Read a file and print it with cat -n style line numbers.\n\n\
        No size limits. Use --offset and --limit to read a specific range of lines."
    )]
    Read {
        /// Path to the file to read
        file: PathBuf,

        /// Line number to start reading from (1-based)
        #[arg(long, value_name = "LINE")]
        offset: Option<usize>,

        /// Maximum number of lines to read from the offset
        #[arg(long, value_name = "LINES")]
        limit: Option<usize>,
    },

    /// Write content from stdin to a file. Creates parent directories if needed.
    #[command(long_about = "Write content from stdin to a file.\n\n\
        Creates parent directories if they do not exist. \
        Overwrites any existing file at the target path.")]
    Write {
        /// Path to the file to write
        file: PathBuf,
    },

    /// Run as a Claude Code PreToolUse hook (reads JSON from stdin).
    #[command(
        long_about = "Run in hook mode for Claude Code's PreToolUse hook system.\n\n\
        Reads a JSON tool call payload from stdin, decides whether to handle it \
        or pass through to the builtin, and writes the hook response to stdout.\n\n\
        This is called by the hook script installed via `fettle install`. \
        You do not normally run this manually."
    )]
    Hook,

    /// Install fettle as a Claude Code pre-tool-use hook.
    #[command(long_about = "Install fettle as a Claude Code pre-tool-use hook.\n\n\
        Registers the hook in ~/.claude/settings.json (primary) and creates a \
        legacy hook script at ~/.claude/hooks/pre-tool-use/fettle (backwards \
        compatibility). Safe to re-run (idempotent).")]
    Install,

    /// Show fettle configuration and status.
    #[command(
        long_about = "Show current fettle configuration and installation status.\n\n\
        Displays: hook installation path, read threshold, write thresholds \
        (floor/ceiling/ratio), backup directory, staging directory, and \
        the full decision tree summary."
    )]
    Info,

    /// Apply a staged write after reviewing the diff.
    #[command(long_about = "Apply a staged Tier 2 write.\n\n\
        When fettle stages a large diff instead of writing directly, it assigns \
        an 8-character hex session ID. Use this command to confirm and apply \
        the staged content to the target file.\n\n\
        Sessions expire after 10 minutes (configurable via FETTLE_WRITE_STAGE_TTL).")]
    Confirm {
        /// The 8-character hex session ID shown in the staging message
        #[arg(value_name = "SESSION_ID")]
        session_id: String,
    },

    /// Discard a staged write without applying it.
    #[command(long_about = "Discard a staged Tier 2 write.\n\n\
        Removes the staged content and session metadata. \
        The original file remains unchanged.")]
    Discard {
        /// The 8-character hex session ID to discard
        #[arg(value_name = "SESSION_ID")]
        session_id: String,
    },

    /// Restore a file from a backup.
    #[command(long_about = "Restore a file from a fettle backup.\n\n\
        The backup can be specified as a filename (resolved within the backup \
        directory) or as a full path. The original target path is read from the \
        .meta sidecar file. Use --to to override the restore target.\n\n\
        Example:\n  \
          fettle rollback main.rs.20260312_143022_517\n  \
          fettle rollback main.rs.20260312_143022_517 --to /tmp/recovered.rs")]
    Rollback {
        /// Backup filename or full path
        #[arg(value_name = "BACKUP")]
        backup: String,

        /// Restore to this path instead of the original
        #[arg(long, value_name = "PATH")]
        to: Option<PathBuf>,
    },

    /// Show pending staged sessions and recent backups.
    #[command(long_about = "Show pending staged sessions and recent backups.\n\n\
        Lists all Tier 2 sessions awaiting confirmation and all backup files \
        created in the last 24 hours.")]
    Status,

    /// Diagnose why an Edit tool search string was not found.
    #[command(
        name = "edit-diagnose",
        long_about = "Diagnose why an Edit tool search string was not found.\n\n\
        When Claude Code's Edit tool reports \"String to replace not found\", run this \
        command to understand why. It searches the file for the exact string and, if \
        not found, shows near-matches with context, highlights likely causes (whitespace, \
        indentation, line endings), and reports match locations if the string does exist.\n\n\
        Example:\n  \
          fettle edit-diagnose src/main.rs 'fn main()'"
    )]
    EditDiagnose {
        /// Path to the file that was being edited
        #[arg(value_name = "FILE")]
        file_path: PathBuf,

        /// The search string (old_string) that was not found
        #[arg(value_name = "SEARCH_STRING")]
        search_string: String,
    },
}
