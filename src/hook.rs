use serde::Deserialize;
use std::collections::HashMap;
use std::path::PathBuf;

use crate::backup;
use crate::diff;
use crate::filetype::{self, FileCategory};
use crate::read;
use crate::stage;
use crate::write;

/// Default threshold below which we let the builtin Read handle text files.
/// Claude Code's Read works fine for files under 48KB (inline mode).
const DEFAULT_THRESHOLD_BYTES: u64 = 48 * 1024;

/// Environment variable to override the threshold.
const THRESHOLD_ENV: &str = "FETTLE_READ_THRESHOLD";

/// File size above which we skip diffing for performance (5MB).
const MAX_DIFF_FILE_SIZE: u64 = 5 * 1024 * 1024;

/// Claude Code pre-tool-use hook JSON format.
#[derive(Debug, Deserialize)]
pub struct HookInput {
    pub tool_name: String,
    pub tool_input: HashMap<String, serde_json::Value>,
}

/// Result of processing a hook.
///
/// All hook invocations exit 0. The decision (allow vs deny) is expressed
/// in JSON on stdout per the Claude Code hooks spec.
pub struct HookResult {
    /// None = allow (pass through to builtin, no output).
    /// Some = deny (fettle handled it, content goes into the JSON envelope).
    pub deny_reason: Option<String>,
}

/// Get the read threshold from env or default.
fn read_threshold() -> u64 {
    std::env::var(THRESHOLD_ENV)
        .ok()
        .and_then(|v| parse_size(&v))
        .unwrap_or(DEFAULT_THRESHOLD_BYTES)
}

/// Parse a size string like "48KB", "64k", "1MB", or just a number (bytes).
fn parse_size(s: &str) -> Option<u64> {
    let s = s.trim().to_lowercase();

    if let Ok(n) = s.parse::<u64>() {
        return Some(n);
    }

    // Try with suffix
    let (num_part, multiplier) = if s.ends_with("kb") {
        (&s[..s.len() - 2], 1024u64)
    } else if s.ends_with("k") {
        (&s[..s.len() - 1], 1024u64)
    } else if s.ends_with("mb") {
        (&s[..s.len() - 2], 1024 * 1024)
    } else if s.ends_with("m") {
        (&s[..s.len() - 1], 1024 * 1024)
    } else {
        return None;
    };

    num_part.trim().parse::<u64>().ok().map(|n| n * multiplier)
}

/// Parse hook input from a JSON string.
pub fn parse_hook_input(json: &str) -> Result<HookInput, String> {
    serde_json::from_str(json).map_err(|e| format!("Failed to parse hook JSON: {e}"))
}

/// Process a hook invocation. This is the decision tree.
pub fn process(input: &HookInput) -> HookResult {
    match input.tool_name.as_str() {
        "Read" => process_read(input),
        "Write" => process_write(input),
        _ => HookResult { deny_reason: None },
    }
}

/// Extract a string field from tool_input.
fn get_str_field<'a>(input: &'a HookInput, field: &str) -> Option<&'a str> {
    input.tool_input.get(field).and_then(|v| v.as_str())
}

/// Process a Read tool call.
fn process_read(input: &HookInput) -> HookResult {
    let file_path = match get_str_field(input, "file_path") {
        Some(p) => PathBuf::from(p),
        None => {
            // Missing file_path: fail open, let builtin handle the error
            return HookResult { deny_reason: None };
        }
    };

    let category = filetype::detect(&file_path);

    // Images, PDFs, notebooks: let the builtin handle them (multimodal)
    if category.allow_builtin() {
        return HookResult { deny_reason: None };
    }

    // Binary files: allow (the builtin will handle or error as appropriate)
    if category == FileCategory::Binary {
        return HookResult { deny_reason: None };
    }

    // Text/SVG files: check size
    let metadata = match std::fs::metadata(&file_path) {
        Ok(m) => m,
        Err(_) => {
            // File doesn't exist or can't stat -- fail open, let the builtin handle
            return HookResult { deny_reason: None };
        }
    };

    let threshold = read_threshold();

    if metadata.len() < threshold {
        // Small file: let the builtin handle it (works great under 48KB)
        return HookResult { deny_reason: None };
    }

    // Large text file: fettle reads it with line numbers
    let offset = input
        .tool_input
        .get("offset")
        .and_then(|v| v.as_u64())
        .map(|v| v as usize);
    let limit = input
        .tool_input
        .get("limit")
        .and_then(|v| v.as_u64())
        .map(|v| v as usize);
    match read::read_file(&file_path, offset, limit) {
        Ok(content) => {
            let size_str = read::format_size(metadata.len());
            let header = format!(
                "fettle: reading {} ({}, {} detected)\n",
                file_path.display(),
                size_str,
                category
            );
            HookResult {
                deny_reason: Some(format!("{header}{content}")),
            }
        }
        Err(_) => {
            // Read failure: fail open, let the builtin try
            HookResult { deny_reason: None }
        }
    }
}

