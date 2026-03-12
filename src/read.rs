use std::fs;
use std::io::{self, BufRead, BufReader};
use std::path::Path;

/// Read a file and format output with `cat -n` style line numbers.
///
/// Returns the formatted string. No size limits.
pub fn read_file(path: &Path, offset: Option<usize>, limit: Option<usize>) -> io::Result<String> {
    let file = fs::File::open(path)?;
    let reader = BufReader::new(file);
    let mut output = String::new();

    // offset is 1-based line number to start from (like cat -n)
    let skip = offset.unwrap_or(1).saturating_sub(1);

    let lines: Box<dyn Iterator<Item = io::Result<String>>> = match limit {
        Some(n) => Box::new(reader.lines().skip(skip).take(n)),
        None => Box::new(reader.lines().skip(skip)),
    };

    for (i, line_result) in lines.enumerate() {
        let line = line_result?;
        let line_num = skip + i + 1;
        // cat -n format: right-aligned 6-char line number, tab, content
        output.push_str(&format!("{line_num:6}\t{line}\n"));
    }

    Ok(output)
}

/// Get file metadata for confirmation messages.
#[cfg(test)]
pub struct FileInfo {
    pub size: u64,
    pub line_count: usize,
}

/// Count lines and get size of a file.
#[cfg(test)]
pub fn file_info(path: &Path) -> io::Result<FileInfo> {
    let metadata = fs::metadata(path)?;
    let size = metadata.len();

    let file = fs::File::open(path)?;
    let reader = BufReader::new(file);
    let line_count = reader.lines().count();

    Ok(FileInfo { size, line_count })
}

/// Format a byte size in a human-readable way.
pub fn format_size(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes}B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1}KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1}MB", bytes as f64 / (1024.0 * 1024.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as IoWrite;
    use tempfile::NamedTempFile;

    fn temp_file_with(content: &str) -> NamedTempFile {
        let mut f = NamedTempFile::new().unwrap();
        f.write_all(content.as_bytes()).unwrap();
        f.flush().unwrap();
        f
    }

    #[test]
    fn test_read_basic() {
        let f = temp_file_with("hello\nworld\n");
        let output = read_file(f.path(), None, None).unwrap();
        assert!(output.contains("1\thello\n"));
        assert!(output.contains("2\tworld\n"));
    }

    #[test]
    fn test_read_line_numbers_right_aligned() {
        let f = temp_file_with("a\n");
        let output = read_file(f.path(), None, None).unwrap();
        // cat -n uses 6-char right-aligned numbers
        assert_eq!(output, "     1\ta\n");
    }

    #[test]
    fn test_read_with_offset() {
        let content = "line1\nline2\nline3\nline4\nline5\n";
        let f = temp_file_with(content);
        let output = read_file(f.path(), Some(3), None).unwrap();
        assert!(!output.contains("line1"));
        assert!(!output.contains("line2"));
        assert!(output.contains("3\tline3\n"));
        assert!(output.contains("4\tline4\n"));
        assert!(output.contains("5\tline5\n"));
    }

    #[test]
    fn test_read_with_limit() {
        let content = "line1\nline2\nline3\nline4\nline5\n";
        let f = temp_file_with(content);
        let output = read_file(f.path(), None, Some(2)).unwrap();
        assert!(output.contains("1\tline1\n"));
        assert!(output.contains("2\tline2\n"));
        assert!(!output.contains("line3"));
    }

    #[test]
    fn test_read_with_offset_and_limit() {
        let content = "line1\nline2\nline3\nline4\nline5\n";
        let f = temp_file_with(content);
        let output = read_file(f.path(), Some(2), Some(2)).unwrap();
        assert!(!output.contains("line1"));
        assert!(output.contains("2\tline2\n"));
        assert!(output.contains("3\tline3\n"));
        assert!(!output.contains("line4"));
    }

    #[test]
    fn test_read_empty_file() {
        let f = temp_file_with("");
        let output = read_file(f.path(), None, None).unwrap();
        assert!(output.is_empty());
    }

    #[test]
    fn test_read_nonexistent_file() {
        let result = read_file(Path::new("/nonexistent/file.txt"), None, None);
        assert!(result.is_err());
    }

    #[test]
    fn test_file_info() {
        let content = "hello\nworld\nfoo\n";
        let f = temp_file_with(content);
        let info = file_info(f.path()).unwrap();
        assert_eq!(info.size, content.len() as u64);
        assert_eq!(info.line_count, 3);
    }

    #[test]
    fn test_format_size() {
        assert_eq!(format_size(500), "500B");
        assert_eq!(format_size(1024), "1.0KB");
        assert_eq!(format_size(48 * 1024), "48.0KB");
        assert_eq!(format_size(1024 * 1024), "1.0MB");
        assert_eq!(format_size(2 * 1024 * 1024 + 512 * 1024), "2.5MB");
    }
}
