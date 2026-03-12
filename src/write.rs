use std::fs;
use std::io::{self, Read};
use std::path::Path;

use crate::read::format_size;

/// Write content to a file. Creates parent directories if needed.
/// Returns a confirmation message with file size and line count.
pub fn write_file(path: &Path, content: &str) -> io::Result<String> {
    // Create parent directories if they don't exist
    if let Some(parent) = path.parent()
        && !parent.exists()
    {
        fs::create_dir_all(parent)?;
    }

    fs::write(path, content)?;

    let line_count = content.lines().count();
    let size = content.len() as u64;
    let size_str = format_size(size);

    Ok(format!(
        "Wrote {} ({}, {} lines)",
        path.display(),
        size_str,
        line_count
    ))
}

/// Read content from stdin for CLI write mode.
pub fn read_stdin() -> io::Result<String> {
    let mut buf = String::new();
    io::stdin().read_to_string(&mut buf)?;
    Ok(buf)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_write_basic() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test.txt");
        let msg = write_file(&path, "hello\nworld\n").unwrap();
        assert!(msg.contains("test.txt"));
        assert!(msg.contains("2 lines"));
        assert_eq!(fs::read_to_string(&path).unwrap(), "hello\nworld\n");
    }

    #[test]
    fn test_write_creates_parent_dirs() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("a").join("b").join("c").join("file.txt");
        let msg = write_file(&path, "nested\n").unwrap();
        assert!(msg.contains("file.txt"));
        assert!(path.exists());
        assert_eq!(fs::read_to_string(&path).unwrap(), "nested\n");
    }

    #[test]
    fn test_write_overwrites_existing() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("overwrite.txt");
        fs::write(&path, "old content").unwrap();
        let _ = write_file(&path, "new content\n").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "new content\n");
    }

    #[test]
    fn test_write_empty_file() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("empty.txt");
        let msg = write_file(&path, "").unwrap();
        assert!(msg.contains("0B"));
        assert!(msg.contains("0 lines"));
    }

    #[test]
    fn test_write_confirmation_format() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("confirm.txt");
        let content = "line1\nline2\nline3\n";
        let msg = write_file(&path, content).unwrap();
        assert!(msg.starts_with("Wrote "));
        assert!(msg.contains("3 lines"));
    }
}
