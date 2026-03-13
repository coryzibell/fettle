use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Backup metadata sidecar.
#[derive(Debug, Serialize, Deserialize)]
pub struct BackupMeta {
    pub original_path: String,
    pub created_at: String,
    pub size_bytes: u64,
}

/// Maximum number of backup files to keep.
const MAX_BACKUP_COUNT: usize = 100;

/// Maximum age of backup files in seconds (24 hours).
const MAX_BACKUP_AGE_SECS: u64 = 24 * 60 * 60;

/// Environment variable to override backup directory.
const BACKUP_DIR_ENV: &str = "FETTLE_BACKUP_DIR";

/// Get the backup directory path.
pub fn backup_dir() -> PathBuf {
    if let Ok(dir) = std::env::var(BACKUP_DIR_ENV) {
        return PathBuf::from(dir);
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home)
        .join(".wonka")
        .join("bench")
        .join("fettle")
        .join("backups")
}

/// Format a SystemTime as YYYYMMDD_HHMMSS_NNN.
fn format_timestamp(time: SystemTime) -> String {
    let duration = time.duration_since(UNIX_EPOCH).unwrap_or_default();
    let secs = duration.as_secs();
    let millis = duration.subsec_millis();

    let days = secs / 86400;
    let time_of_day = secs % 86400;
    let hours = time_of_day / 3600;
    let minutes = (time_of_day % 3600) / 60;
    let seconds = time_of_day % 60;

    let (year, month, day) = days_to_date(days);

    format!("{year:04}{month:02}{day:02}_{hours:02}{minutes:02}{seconds:02}_{millis:03}")
}

/// Convert days since Unix epoch to (year, month, day).
/// Algorithm from Howard Hinnant's date algorithms.
pub(crate) fn days_to_date(days: u64) -> (i64, u64, u64) {
    let z = days as i64 + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}

/// Format a SystemTime as ISO 8601 string.
fn format_iso8601(time: SystemTime) -> String {
    let duration = time.duration_since(UNIX_EPOCH).unwrap_or_default();
    let secs = duration.as_secs();
    let millis = duration.subsec_millis();

    let days = secs / 86400;
    let time_of_day = secs % 86400;
    let hours = time_of_day / 3600;
    let minutes = (time_of_day % 3600) / 60;
    let seconds = time_of_day % 60;

    let (year, month, day) = days_to_date(days);

    format!("{year:04}-{month:02}-{day:02}T{hours:02}:{minutes:02}:{seconds:02}.{millis:03}Z")
}

/// Result of creating a backup.
pub struct BackupResult {
    /// The backup file path.
    pub backup_path: PathBuf,
    /// Just the filename portion for display.
    pub backup_filename: String,
}

/// Create a backup of a file. Uses the default backup directory.
///
/// Returns the backup path on success, or None if the backup failed.
/// Backup failure is never fatal -- it logs a warning and moves on.
pub fn create_backup(original_path: &Path, content: &[u8]) -> Option<BackupResult> {
    create_backup_in(original_path, content, &backup_dir())
}

/// Create a backup of a file in a specific directory.
fn create_backup_in(original_path: &Path, content: &[u8], dir: &Path) -> Option<BackupResult> {
    // Create backup directory if needed
    if let Err(e) = fs::create_dir_all(dir) {
        eprintln!(
            "fettle: warning: cannot create backup dir {}: {e}",
            dir.display()
        );
        return None;
    }

    let filename = original_path
        .file_name()
        .map(|f| f.to_string_lossy().to_string())
        .unwrap_or_else(|| "unknown".to_string());

    let timestamp = format_timestamp(SystemTime::now());
    let backup_name = format!("{filename}.{timestamp}");
    let backup_path = dir.join(&backup_name);
    let meta_path = dir.join(format!("{backup_name}.meta"));

    // Write backup file
    if let Err(e) = fs::write(&backup_path, content) {
        eprintln!(
            "fettle: warning: cannot write backup {}: {e}",
            backup_path.display()
        );
        return None;
    }

    // Write metadata sidecar
    let meta = BackupMeta {
        original_path: original_path.to_string_lossy().to_string(),
        created_at: format_iso8601(SystemTime::now()),
        size_bytes: content.len() as u64,
    };

    if let Ok(json) = serde_json::to_string_pretty(&meta)
        && let Err(e) = fs::write(&meta_path, json)
    {
        eprintln!(
            "fettle: warning: cannot write backup metadata {}: {e}",
            meta_path.display()
        );
    }

    Some(BackupResult {
        backup_path,
        backup_filename: backup_name,
    })
}

/// Run opportunistic purge of old/excess backups.
///
/// Removes backups older than 24 hours and trims to 100 files max.
/// This is called before each backup creation.
pub fn purge_old_backups() {
    purge_old_backups_in(&backup_dir());
}