/// Process a Write tool call with the enhanced tiered protocol.
///
/// Decision tree:
/// 1. New file (doesn't exist) -> write directly, return confirmation
/// 2. Existing file, content identical -> skip everything, return "no changes"
/// 3. Existing file, small diff -> backup + write + return summary
/// 4. Existing file, large diff -> backup + stage + return diff + confirm instructions
fn process_write(input: &HookInput) -> HookResult {
    let file_path = match get_str_field(input, "file_path") {
        Some(p) => PathBuf::from(p),
        None => {
            return HookResult {
                deny_reason: Some("fettle: Write hook missing file_path".to_string()),
            };
        }
    };

    let content = match get_str_field(input, "content") {
        Some(c) => c.to_string(),
        None => {
            return HookResult {
                deny_reason: Some("fettle: Write hook missing content".to_string()),
            };
        }
    };

    let file_path_str = file_path.to_string_lossy().to_string();
    let line_count = content.lines().count();
    let size = content.len() as u64;
    let size_str = read::format_size(size);

    // Check if file exists
    if !file_path.exists() {
        // New file: write directly
        match write::write_file(&file_path, &content) {
            Ok(_) => HookResult {
                deny_reason: Some(format!(
                    "fettle: Wrote {file_path_str} ({size_str}, {line_count} lines) [new file]"
                )),
            },
            Err(e) => HookResult {
                deny_reason: Some(format!("fettle: failed to write {file_path_str}: {e}")),
            },
        }
    } else {
        // Existing file: read current content
        let existing = match std::fs::read(&file_path) {
            Ok(bytes) => bytes,
            Err(_) => {
                // Cannot read existing file: write anyway without diff/backup
                return match write::write_file(&file_path, &content) {
                    Ok(_) => HookResult {
                        deny_reason: Some(format!(
                            "fettle: Wrote {file_path_str} ({size_str}, {line_count} lines) [no backup: read failed]"
                        )),
                    },
                    Err(e) => HookResult {
                        deny_reason: Some(format!("fettle: failed to write {file_path_str}: {e}")),
                    },
                };
            }
        };

        // No-change fast path: byte-level comparison
        if existing == content.as_bytes() {
            return HookResult {
                deny_reason: Some(format!(
                    "fettle: No changes to {file_path_str} (content identical)"
                )),
            };
        }

        // Check if existing content is valid UTF-8 for diffing
        let existing_str = match std::str::from_utf8(&existing) {
            Ok(s) => s,
            Err(_) => {
                // Non-UTF-8 (binary) content: backup + write directly, skip diff
                backup::purge_old_backups();
                let backup_info = backup::create_backup(&file_path, &existing);
                let backup_msg = backup_info
                    .as_ref()
                    .map(|b| format!("\n  backup: {}", b.backup_filename))
                    .unwrap_or_default();

                return match write::write_file(&file_path, &content) {
                    Ok(_) => HookResult {
                        deny_reason: Some(format!(
                            "fettle: Wrote {file_path_str} ({size_str}, binary content, diff skipped){backup_msg}"
                        )),
                    },
                    Err(e) => HookResult {
                        deny_reason: Some(format!("fettle: failed to write {file_path_str}: {e}")),
                    },
                };
            }
        };

        // Check file size for diff skip
        if existing.len() as u64 > MAX_DIFF_FILE_SIZE {
            backup::purge_old_backups();
            let backup_info = backup::create_backup(&file_path, &existing);
            let backup_msg = backup_info
                .as_ref()
                .map(|b| format!("\n  backup: {}", b.backup_filename))
                .unwrap_or_default();

            return match write::write_file(&file_path, &content) {
                Ok(_) => HookResult {
                    deny_reason: Some(format!(
                        "fettle: Wrote {file_path_str} ({size_str}, {line_count} lines, diff skipped: file >5MB){backup_msg}"
                    )),
                },
                Err(e) => HookResult {
                    deny_reason: Some(format!("fettle: failed to write {file_path_str}: {e}")),
                },
            };
        }

        // Compute diff
        let diff_result = diff::compute_diff(existing_str, &content, &file_path_str);

        // Opportunistic backup purge
        backup::purge_old_backups();

        // Create backup
        let backup_info = backup::create_backup(&file_path, &existing);
        let backup_msg = backup_info
            .as_ref()
            .map(|b| format!("\n  backup: {}", b.backup_filename))
            .unwrap_or_default();

        // Classify tier
        let thresholds = diff::WriteThresholds::from_env();
        match thresholds.classify(&diff_result) {
            diff::WriteTier::DirectWrite => {
                // Tier 1: write directly
                match write::write_file(&file_path, &content) {
                    Ok(_) => HookResult {
                        deny_reason: Some(format!(
                            "fettle: Wrote {file_path_str} ({size_str}, {line_count} lines, {} changed){backup_msg}",
                            diff_result.summary()
                        )),
                    },
                    Err(e) => HookResult {
                        deny_reason: Some(format!("fettle: failed to write {file_path_str}: {e}")),
                    },
                }
            }
            diff::WriteTier::StagedWrite => {
                // Tier 2: stage for confirmation
                stage::purge_expired_sessions();

                let backup_path_str = backup_info
                    .as_ref()
                    .map(|b| b.backup_path.to_string_lossy().to_string());

                let diff_summary = diff_result.summary();
                let change_pct = if diff_result.old_line_count > 0 {
                    (diff_result.change_ratio() * 100.0) as usize
                } else {
                    100
                };

                match stage::stage_write(
                    &file_path_str,
                    &content,
                    backup_path_str.as_deref(),
                    &diff_summary,
                ) {
                    Ok(session_id) => {
                        let diff_display = diff::truncate_diff(&diff_result.unified);
                        HookResult {
                            deny_reason: Some(format!(
                                "fettle: Staged write for {file_path_str} (session: {session_id})\n  \
                                 {} lines changed ({change_pct}% of file){backup_msg}\n\n\
                                 {diff_display}\n\n\
                                 To apply: run `fettle confirm {session_id}` via Bash\n\
                                 To discard: run `fettle discard {session_id}` via Bash",
                                diff_summary
                            )),
                        }
                    }
                    Err(_) => {
                        // Stage failure: fall back to direct write (Tier 1 behavior)
                        match write::write_file(&file_path, &content) {
                            Ok(_) => HookResult {
                                deny_reason: Some(format!(
                                    "fettle: Wrote {file_path_str} ({size_str}, {line_count} lines, {} changed) [staging failed, wrote directly]{backup_msg}",
                                    diff_result.summary()
                                )),
                            },
                            Err(e) => HookResult {
                                deny_reason: Some(format!(
                                    "fettle: failed to write {file_path_str}: {e}"
                                )),
                            },
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as IoWrite;
    use tempfile::{NamedTempFile, TempDir};

    fn make_hook_input(tool_name: &str, fields: &[(&str, &str)]) -> HookInput {
        let mut tool_input = HashMap::new();
        for (k, v) in fields {
            tool_input.insert(k.to_string(), serde_json::Value::String(v.to_string()));
        }
        HookInput {
            tool_name: tool_name.to_string(),
            tool_input,
        }
    }

    #[test]
    fn test_parse_hook_json() {
        let json = r#"{"tool_name":"Read","tool_input":{"file_path":"/tmp/test.txt"}}"#;
        let input = parse_hook_input(json).unwrap();
        assert_eq!(input.tool_name, "Read");
        assert_eq!(
            input.tool_input.get("file_path").unwrap().as_str().unwrap(),
            "/tmp/test.txt"
        );
    }

    #[test]
    fn test_parse_invalid_json() {
        assert!(parse_hook_input("not json").is_err());
    }

    #[test]
    fn test_read_image_allows_builtin() {
        let input = make_hook_input("Read", &[("file_path", "/tmp/photo.png")]);
        let result = process(&input);
        assert!(result.deny_reason.is_none());
    }

    #[test]
    fn test_read_pdf_allows_builtin() {
        let input = make_hook_input("Read", &[("file_path", "/tmp/doc.pdf")]);
        let result = process(&input);
        assert!(result.deny_reason.is_none());
    }

    #[test]
    fn test_read_notebook_allows_builtin() {
        let input = make_hook_input("Read", &[("file_path", "/tmp/nb.ipynb")]);
        let result = process(&input);
        assert!(result.deny_reason.is_none());
    }

    #[test]
    fn test_read_small_text_allows_builtin() {
        let mut f = NamedTempFile::new().unwrap();
        f.write_all(b"small file\n").unwrap();
        f.flush().unwrap();

        let input = make_hook_input("Read", &[("file_path", f.path().to_str().unwrap())]);
        let result = process(&input);
        assert!(result.deny_reason.is_none());
    }

    #[test]
    fn test_read_large_text_denies_with_content() {
        let mut f = NamedTempFile::with_suffix(".txt").unwrap();
        // Write >48KB of content
        let line = "x".repeat(100) + "\n";
        for _ in 0..500 {
            f.write_all(line.as_bytes()).unwrap();
        }
        f.flush().unwrap();

        let input = make_hook_input("Read", &[("file_path", f.path().to_str().unwrap())]);
        let result = process(&input);
        assert!(result.deny_reason.is_some());
        let reason = result.deny_reason.unwrap();
        assert!(reason.contains("fettle: reading"));
    }

    #[test]
    fn test_read_svg_handled_as_text() {
        let mut f = NamedTempFile::with_suffix(".svg").unwrap();
        // Write >48KB of SVG content
        let line = "<path d=\"M0,0 L100,100\" />\n";
        for _ in 0..2000 {
            f.write_all(line.as_bytes()).unwrap();
        }
        f.flush().unwrap();

        let input = make_hook_input("Read", &[("file_path", f.path().to_str().unwrap())]);
        let result = process(&input);
        assert!(result.deny_reason.is_some());
        let reason = result.deny_reason.unwrap();
        assert!(reason.contains("svg detected"));
    }

    #[test]
    fn test_read_nonexistent_allows_builtin() {
        let input = make_hook_input("Read", &[("file_path", "/tmp/nonexistent_fettle_test.txt")]);
        let result = process(&input);
        // Fail open: let builtin handle the error
        assert!(result.deny_reason.is_none());
    }

    #[test]
    fn test_write_new_file() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("new_output.txt");
        let input = make_hook_input(
            "Write",
            &[
                ("file_path", path.to_str().unwrap()),
                ("content", "hello from fettle\n"),
            ],
        );
        let result = process(&input);
        assert!(result.deny_reason.is_some());
        let reason = result.deny_reason.unwrap();
        assert!(reason.contains("fettle: Wrote"));
        assert!(reason.contains("[new file]"));
        assert!(reason.contains("1 lines"));

        // Verify file was actually written
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "hello from fettle\n"
        );
    }

    #[test]
    fn test_write_creates_dirs() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("a").join("b").join("deep.txt");
        let input = make_hook_input(
            "Write",
            &[
                ("file_path", path.to_str().unwrap()),
                ("content", "deep write\n"),
            ],
        );
        let result = process(&input);
        assert!(result.deny_reason.is_some());
        assert!(path.exists());
    }

    #[test]
    fn test_write_missing_content() {
        let input = make_hook_input("Write", &[("file_path", "/tmp/test.txt")]);
        let result = process(&input);
        assert!(result.deny_reason.is_some());
        assert!(result.deny_reason.unwrap().contains("missing content"));
    }

    #[test]
    fn test_write_no_change() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("unchanged.txt");
        let content = "existing content\n";
        std::fs::write(&path, content).unwrap();

        let input = make_hook_input(
            "Write",
            &[("file_path", path.to_str().unwrap()), ("content", content)],
        );
        let result = process(&input);
        assert!(result.deny_reason.is_some());
        let reason = result.deny_reason.unwrap();
        assert!(reason.contains("No changes"));
        assert!(reason.contains("content identical"));
    }

    #[test]
    fn test_write_small_diff() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("small_diff.txt");
        let backup_dir = dir.path().join("backups");
        unsafe {
            std::env::set_var("FETTLE_BACKUP_DIR", backup_dir.to_str().unwrap());
        }

        // Create a file with several lines
        let old_content = "line1\nline2\nline3\nline4\nline5\n";
        std::fs::write(&path, old_content).unwrap();

        // Change one line (2 changed lines: 1 delete + 1 insert = under floor of 10)
        let new_content = "line1\nmodified\nline3\nline4\nline5\n";
        let input = make_hook_input(
            "Write",
            &[
                ("file_path", path.to_str().unwrap()),
                ("content", new_content),
            ],
        );
        let result = process(&input);
        assert!(result.deny_reason.is_some());
        let reason = result.deny_reason.unwrap();
        assert!(reason.contains("fettle: Wrote"));
        assert!(reason.contains("changed"));
        assert!(reason.contains("backup:"));

        // File should be written
        assert_eq!(std::fs::read_to_string(&path).unwrap(), new_content);

        // Backup should exist
        assert!(backup_dir.exists());
        let backup_count = std::fs::read_dir(&backup_dir).unwrap().flatten().count();
        assert!(backup_count >= 1); // At least the backup + sidecar

        unsafe {
            std::env::remove_var("FETTLE_BACKUP_DIR");
        }
    }

    #[test]
    fn test_write_large_diff_stages() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("large_diff.txt");
        let backup_dir = dir.path().join("backups");
        let stage_test_dir = dir.path().join("stage");
        unsafe {
            std::env::set_var("FETTLE_BACKUP_DIR", backup_dir.to_str().unwrap());
        }
        unsafe {
            std::env::set_var("FETTLE_STAGE_DIR", stage_test_dir.to_str().unwrap());
        }

        // Create a file with many lines
        let old_lines: Vec<String> = (1..=20).map(|i| format!("line{i}")).collect();
        let old_content = old_lines.join("\n") + "\n";
        std::fs::write(&path, &old_content).unwrap();

        // Change most of the lines (over 40% ratio and over floor)
        let new_lines: Vec<String> = (1..=20).map(|i| format!("changed{i}")).collect();
        let new_content = new_lines.join("\n") + "\n";

        let input = make_hook_input(
            "Write",
            &[
                ("file_path", path.to_str().unwrap()),
                ("content", &new_content),
            ],
        );
        let result = process(&input);
        assert!(result.deny_reason.is_some());
        let reason = result.deny_reason.unwrap();
        assert!(reason.contains("Staged write for"));
        assert!(reason.contains("session:"));
        assert!(reason.contains("fettle confirm"));
        assert!(reason.contains("fettle discard"));

        // File should NOT be written (still has old content)
        assert_eq!(std::fs::read_to_string(&path).unwrap(), old_content);

        // Staging directory should exist
        assert!(stage_test_dir.exists());

        unsafe {
            std::env::remove_var("FETTLE_BACKUP_DIR");
        }
        unsafe {
            std::env::remove_var("FETTLE_STAGE_DIR");
        }
    }

    #[test]
    fn test_unknown_tool_allows() {
        let input = make_hook_input("Bash", &[("command", "echo hi")]);
        let result = process(&input);
        assert!(result.deny_reason.is_none());
    }

    #[test]
    fn test_parse_size_bytes() {
        assert_eq!(parse_size("49152"), Some(49152));
    }

    #[test]
    fn test_parse_size_kb() {
        assert_eq!(parse_size("48KB"), Some(48 * 1024));
        assert_eq!(parse_size("48k"), Some(48 * 1024));
        assert_eq!(parse_size("48kb"), Some(48 * 1024));
    }

    #[test]
    fn test_parse_size_mb() {
        assert_eq!(parse_size("1MB"), Some(1024 * 1024));
        assert_eq!(parse_size("1m"), Some(1024 * 1024));
    }

    #[test]
    fn test_parse_size_invalid() {
        assert_eq!(parse_size("abc"), None);
        assert_eq!(parse_size(""), None);
    }

    #[test]
    fn test_threshold_env_override() {
        // This test just validates parse_size works for threshold logic
        // We can't easily test env var in parallel tests without race conditions
        let val = parse_size("64KB").unwrap();
        assert_eq!(val, 64 * 1024);
    }

    #[test]
    fn test_read_large_text_with_offset_and_limit() {
        let mut f = NamedTempFile::with_suffix(".txt").unwrap();
        // Write >48KB of numbered lines
        for i in 1..=500 {
            let line = format!("line-{i}-{}\n", "x".repeat(95));
            f.write_all(line.as_bytes()).unwrap();
        }
        f.flush().unwrap();

        // Build hook input with numeric offset and limit values
        let mut tool_input = HashMap::new();
        tool_input.insert(
            "file_path".to_string(),
            serde_json::Value::String(f.path().to_str().unwrap().to_string()),
        );
        tool_input.insert("offset".to_string(), serde_json::json!(100));
        tool_input.insert("limit".to_string(), serde_json::json!(5));
        let input = HookInput {
            tool_name: "Read".to_string(),
            tool_input,
        };

        let result = process(&input);
        assert!(result.deny_reason.is_some());
        let reason = result.deny_reason.unwrap();

        // Should contain lines 100-104 (offset=100, limit=5)
        assert!(reason.contains("line-100-"), "should contain line 100");
        assert!(reason.contains("line-104-"), "should contain line 104");
        // Should NOT contain lines outside the window
        assert!(!reason.contains("line-99-"), "should not contain line 99");
        assert!(!reason.contains("line-105-"), "should not contain line 105");
    }
}
