use std::fs;
use std::io;
use std::path::PathBuf;

use serde_json::Value;

/// The hook directory for Claude Code pre-tool-use hooks (legacy script method).
fn hooks_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home)
        .join(".claude")
        .join("hooks")
        .join("pre-tool-use")
}

/// The Claude Code settings.json path.
fn settings_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".claude").join("settings.json")
}

/// Find the fettle binary path.
fn fettle_binary() -> io::Result<PathBuf> {
    std::env::current_exe()
}

/// The hook entry that fettle injects into settings.json.
fn fettle_hook_entry() -> Value {
    serde_json::json!({
        "matcher": "Read|Write|Edit",
        "hooks": [
            {
                "type": "command",
                "command": "fettle hook"
            }
        ]
    })
}

/// Check whether a PreToolUse array already contains a fettle hook entry.
///
/// Looks for any entry whose `hooks` array contains an object with
/// `"command"` matching `"fettle hook"`.
fn has_fettle_hook(pre_tool_use: &Value) -> bool {
    let Some(arr) = pre_tool_use.as_array() else {
        return false;
    };
    for entry in arr {
        if let Some(hooks) = entry.get("hooks").and_then(|h| h.as_array()) {
            for hook in hooks {
                if hook.get("command").and_then(|c| c.as_str()) == Some("fettle hook") {
                    return true;
                }
            }
        }
    }
    false
}

/// Inject the fettle hook into settings.json.
///
/// Returns Ok(true) if newly added, Ok(false) if already present.
fn inject_settings_json() -> Result<bool, String> {
    let path = settings_path();

    // Ensure ~/.claude/ directory exists
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| {
            format!(
                "Failed to create directory {}: {e}",
                parent.display()
            )
        })?;
    }

    // Read existing file or start with empty object
    let contents = match fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) if e.kind() == io::ErrorKind::NotFound => String::from("{}"),
        Err(e) => return Err(format!("Failed to read {}: {e}", path.display())),
    };

    // Parse as JSON
    let mut root: Value = serde_json::from_str(&contents).map_err(|e| {
        format!(
            "Failed to parse {} as JSON: {e}\n\
             Refusing to modify a malformed settings file.",
            path.display()
        )
    })?;

    // Root must be an object
    let Some(root_obj) = root.as_object_mut() else {
        return Err(format!(
            "{} root is not a JSON object. Refusing to modify.",
            path.display()
        ));
    };

    // Navigate to or create hooks.PreToolUse
    let hooks_val = root_obj
        .entry("hooks")
        .or_insert_with(|| serde_json::json!({}));

    if !hooks_val.is_object() {
        return Err(format!(
            "{}: \"hooks\" key exists but is not an object. \
             Refusing to modify to avoid corruption.",
            path.display()
        ));
    }

    let hooks_obj = hooks_val.as_object_mut().unwrap();

    let pre_tool_use = hooks_obj
        .entry("PreToolUse")
        .or_insert_with(|| serde_json::json!([]));

    if !pre_tool_use.is_array() {
        return Err(format!(
            "{}: \"hooks.PreToolUse\" exists but is not an array. \
             Refusing to modify to avoid corruption.",
            path.display()
        ));
    }

    // Check if already present
    if has_fettle_hook(pre_tool_use) {
        return Ok(false);
    }

    // Append the entry
    pre_tool_use
        .as_array_mut()
        .unwrap()
        .push(fettle_hook_entry());

    // Write back with pretty formatting (2-space indent)
    let formatted =
        serde_json::to_string_pretty(&root).map_err(|e| format!("Failed to serialize JSON: {e}"))?;
    let formatted = formatted + "\n";

    fs::write(&path, formatted.as_bytes())
        .map_err(|e| format!("Failed to write {}: {e}", path.display()))?;

    Ok(true)
}

/// Install the legacy hook script at ~/.claude/hooks/pre-tool-use/fettle.
fn install_legacy_script(fettle_bin: &std::path::Path) -> Result<PathBuf, String> {
    let hooks_dir = hooks_dir();
    let hook_path = hooks_dir.join("fettle");

    fs::create_dir_all(&hooks_dir).map_err(|e| {
        format!(
            "Failed to create hooks directory {}: {e}",
            hooks_dir.display()
        )
    })?;

    let script = format!("#!/bin/sh\nexec \"{}\" hook\n", fettle_bin.display());

    fs::write(&hook_path, &script)
        .map_err(|e| format!("Failed to write hook script {}: {e}", hook_path.display()))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = fs::Permissions::from_mode(0o755);
        fs::set_permissions(&hook_path, perms)
            .map_err(|e| format!("Failed to set permissions on {}: {e}", hook_path.display()))?;
    }

    Ok(hook_path)
}