/// Purge old/excess backups in a specific directory.
fn purge_old_backups_in(dir: &Path) {
    if !dir.exists() {
        return;
    }

    let now = SystemTime::now();
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    // Collect all backup files (not .meta files)
    let mut backups: Vec<(PathBuf, SystemTime)> = Vec::new();

    for entry in entries.flatten() {
        let path = entry.path();
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();

        // Skip .meta files -- they are cleaned alongside their backup
        if name.ends_with(".meta") {
            continue;
        }

        let modified = entry.metadata().and_then(|m| m.modified()).unwrap_or(now);

        // Remove if older than 24h
        if let Ok(age) = now.duration_since(modified)
            && age.as_secs() > MAX_BACKUP_AGE_SECS
        {
            let _ = fs::remove_file(&path);
            let meta_path = PathBuf::from(format!("{}.meta", path.display()));
            let _ = fs::remove_file(&meta_path);
            continue;
        }

        backups.push((path, modified));
    }

    // Trim to max count (oldest first)
    if backups.len() > MAX_BACKUP_COUNT {
        backups.sort_by_key(|(_, time)| *time);
        let excess = backups.len() - MAX_BACKUP_COUNT;
        for (path, _) in backups.iter().take(excess) {
            let _ = fs::remove_file(path);
            let meta_path = PathBuf::from(format!("{}.meta", path.display()));
            let _ = fs::remove_file(&meta_path);
        }
    }
}

/// Restore a file from a backup.
///
/// Reads the .meta sidecar to determine the original path.
/// The `to_override` parameter allows overriding the restore target.
pub fn rollback(backup: &str, to_override: Option<&Path>) -> Result<String, String> {
    let dir = backup_dir();

    // Resolve backup path: full path or just filename
    let backup_path = if Path::new(backup).is_absolute() {
        PathBuf::from(backup)
    } else {
        dir.join(backup)
    };

    if !backup_path.exists() {
        return Err(format!(
            "fettle: No backup found at {}",
            backup_path.display()
        ));
    }

    // Determine restore target
    let target = if let Some(to) = to_override {
        to.to_path_buf()
    } else {
        // Read .meta sidecar
        let meta_path = PathBuf::from(format!("{}.meta", backup_path.display()));
        if !meta_path.exists() {
            return Err("fettle: Cannot rollback -- no metadata for this backup. \
                 Specify the target path: fettle rollback <backup> --to <path>"
                .to_string());
        }

        let meta_content = fs::read_to_string(&meta_path)
            .map_err(|e| format!("fettle: Cannot read backup metadata: {e}"))?;
        let meta: BackupMeta = serde_json::from_str(&meta_content)
            .map_err(|e| format!("fettle: Cannot parse backup metadata: {e}"))?;

        PathBuf::from(&meta.original_path)
    };

    // Read backup content
    let content =
        fs::read(&backup_path).map_err(|e| format!("fettle: Cannot read backup file: {e}"))?;

    // Create parent dirs if needed
    if let Some(parent) = target.parent()
        && !parent.exists()
    {
        fs::create_dir_all(parent).map_err(|e| format!("fettle: Cannot create directory: {e}"))?;
    }

    // Write to restore target
    fs::write(&target, &content)
        .map_err(|e| format!("fettle: Cannot write to {}: {e}", target.display()))?;

    let backup_filename = backup_path
        .file_name()
        .map(|f| f.to_string_lossy().to_string())
        .unwrap_or_else(|| backup.to_string());

    Ok(format!(
        "fettle: Restored {} from backup\n  backup: {backup_filename}",
        target.display()
    ))
}

/// List recent backups for status display.
pub fn list_recent_backups() -> Vec<(String, String, String)> {
    list_recent_backups_in(&backup_dir())
}

/// List recent backups in a specific directory.
fn list_recent_backups_in(dir: &Path) -> Vec<(String, String, String)> {
    if !dir.exists() {
        return Vec::new();
    }

    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return Vec::new(),
    };

    let now = SystemTime::now();
    let mut backups: Vec<(String, String, String, SystemTime)> = Vec::new();

    for entry in entries.flatten() {
        let path = entry.path();
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();

        if name.ends_with(".meta") {
            continue;
        }

        let modified = entry.metadata().and_then(|m| m.modified()).unwrap_or(now);

        let meta_path = PathBuf::from(format!("{}.meta", path.display()));
        let original = if let Ok(content) = fs::read_to_string(&meta_path) {
            serde_json::from_str::<BackupMeta>(&content)
                .map(|m| m.original_path)
                .unwrap_or_else(|_| "unknown".to_string())
        } else {
            "unknown".to_string()
        };

        let age = format_age(now, modified);
        backups.push((name, original, age, modified));
    }

    // Sort by time, newest first
    backups.sort_by(|a, b| b.3.cmp(&a.3));

    backups
        .into_iter()
        .map(|(name, original, age, _)| (name, original, age))
        .collect()
}

