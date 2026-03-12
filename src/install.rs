use std::fs;
use std::io;
use std::path::PathBuf;

/// The hook directory for Claude Code pre-tool-use hooks.
fn hooks_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home)
        .join(".claude")
        .join("hooks")
        .join("pre-tool-use")
}

/// Find the strop binary path.
fn strop_binary() -> io::Result<PathBuf> {
    std::env::current_exe()
}

/// Install strop as a Claude Code pre-tool-use hook.
///
/// Creates a small shell wrapper script in ~/.claude/hooks/pre-tool-use/
/// that invokes strop in hook mode. This is more portable than a symlink
/// because the hook receives JSON on stdin, and we need to pipe it through.
pub fn install() -> Result<String, String> {
    let hooks_dir = hooks_dir();
    let hook_path = hooks_dir.join("strop");

    // Create the hooks directory if needed
    fs::create_dir_all(&hooks_dir).map_err(|e| {
        format!(
            "Failed to create hooks directory {}: {e}",
            hooks_dir.display()
        )
    })?;

    let strop_bin =
        strop_binary().map_err(|e| format!("Failed to determine strop binary path: {e}"))?;

    // Create a simple wrapper script that passes stdin through to strop
    let script = format!("#!/bin/sh\nexec \"{}\" hook\n", strop_bin.display());

    fs::write(&hook_path, &script)
        .map_err(|e| format!("Failed to write hook script {}: {e}", hook_path.display()))?;

    // Make it executable
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = fs::Permissions::from_mode(0o755);
        fs::set_permissions(&hook_path, perms)
            .map_err(|e| format!("Failed to set permissions on {}: {e}", hook_path.display()))?;
    }

    let mut msg = format!("Installed strop hook at {}\n", hook_path.display());
    msg.push_str(&format!("  Binary: {}\n", strop_bin.display()));
    msg.push_str(&format!("  Hook dir: {}\n", hooks_dir.display()));
    msg.push_str("\nstrop will now intercept Read and Write tool calls.\n");
    msg.push_str("  - Read: images/PDFs/notebooks pass through to builtin\n");
    msg.push_str("  - Read: text < 48KB passes through to builtin\n");
    msg.push_str("  - Read: text >= 48KB handled by strop (no size limits)\n");
    msg.push_str("  - Write: always handled by strop (no read-gate)\n");

    Ok(msg)
}

/// Check installation status.
pub fn status() -> (bool, PathBuf) {
    let hook_path = hooks_dir().join("strop");
    let installed = hook_path.exists();
    (installed, hook_path)
}