/// Install fettle as a Claude Code pre-tool-use hook.
///
/// Primary method: inject hook configuration into ~/.claude/settings.json.
/// Legacy method: also create the hook script at ~/.claude/hooks/pre-tool-use/fettle.
pub fn install() -> Result<String, String> {
    let fettle_bin =
        fettle_binary().map_err(|e| format!("Failed to determine fettle binary path: {e}"))?;

    // Primary: settings.json injection
    let settings_path = settings_path();
    let settings_status = match inject_settings_json() {
        Ok(true) => format!(
            "  Settings: {} (hook registered for Read|Write|Edit)",
            settings_path.display()
        ),
        Ok(false) => format!(
            "  Settings: {} (fettle hook already configured)",
            settings_path.display()
        ),
        Err(e) => {
            return Err(format!("Failed to update settings.json: {e}"));
        }
    };

    // Legacy: hook script
    let hook_path = install_legacy_script(&fettle_bin)?;

    let mut msg = String::from("fettle installed successfully!\n\n");
    msg.push_str(&settings_status);
    msg.push('\n');
    msg.push_str(&format!(
        "  Script:   {} (legacy compatibility)\n",
        hook_path.display()
    ));
    msg.push_str(&format!("  Binary:   {}\n", fettle_bin.display()));
    msg.push_str(
        "\nfettle will now intercept Read, Write, and Edit tool calls in Claude Code.\n",
    );

    Ok(msg)
}

/// Check installation status for the settings.json method.
///
/// Returns true if a fettle hook entry exists in hooks.PreToolUse.
pub fn settings_json_installed() -> bool {
    let path = settings_path();
    let Ok(contents) = fs::read_to_string(&path) else {
        return false;
    };
    let Ok(root) = serde_json::from_str::<Value>(&contents) else {
        return false;
    };
    let Some(hooks) = root.get("hooks") else {
        return false;
    };
    let Some(pre_tool_use) = hooks.get("PreToolUse") else {
        return false;
    };
    has_fettle_hook(pre_tool_use)
}

/// Check installation status for the legacy script method.
pub fn script_installed() -> (bool, PathBuf) {
    let hook_path = hooks_dir().join("fettle");
    let installed = hook_path.exists();
    (installed, hook_path)
}