/// Format the age of a timestamp relative to now in human-readable form.
fn format_age(now: SystemTime, then: SystemTime) -> String {
    let secs = now.duration_since(then).map(|d| d.as_secs()).unwrap_or(0);

    if secs < 60 {
        format!("{secs} sec ago")
    } else if secs < 3600 {
        format!("{} min ago", secs / 60)
    } else {
        format!("{} hr ago", secs / 3600)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_backup_creates_file_and_sidecar() {
        let dir = TempDir::new().unwrap();
        let backup_d = dir.path().join("backups");

        let original = dir.path().join("source.rs");
        fs::write(&original, "fn main() {}").unwrap();

        let result = create_backup_in(&original, b"fn main() {}", &backup_d);
        assert!(result.is_some());

        let result = result.unwrap();
        assert!(result.backup_path.exists());
        assert!(result.backup_filename.starts_with("source.rs."));

        // Check sidecar exists
        let meta_path = PathBuf::from(format!("{}.meta", result.backup_path.display()));
        assert!(meta_path.exists());

        // Parse sidecar
        let meta_content = fs::read_to_string(&meta_path).unwrap();
        let meta: BackupMeta = serde_json::from_str(&meta_content).unwrap();
        assert_eq!(meta.original_path, original.to_string_lossy());
        assert_eq!(meta.size_bytes, 12);
    }

    #[test]
    fn test_backup_naming_includes_timestamp() {
        let dir = TempDir::new().unwrap();
        let backup_d = dir.path().join("backups");

        let original = dir.path().join("test.txt");
        let result = create_backup_in(&original, b"content", &backup_d);
        assert!(result.is_some());

        let name = result.unwrap().backup_filename;
        assert!(name.starts_with("test.txt."));
        let ts_part = &name["test.txt.".len()..];
        assert!(ts_part.contains('_'));
    }

    #[test]
    fn test_purge_respects_max_count() {
        let dir = TempDir::new().unwrap();
        let backup_d = dir.path().join("backups");
        fs::create_dir_all(&backup_d).unwrap();

        // Create 105 backup files (over the 100 limit)
        for i in 0..105 {
            let name = format!("file.txt.20260312_000000_{i:03}");
            fs::write(backup_d.join(&name), format!("backup {i}")).unwrap();
        }

        purge_old_backups_in(&backup_d);

        let remaining: Vec<_> = fs::read_dir(&backup_d)
            .unwrap()
            .flatten()
            .filter(|e| !e.path().to_string_lossy().ends_with(".meta"))
            .collect();
        assert!(remaining.len() <= MAX_BACKUP_COUNT);
    }

    #[test]
    fn test_rollback_restores_from_backup() {
        let dir = TempDir::new().unwrap();
        let backup_d = dir.path().join("backups");

        let original = dir.path().join("original.txt");
        fs::write(&original, "current content").unwrap();

        let result = create_backup_in(&original, b"old content", &backup_d).unwrap();

        // Overwrite the original
        fs::write(&original, "new content").unwrap();

        // Rollback using full path
        let msg = rollback(result.backup_path.to_str().unwrap(), None).unwrap();
        assert!(msg.contains("Restored"));
        assert_eq!(fs::read_to_string(&original).unwrap(), "old content");
    }

    #[test]
    fn test_rollback_with_to_override() {
        let dir = TempDir::new().unwrap();
        let backup_d = dir.path().join("backups");

        let original = dir.path().join("original.txt");
        let alt_target = dir.path().join("alternative.txt");

        let result = create_backup_in(&original, b"backup content", &backup_d).unwrap();

        let msg = rollback(result.backup_path.to_str().unwrap(), Some(&alt_target)).unwrap();
        assert!(msg.contains("Restored"));
        assert_eq!(fs::read_to_string(&alt_target).unwrap(), "backup content");
    }

    #[test]
    fn test_rollback_missing_sidecar_without_to() {
        let dir = TempDir::new().unwrap();

        let backup_path = dir.path().join("orphan.txt.20260312_000000_000");
        fs::write(&backup_path, "orphan content").unwrap();

        let result = rollback(backup_path.to_str().unwrap(), None);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("no metadata"));
    }

    #[test]
    fn test_rollback_by_full_path() {
        let dir = TempDir::new().unwrap();
        let backup_d = dir.path().join("backups");

        let original = dir.path().join("target.txt");
        let result = create_backup_in(&original, b"backed up", &backup_d).unwrap();

        let msg = rollback(result.backup_path.to_str().unwrap(), Some(&original)).unwrap();
        assert!(msg.contains("Restored"));
        assert_eq!(fs::read_to_string(&original).unwrap(), "backed up");
    }

    #[test]
    fn test_rollback_nonexistent_backup() {
        let result = rollback("/nonexistent/backup.txt", None);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("No backup found"));
    }

    #[test]
    fn test_format_age() {
        let now = SystemTime::now();

        let then = now - std::time::Duration::from_secs(30);
        assert!(format_age(now, then).contains("sec ago"));

        let then = now - std::time::Duration::from_secs(120);
        assert!(format_age(now, then).contains("min ago"));

        let then = now - std::time::Duration::from_secs(7200);
        assert!(format_age(now, then).contains("hr ago"));
    }

    #[test]
    fn test_days_to_date() {
        assert_eq!(days_to_date(0), (1970, 1, 1));
        let (y, m, d) = days_to_date(20524);
        assert_eq!(y, 2026);
        assert_eq!(m, 3);
        assert_eq!(d, 12);
    }
}
