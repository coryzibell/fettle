use std::path::Path;

/// Run edit diagnostics on a file for a given search string.
///
/// This is the standalone diagnostic tool for when Claude Code's Edit tool
/// reports "String to replace not found" and the user wants to understand why.
/// The diagnostic logic was originally in the hook's `process_edit` error path,
/// but Claude Code validates Edit parameters before the hook fires, making
/// that path unreachable. This command gives users direct access to the same
/// diagnostics.
pub fn run(file_path: &Path, search_string: &str) -> Result<String, String> {
    if !file_path.exists() {
        return Err(format!("File does not exist: {}", file_path.display()));
    }

    let bytes = std::fs::read(file_path)
        .map_err(|e| format!("Cannot read {}: {e}", file_path.display()))?;

    let content = std::str::from_utf8(&bytes)
        .map_err(|_| format!("{} is not valid UTF-8 text", file_path.display()))?;

    let occurrences = find_all_occurrences(content, search_string);

    if occurrences.is_empty() {
        Ok(format_not_found(content, search_string, file_path))
    } else if occurrences.len() == 1 {
        Ok(format_single_match(content, &occurrences[0], file_path))
    } else {
        Ok(format_multiple_matches(content, &occurrences, file_path))
    }
}

/// A match location within the file.
struct Occurrence {
    /// 0-based line index where the match starts.
    start_line: usize,
    /// 0-based line index where the match ends (inclusive).
    end_line: usize,
    /// Byte offset in the file where the match starts.
    byte_offset: usize,
}

/// Find all exact occurrences of `needle` in `content`, returning their line positions.
fn find_all_occurrences(content: &str, needle: &str) -> Vec<Occurrence> {
    let mut results = Vec::new();
    let mut search_from = 0;

    while let Some(byte_offset) = content[search_from..].find(needle) {
        let absolute_offset = search_from + byte_offset;
        let start_line = content[..absolute_offset].matches('\n').count();
        let end_line = start_line + needle.matches('\n').count();
        results.push(Occurrence {
            start_line,
            end_line,
            byte_offset: absolute_offset,
        });
        search_from = absolute_offset + needle.len();
    }

    results
}

/// Format output when the search string is not found.
fn format_not_found(content: &str, search_string: &str, file_path: &Path) -> String {
    let lines: Vec<&str> = content.lines().collect();
    let total = lines.len();
    let mut out = String::new();

    out.push_str(&format!(
        "Not found in {} ({total} lines)\n\n",
        file_path.display()
    ));

    out.push_str(&format!(
        "Searched for ({} chars):\n  {:?}\n\n",
        search_string.len(),
        truncate_str(search_string, 200),
    ));

    // Try to find the first line of search_string as a partial/near match
    let near = find_near_match_context(&lines, search_string);
    out.push_str(&near);

    // Suggest common causes
    out.push_str("\n\nCommon causes:\n");
    out.push_str("  - Whitespace differences (tabs vs spaces, trailing spaces)\n");
    out.push_str("  - Indentation mismatch (wrong number of spaces)\n");
    out.push_str("  - Line ending differences (\\r\\n vs \\n)\n");
    out.push_str("  - The file was modified after you last read it\n");

    // Check for \\r\\n in the file
    if content.contains('\r') {
        out.push_str(
            "\nNote: This file contains \\r\\n (Windows) line endings. \
             Your search string may use \\n (Unix) endings.\n",
        );
    }

    out
}

/// Format output for a single exact match.
fn format_single_match(content: &str, occ: &Occurrence, file_path: &Path) -> String {
    let lines: Vec<&str> = content.lines().collect();
    let total = lines.len();
    let mut out = String::new();

    out.push_str(&format!(
        "Exact match found in {} ({total} lines)\n\n",
        file_path.display()
    ));

    out.push_str(&format!(
        "Match at line {} (byte offset {}):\n",
        occ.start_line + 1,
        occ.byte_offset,
    ));

    out.push_str(&format_context(&lines, occ.start_line, occ.end_line));
    out
}

/// Format output for multiple exact matches.
fn format_multiple_matches(content: &str, occurrences: &[Occurrence], file_path: &Path) -> String {
    let lines: Vec<&str> = content.lines().collect();
    let total = lines.len();
    let mut out = String::new();

    out.push_str(&format!(
        "Found {} exact matches in {} ({total} lines)\n",
        occurrences.len(),
        file_path.display()
    ));

    for (i, occ) in occurrences.iter().enumerate() {
        out.push_str(&format!(
            "\nMatch {} at line {} (byte offset {}):\n",
            i + 1,
            occ.start_line + 1,
            occ.byte_offset,
        ));
        out.push_str(&format_context(&lines, occ.start_line, occ.end_line));
    }

    out.push_str(&format!(
        "\nThe string appears {} times. To replace all occurrences, use replace_all=true.\n\
         To replace a specific one, include more surrounding context in old_string to make it unique.\n",
        occurrences.len()
    ));

    out
}

/// Show lines around a match with line numbers.
fn format_context(lines: &[&str], start_line: usize, end_line: usize) -> String {
    let total = lines.len();
    let ctx_start = start_line.saturating_sub(2);
    let ctx_end = (end_line + 3).min(total);

    let snippet: Vec<String> = lines[ctx_start..ctx_end]
        .iter()
        .enumerate()
        .map(|(j, l)| {
            let line_num = ctx_start + j + 1;
            let marker = if (ctx_start + j) >= start_line && (ctx_start + j) <= end_line {
                ">"
            } else {
                " "
            };
            format!("{marker} {:>4}| {l}", line_num)
        })
        .collect();

    snippet.join("\n") + "\n"
}