/// Check installation status (either method).
#[allow(dead_code)]
pub fn status() -> (bool, PathBuf) {
    let hook_path = hooks_dir().join("fettle");
    let installed = hook_path.exists() || settings_json_installed();
    (installed, hook_path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    /// Helper: set HOME to a temp directory for isolated testing.
    fn with_temp_home(f: impl FnOnce(&std::path::Path)) {
        let tmp = tempfile::tempdir().unwrap();
        let old_home = env::var("HOME").ok();
        // SAFETY: tests using this helper are marked #[serial_test::serial]
        // so no concurrent mutation of environment variables occurs.
        unsafe { env::set_var("HOME", tmp.path()) };

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            f(tmp.path());
        }));

        if let Some(h) = old_home {
            unsafe { env::set_var("HOME", h) };
        } else {
            unsafe { env::remove_var("HOME") };
        }

        if let Err(e) = result {
            std::panic::resume_unwind(e);
        }
    }

    #[test]
    fn test_fettle_hook_entry_shape() {
        let entry = fettle_hook_entry();
        assert_eq!(entry["matcher"], "Read|Write|Edit");
        assert!(entry["hooks"].is_array());
        assert_eq!(entry["hooks"][0]["command"], "fettle hook");
    }

    #[test]
    fn test_has_fettle_hook_positive() {
        let arr = serde_json::json!([
            {
                "matcher": "Read|Write|Edit",
                "hooks": [{"type": "command", "command": "fettle hook"}]
            }
        ]);
        assert!(has_fettle_hook(&arr));
    }

    #[test]
    fn test_has_fettle_hook_negative() {
        let arr = serde_json::json!([
            {
                "matcher": "Read",
                "hooks": [{"type": "command", "command": "other-tool"}]
            }
        ]);
        assert!(!has_fettle_hook(&arr));
    }

    #[test]
    fn test_has_fettle_hook_empty() {
        let arr = serde_json::json!([]);
        assert!(!has_fettle_hook(&arr));
    }

    #[test]
    fn test_has_fettle_hook_not_array() {
        let val = serde_json::json!("not an array");
        assert!(!has_fettle_hook(&val));
    }

    #[test]
    #[serial_test::serial]
    fn test_inject_creates_settings_from_scratch() {
        with_temp_home(|home| {
            let result = inject_settings_json();
            assert!(result.is_ok());
            assert!(result.unwrap()); // newly added

            let path = home.join(".claude").join("settings.json");
            let contents = fs::read_to_string(path).unwrap();
            assert!(contents.ends_with('\n'), "settings.json should end with a trailing newline");
            let root: Value = serde_json::from_str(&contents).unwrap();
            assert!(has_fettle_hook(&root["hooks"]["PreToolUse"]));
        });
    }

    #[test]
    #[serial_test::serial]
    fn test_inject_preserves_existing_content() {
        with_temp_home(|home| {
            let claude_dir = home.join(".claude");
            fs::create_dir_all(&claude_dir).unwrap();
            let settings = claude_dir.join("settings.json");
            fs::write(
                &settings,
                r#"{"existingKey": "existingValue", "hooks": {"PostToolUse": []}}"#,
            )
            .unwrap();

            let result = inject_settings_json();
            assert!(result.is_ok());
            assert!(result.unwrap());

            let contents = fs::read_to_string(&settings).unwrap();
            let root: Value = serde_json::from_str(&contents).unwrap();
            assert_eq!(root["existingKey"], "existingValue");
            assert!(root["hooks"]["PostToolUse"].is_array());
            assert!(has_fettle_hook(&root["hooks"]["PreToolUse"]));
        });
    }

    #[test]
    #[serial_test::serial]
    fn test_inject_idempotent() {
        with_temp_home(|_home| {
            let first = inject_settings_json();
            assert!(first.is_ok());
            assert!(first.unwrap()); // newly added

            let second = inject_settings_json();
            assert!(second.is_ok());
            assert!(!second.unwrap()); // already present
        });
    }

    #[test]
    #[serial_test::serial]
    fn test_inject_rejects_non_object_hooks() {
        with_temp_home(|home| {
            let claude_dir = home.join(".claude");
            fs::create_dir_all(&claude_dir).unwrap();
            let settings = claude_dir.join("settings.json");
            fs::write(&settings, r#"{"hooks": "not an object"}"#).unwrap();

            let result = inject_settings_json();
            assert!(result.is_err());
            assert!(result.unwrap_err().contains("not an object"));
        });
    }

    #[test]
    #[serial_test::serial]
    fn test_inject_rejects_non_array_pre_tool_use() {
        with_temp_home(|home| {
            let claude_dir = home.join(".claude");
            fs::create_dir_all(&claude_dir).unwrap();
            let settings = claude_dir.join("settings.json");
            fs::write(
                &settings,
                r#"{"hooks": {"PreToolUse": "not an array"}}"#,
            )
            .unwrap();

            let result = inject_settings_json();
            assert!(result.is_err());
            assert!(result.unwrap_err().contains("not an array"));
        });
    }

    #[test]
    #[serial_test::serial]
    fn test_inject_rejects_malformed_json() {
        with_temp_home(|home| {
            let claude_dir = home.join(".claude");
            fs::create_dir_all(&claude_dir).unwrap();
            let settings = claude_dir.join("settings.json");
            fs::write(&settings, "not json at all {{{").unwrap();

            let result = inject_settings_json();
            assert!(result.is_err());
            assert!(result.unwrap_err().contains("Failed to parse"));
        });
    }

    #[test]
    #[serial_test::serial]
    fn test_settings_json_installed_false_when_missing() {
        with_temp_home(|_home| {
            assert!(!settings_json_installed());
        });
    }

    #[test]
    #[serial_test::serial]
    fn test_settings_json_installed_true_after_inject() {
        with_temp_home(|_home| {
            inject_settings_json().unwrap();
            assert!(settings_json_installed());
        });
    }

    #[test]
    #[serial_test::serial]
    fn test_inject_appends_to_existing_pre_tool_use() {
        with_temp_home(|home| {
            let claude_dir = home.join(".claude");
            fs::create_dir_all(&claude_dir).unwrap();
            let settings = claude_dir.join("settings.json");
            fs::write(
                &settings,
                r#"{"hooks": {"PreToolUse": [{"matcher": "Bash", "hooks": [{"type": "command", "command": "other-tool"}]}]}}"#,
            )
            .unwrap();

            let result = inject_settings_json();
            assert!(result.is_ok());
            assert!(result.unwrap());

            let contents = fs::read_to_string(&settings).unwrap();
            let root: Value = serde_json::from_str(&contents).unwrap();
            let arr = root["hooks"]["PreToolUse"].as_array().unwrap();
            assert_eq!(arr.len(), 2); // original + fettle
            assert_eq!(arr[0]["matcher"], "Bash");
            assert!(has_fettle_hook(&root["hooks"]["PreToolUse"]));
        });
    }

    #[test]
    #[serial_test::serial]
    fn test_inject_rejects_non_object_root() {
        with_temp_home(|home| {
            let claude_dir = home.join(".claude");
            fs::create_dir_all(&claude_dir).unwrap();
            let settings = claude_dir.join("settings.json");
            fs::write(&settings, "[1, 2, 3]").unwrap();

            let result = inject_settings_json();
            assert!(result.is_err());
            assert!(result.unwrap_err().contains("not a JSON object"));
        });
    }
}
