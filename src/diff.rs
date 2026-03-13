use std::env;

/// Result of computing a diff between old and new file content.
pub struct DiffResult {
    /// Number of inserted lines.
    pub insertions: usize,
    /// Number of deleted lines.
    pub deletions: usize,
    /// Total lines in the original file.
    pub old_line_count: usize,
    /// The unified diff string (with context).
    pub unified: String,
}

impl DiffResult {
    /// Total changed lines (insertions + deletions).
    pub fn changed_lines(&self) -> usize {
        self.insertions + self.deletions
    }

    /// Changed lines as a fraction of the original file size.
    /// Returns 1.0 for empty files (100% change) so that large writes to
    /// empty files get proper threshold scrutiny rather than bypassing it.
    pub fn change_ratio(&self) -> f64 {
        if self.old_line_count == 0 {
            if self.changed_lines() > 0 { 1.0 } else { 0.0 }
        } else {
            self.changed_lines() as f64 / self.old_line_count as f64
        }
    }

    /// Compact summary string: "+N -M"
    pub fn summary(&self) -> String {
        format!("+{} -{}", self.insertions, self.deletions)
    }
}

/// Classification of a write operation.
pub enum WriteTier {
    /// Small diff -- apply immediately.
    DirectWrite,
    /// Large diff -- stage for confirmation.
    StagedWrite,
}

/// Write threshold configuration, loaded from env or defaults.
pub struct WriteThresholds {
    pub absolute_floor: usize,
    pub absolute_ceil: usize,
    pub ratio_threshold: f64,
}

impl Default for WriteThresholds {
    fn default() -> Self {
        Self {
            absolute_floor: 10,
            absolute_ceil: 80,
            ratio_threshold: 0.40,
        }
    }
}

impl WriteThresholds {
    /// Load thresholds from environment variables, falling back to defaults.
    ///
    /// If floor >= ceiling, logs a warning and swaps them.
    pub fn from_env() -> Self {
        let defaults = Self::default();
        let mut result = Self {
            absolute_floor: env::var("FETTLE_WRITE_FLOOR")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(defaults.absolute_floor),
            absolute_ceil: env::var("FETTLE_WRITE_CEIL")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(defaults.absolute_ceil),
            ratio_threshold: env::var("FETTLE_WRITE_RATIO")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(defaults.ratio_threshold),
        };

        if result.absolute_floor >= result.absolute_ceil {
            eprintln!(
                "fettle: warning: FETTLE_WRITE_FLOOR ({}) >= FETTLE_WRITE_CEIL ({}), swapping",
                result.absolute_floor, result.absolute_ceil
            );
            std::mem::swap(&mut result.absolute_floor, &mut result.absolute_ceil);
        }

        result
    }

    /// Classify a diff result into a write tier.
    #[allow(clippy::if_same_then_else)]
    pub fn classify(&self, diff: &DiffResult) -> WriteTier {
        let changed = diff.changed_lines();
        if changed <= self.absolute_floor {
            WriteTier::DirectWrite
        } else if changed >= self.absolute_ceil {
            WriteTier::StagedWrite
        } else if diff.change_ratio() > self.ratio_threshold {
            WriteTier::StagedWrite
        } else {
            WriteTier::DirectWrite
        }
    }
}

/// Compute a line-level diff between old and new content.
///
/// Uses the `similar` crate with the Myers diff algorithm (same as git diff).
/// Returns a `DiffResult` with insertion/deletion counts and a unified diff string.
pub fn compute_diff(old: &str, new: &str, file_path: &str) -> DiffResult {
    let text_diff = similar::TextDiff::from_lines(old, new);

    let mut insertions = 0usize;
    let mut deletions = 0usize;

    for change in text_diff.iter_all_changes() {
        match change.tag() {
            similar::ChangeTag::Insert => insertions += 1,
            similar::ChangeTag::Delete => deletions += 1,
            similar::ChangeTag::Equal => {}
        }
    }

    let unified = text_diff
        .unified_diff()
        .context_radius(3)
        .header(
            &format!("{file_path} (current)"),
            &format!("{file_path} (proposed)"),
        )
        .to_string();

    DiffResult {
        insertions,
        deletions,
        old_line_count: old.lines().count(),
        unified,
    }
}

/// Maximum number of diff lines to display in Tier 2 messages.
/// Diffs longer than this are truncated with a note.
const MAX_DIFF_DISPLAY_LINES: usize = 200;