/// Find context near where old_string might be in the file.
/// Searches for the first line of old_string as a substring match.
fn find_near_match_context(lines: &[&str], old_string: &str) -> String {
    let total = lines.len();

    // Try to find the first line of old_string as a partial match
    let first_line_of_search = old_string.lines().next().unwrap_or(old_string);
    let trimmed_search = first_line_of_search.trim();

    if !trimmed_search.is_empty() {
        let mut matches_found = Vec::new();
        for (i, line) in lines.iter().enumerate() {
            if line.contains(trimmed_search) {
                matches_found.push(i);
            }
        }

        if !matches_found.is_empty() {
            let mut out = format!(
                "Partial match for first line ({} near-matches):\n",
                matches_found.len()
            );

            // Show up to 3 near-match locations
            for &line_idx in matches_found.iter().take(3) {
                let start = line_idx.saturating_sub(2);
                let end = (line_idx + 3).min(total);
                let snippet: Vec<String> = lines[start..end]
                    .iter()
                    .enumerate()
                    .map(|(j, l)| {
                        let num = start + j + 1;
                        let marker = if start + j == line_idx { ">" } else { " " };
                        format!("{marker} {:>4}| {l}", num)
                    })
                    .collect();
                out.push_str(&format!("\n  Near line {}:\n", line_idx + 1));
                out.push_str(&snippet.join("\n"));
                out.push('\n');
            }

            if matches_found.len() > 3 {
                out.push_str(&format!(
                    "\n  ... and {} more near-matches\n",
                    matches_found.len() - 3
                ));
            }

            return out;
        }
    }

    // No partial match found, show the first few lines for orientation
    let preview_count = 10.min(total);
    let snippet: Vec<String> = lines[..preview_count]
        .iter()
        .enumerate()
        .map(|(i, l)| format!("  {:>4}| {l}", i + 1))
        .collect();
    format!(
        "No partial matches found. File has {total} lines. First {preview_count}:\n{}",
        snippet.join("\n")
    )
}

/// Truncate a string for display, appending "..." if truncated.
fn truncate_str(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        // Find a safe char boundary
        let mut end = max_len;
        while end > 0 && !s.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}...", &s[..end])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use tempfile::TempDir;

    fn write_test_file(dir: &TempDir, name: &str, content: &str) -> PathBuf {
        let path = dir.path().join(name);
        std::fs::write(&path, content).unwrap();
        path
    }

    #[test]
    fn test_exact_match_single() {
        let dir = TempDir::new().unwrap();
        let path = write_test_file(&dir, "test.txt", "line1\nline2\nline3\nline4\nline5\n");

        let result = run(&path, "line3").unwrap();
        assert!(result.contains("Exact match found"));
        assert!(result.contains("line 3"));
    }

    #[test]
    fn test_exact_match_multiple() {
        let dir = TempDir::new().unwrap();
        let content = "hello world\nfoo bar\nhello world\nbaz\nhello world\n";
        let path = write_test_file(&dir, "test.txt", content);

        let result = run(&path, "hello world").unwrap();
        assert!(result.contains("3 exact matches"));
        assert!(result.contains("Match 1"));
        assert!(result.contains("Match 2"));
        assert!(result.contains("Match 3"));
        assert!(result.contains("replace_all=true"));
    }

    #[test]
    fn test_not_found_with_near_match() {
        let dir = TempDir::new().unwrap();
        let content = "fn main() {\n    println!(\"hello\");\n}\n";
        let path = write_test_file(&dir, "test.rs", content);

        // Search for a multiline string whose first line matches but the whole thing does not
        let result = run(&path, "println!(\"hello\");\n    return 0;").unwrap();
        assert!(result.contains("Not found"));
        assert!(result.contains("Partial match"));
        assert!(result.contains("Whitespace differences"));
    }

    #[test]
    fn test_not_found_no_near_match() {
        let dir = TempDir::new().unwrap();
        let content = "line1\nline2\nline3\n";
        let path = write_test_file(&dir, "test.txt", content);

        let result = run(&path, "completely_absent_string").unwrap();
        assert!(result.contains("Not found"));
        assert!(result.contains("No partial matches"));
    }

    #[test]
    fn test_file_does_not_exist() {
        let path = PathBuf::from("/tmp/fettle_edit_diagnose_nonexistent.txt");
        let result = run(&path, "anything");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("does not exist"));
    }

    #[test]
    fn test_multiline_search() {
        let dir = TempDir::new().unwrap();
        let content = "aaa\nbbb\nccc\nddd\neee\n";
        let path = write_test_file(&dir, "test.txt", content);

        let result = run(&path, "bbb\nccc").unwrap();
        assert!(result.contains("Exact match found"));
        assert!(result.contains("line 2"));
    }

    #[test]
    fn test_windows_line_endings_note() {
        let dir = TempDir::new().unwrap();
        let content = "line1\r\nline2\r\nline3\r\n";
        let path = write_test_file(&dir, "test.txt", content);

        let result = run(&path, "line2\nline3").unwrap();
        assert!(result.contains("Not found"));
        assert!(result.contains("Windows"));
    }

    #[test]
    fn test_truncate_str_short() {
        assert_eq!(truncate_str("hello", 10), "hello");
    }

    #[test]
    fn test_truncate_str_long() {
        let long = "a".repeat(300);
        let truncated = truncate_str(&long, 200);
        assert!(truncated.ends_with("..."));
        assert!(truncated.len() <= 203); // 200 + "..."
    }
}