/// Truncate a unified diff to a maximum number of lines if needed.
pub fn truncate_diff(unified: &str) -> String {
    let lines: Vec<&str> = unified.lines().collect();
    if lines.len() <= MAX_DIFF_DISPLAY_LINES {
        unified.to_string()
    } else {
        let truncated: String = lines[..MAX_DIFF_DISPLAY_LINES].join("\n");
        let remaining = lines.len() - MAX_DIFF_DISPLAY_LINES;
        format!("{truncated}\n... {remaining} more lines of diff truncated.")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_identical_content() {
        let content = "line1\nline2\nline3\n";
        let diff = compute_diff(content, content, "test.rs");
        assert_eq!(diff.insertions, 0);
        assert_eq!(diff.deletions, 0);
        assert_eq!(diff.changed_lines(), 0);
    }

    #[test]
    fn test_single_line_change() {
        let old = "line1\nline2\nline3\n";
        let new = "line1\nmodified\nline3\n";
        let diff = compute_diff(old, new, "test.rs");
        // A modification = 1 deletion + 1 insertion
        assert_eq!(diff.insertions, 1);
        assert_eq!(diff.deletions, 1);
        assert_eq!(diff.changed_lines(), 2);
    }

    #[test]
    fn test_pure_insertions() {
        let old = "line1\nline3\n";
        let new = "line1\nline2\nline3\n";
        let diff = compute_diff(old, new, "test.rs");
        assert_eq!(diff.insertions, 1);
        assert_eq!(diff.deletions, 0);
        assert_eq!(diff.changed_lines(), 1);
    }

    #[test]
    fn test_pure_deletions() {
        let old = "line1\nline2\nline3\n";
        let new = "line1\nline3\n";
        let diff = compute_diff(old, new, "test.rs");
        assert_eq!(diff.insertions, 0);
        assert_eq!(diff.deletions, 1);
        assert_eq!(diff.changed_lines(), 1);
    }

    #[test]
    fn test_empty_old_file() {
        let old = "";
        let new = "line1\nline2\nline3\n";
        let diff = compute_diff(old, new, "test.rs");
        assert_eq!(diff.insertions, 3);
        assert_eq!(diff.deletions, 0);
        assert_eq!(diff.old_line_count, 0);
        assert_eq!(diff.change_ratio(), 1.0); // empty file with insertions = 100% change
    }

    #[test]
    fn test_empty_new_file() {
        let old = "line1\nline2\nline3\n";
        let new = "";
        let diff = compute_diff(old, new, "test.rs");
        assert_eq!(diff.insertions, 0);
        assert_eq!(diff.deletions, 3);
        assert_eq!(diff.old_line_count, 3);
    }

    #[test]
    fn test_change_ratio() {
        let old = "a\nb\nc\nd\ne\n"; // 5 lines
        let new = "a\nX\nY\nd\ne\n"; // changed 2 of 5 lines
        let diff = compute_diff(old, new, "test.rs");
        // 2 deletions + 2 insertions = 4 changed lines
        // ratio = 4 / 5 = 0.8
        assert_eq!(diff.changed_lines(), 4);
        assert!((diff.change_ratio() - 0.8).abs() < f64::EPSILON);
    }

    #[test]
    fn test_summary_format() {
        let old = "a\nb\n";
        let new = "a\nc\nd\n";
        let diff = compute_diff(old, new, "test.rs");
        let summary = diff.summary();
        assert!(summary.starts_with('+'));
        assert!(summary.contains('-'));
    }

    #[test]
    fn test_unified_diff_has_headers() {
        let old = "line1\nline2\n";
        let new = "line1\nchanged\n";
        let diff = compute_diff(old, new, "src/main.rs");
        assert!(diff.unified.contains("src/main.rs (current)"));
        assert!(diff.unified.contains("src/main.rs (proposed)"));
    }

    #[test]
    fn test_threshold_floor() {
        let thresholds = WriteThresholds::default();
        // 5 changed lines (under floor of 10) -> DirectWrite
        let diff = DiffResult {
            insertions: 3,
            deletions: 2,
            old_line_count: 100,
            unified: String::new(),
        };
        assert!(matches!(thresholds.classify(&diff), WriteTier::DirectWrite));
    }

    #[test]
    fn test_threshold_ceiling() {
        let thresholds = WriteThresholds::default();
        // 85 changed lines (over ceiling of 80) -> StagedWrite
        let diff = DiffResult {
            insertions: 50,
            deletions: 35,
            old_line_count: 1000,
            unified: String::new(),
        };
        assert!(matches!(thresholds.classify(&diff), WriteTier::StagedWrite));
    }

    #[test]
    fn test_threshold_ratio_triggers() {
        let thresholds = WriteThresholds::default();
        // 30 changed lines in 50-line file = 60% > 40% -> StagedWrite
        let diff = DiffResult {
            insertions: 15,
            deletions: 15,
            old_line_count: 50,
            unified: String::new(),
        };
        assert!(matches!(thresholds.classify(&diff), WriteTier::StagedWrite));
    }

    #[test]
    fn test_threshold_ratio_passes() {
        let thresholds = WriteThresholds::default();
        // 30 changed lines in 500-line file = 6% < 40% -> DirectWrite
        let diff = DiffResult {
            insertions: 15,
            deletions: 15,
            old_line_count: 500,
            unified: String::new(),
        };
        assert!(matches!(thresholds.classify(&diff), WriteTier::DirectWrite));
    }

    #[test]
    fn test_threshold_at_floor_boundary() {
        let thresholds = WriteThresholds::default();
        // Exactly 10 changed lines (at floor) -> DirectWrite (<=)
        let diff = DiffResult {
            insertions: 5,
            deletions: 5,
            old_line_count: 20,
            unified: String::new(),
        };
        assert!(matches!(thresholds.classify(&diff), WriteTier::DirectWrite));
    }

    #[test]
    fn test_threshold_at_ceiling_boundary() {
        let thresholds = WriteThresholds::default();
        // Exactly 80 changed lines (at ceiling) -> StagedWrite (>=)
        let diff = DiffResult {
            insertions: 40,
            deletions: 40,
            old_line_count: 1000,
            unified: String::new(),
        };
        assert!(matches!(thresholds.classify(&diff), WriteTier::StagedWrite));
    }

    #[test]
    fn test_truncate_diff_short() {
        let short = "line1\nline2\nline3\n";
        assert_eq!(truncate_diff(short), short.to_string());
    }

    #[test]
    fn test_truncate_diff_long() {
        let lines: Vec<String> = (1..=300).map(|i| format!("line {i}")).collect();
        let long = lines.join("\n");
        let truncated = truncate_diff(&long);
        assert!(truncated.contains("100 more lines of diff truncated."));
        assert!(!truncated.contains("line 201\n"));
    }
}
