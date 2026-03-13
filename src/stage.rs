use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

/// Default TTL for staged sessions in seconds (10 minutes).
const DEFAULT_STAGE_TTL_SECS: u64 = 600;

/// Environment variable to override stage directory.
const STAGE_DIR_ENV: &str = "FETTLE_STAGE_DIR";

/// Environment variable to override stage TTL.
const STAGE_TTL_ENV: &str = "FETTLE_WRITE_STAGE_TTL";

/// Status of a staged session.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SessionStatus {
    Pending,
    Applied,
    Discarded,
    Expired,
}

/// A staged write session, persisted as metadata.json.
#[derive(Debug, Serialize, Deserialize)]
pub struct StagedSession {
    pub session_id: String,
    pub target_path: String,
    pub backup_path: Option<String>,
    pub created_at: String,
    pub diff_summary: String,
    pub status: SessionStatus,
}

/// Get the staging directory path.
pub fn stage_dir() -> PathBuf {
    if let Ok(dir) = std::env::var(STAGE_DIR_ENV) {
        return PathBuf::from(dir);
    }
    PathBuf::from("/tmp/fettle-stage")
}

/// Get the configured TTL in seconds.
fn stage_ttl_secs() -> u64 {
    std::env::var(STAGE_TTL_ENV)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_STAGE_TTL_SECS)
}

/// Generate a session ID, checking for collisions against a specific directory.
///
/// Uses XOR-folding of timestamp nanos, path bytes, and process ID to produce
/// a 32-bit value, formatted as 8 hex characters. Not cryptographically random
/// -- just unique enough to avoid collisions in practice.
///
/// If the resulting session directory already exists (e.g., two writes to the
/// same file in the same nanosecond), the hash is incremented until a free
/// slot is found.
fn generate_session_id_in(file_path: &str, base_dir: &Path) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();

    let pid = std::process::id();

    let mut hash: u32 = (nanos & 0xFFFFFFFF) as u32 ^ ((nanos >> 32) & 0xFFFFFFFF) as u32;
    hash = hash.wrapping_mul(31).wrapping_add(pid);

    for byte in file_path.bytes() {
        hash = hash.wrapping_mul(31).wrapping_add(byte as u32);
    }

    // Check for collision: if the directory already exists, increment until free.
    loop {
        let id = format!("{hash:08x}");
        if !base_dir.join(&id).exists() {
            return id;
        }
        hash = hash.wrapping_add(1);
    }
}

/// Parse an ISO 8601 timestamp string back to epoch seconds.
fn parse_iso8601_epoch_secs(s: &str) -> Option<u64> {
    let parts: Vec<&str> = s.split('T').collect();
    if parts.len() != 2 {
        return None;
    }

    let date_parts: Vec<u64> = parts[0].split('-').filter_map(|p| p.parse().ok()).collect();
    let time_str = parts[1].trim_end_matches('Z');
    let time_main: Vec<&str> = time_str.split('.').collect();
    let time_parts: Vec<u64> = time_main[0]
        .split(':')
        .filter_map(|p| p.parse().ok())
        .collect();

    if date_parts.len() != 3 || time_parts.len() != 3 {
        return None;
    }

    let year = date_parts[0] as i64;
    let month = date_parts[1];
    let day = date_parts[2];
    let hours = time_parts[0];
    let minutes = time_parts[1];
    let seconds = time_parts[2];

    let days = date_to_days(year, month, day);
    Some(days * 86400 + hours * 3600 + minutes * 60 + seconds)
}

/// Convert (year, month, day) to days since Unix epoch.
fn date_to_days(year: i64, month: u64, day: u64) -> u64 {
    let y = if month <= 2 { year - 1 } else { year };
    let m = if month <= 2 { month + 9 } else { month - 3 };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = (y - era * 400) as u64;
    let doy = (153 * m + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    (era as u64).wrapping_mul(146097).wrapping_add(doe) - 719468
}

/// Stage proposed content for a Tier 2 write. Uses the default stage directory.
pub fn stage_write(
    file_path: &str,
    content: &str,
    backup_path: Option<&str>,
    diff_summary: &str,
) -> Result<String, String> {
    stage_write_in(file_path, content, backup_path, diff_summary, &stage_dir())
}

/// Stage proposed content in a specific directory.
fn stage_write_in(
    file_path: &str,
    content: &str,
    backup_path: Option<&str>,
    diff_summary: &str,
    base_dir: &Path,
) -> Result<String, String> {
    let session_id = generate_session_id_in(file_path, base_dir);
    let dir = base_dir.join(&session_id);

    fs::create_dir_all(&dir).map_err(|e| {
        format!(
            "fettle: Cannot create staging directory {}: {e}",
            dir.display()
        )
    })?;

    // Restrict permissions: staged content may contain secrets
    #[cfg(unix)]
    {
        let _ = fs::set_permissions(base_dir, fs::Permissions::from_mode(0o700));
        let _ = fs::set_permissions(&dir, fs::Permissions::from_mode(0o700));
    }

    // Write proposed content
    let content_path = dir.join("content");
    fs::write(&content_path, content)
        .map_err(|e| format!("fettle: Cannot write staged content: {e}"))?;

    // Write metadata
    let session = StagedSession {
        session_id: session_id.clone(),
        target_path: file_path.to_string(),
        backup_path: backup_path.map(|s| s.to_string()),
        created_at: crate::backup::format_iso8601(SystemTime::now()),
        diff_summary: diff_summary.to_string(),
        status: SessionStatus::Pending,
    };

    let metadata_path = dir.join("metadata.json");
    let json = serde_json::to_string_pretty(&session)
        .map_err(|e| format!("fettle: Cannot serialize session metadata: {e}"))?;
    fs::write(&metadata_path, json)
        .map_err(|e| format!("fettle: Cannot write session metadata: {e}"))?;

    Ok(session_id)
}

/// Load a staged session's metadata from a specific directory.
fn load_session_in(session_id: &str, base_dir: &Path) -> Result<(StagedSession, PathBuf), String> {
    let dir = base_dir.join(session_id);

    if !dir.exists() {
        return Err(format!(
            "fettle: No session '{session_id}'. It may have expired or been discarded."
        ));
    }

    let metadata_path = dir.join("metadata.json");
    if !metadata_path.exists() {
        return Err(format!("fettle: Corrupted session '{session_id}'."));
    }

    let content = fs::read_to_string(&metadata_path)
        .map_err(|e| format!("fettle: Cannot read session metadata: {e}"))?;

    let session: StagedSession = serde_json::from_str(&content)
        .map_err(|e| format!("fettle: Cannot parse session metadata: {e}"))?;

    Ok((session, dir))
}

/// Check if a session has expired based on configured TTL.
fn is_expired(session: &StagedSession) -> Option<u64> {
    is_expired_with_ttl(session, stage_ttl_secs())
}

/// Check if a session has expired with a specific TTL.
fn is_expired_with_ttl(session: &StagedSession, ttl: u64) -> Option<u64> {
    let created_epoch = parse_iso8601_epoch_secs(&session.created_at)?;
    let now_epoch = SystemTime::now().duration_since(UNIX_EPOCH).ok()?.as_secs();

    let age = now_epoch.saturating_sub(created_epoch);

    if age > ttl { Some(age / 60) } else { None }
}

/// Confirm and apply a staged write.
pub fn confirm(session_id: &str) -> Result<String, String> {
    confirm_in(session_id, &stage_dir())
}

/// Confirm and apply a staged write from a specific directory.
fn confirm_in(session_id: &str, base_dir: &Path) -> Result<String, String> {
    let (mut session, dir) = load_session_in(session_id, base_dir)?;

    // Check status
    match session.status {
        SessionStatus::Applied => {
            return Err(format!("fettle: Session {session_id} was already applied."));
        }
        SessionStatus::Discarded => {
            return Err(format!(
                "fettle: Session {session_id} was already discarded."
            ));
        }
        SessionStatus::Expired => {
            return Err(format!(
                "fettle: Session {session_id} has expired. Re-run the Write to try again."
            ));
        }
        SessionStatus::Pending => {}
    }

    // Check expiry
    if let Some(minutes) = is_expired(&session) {
        session.status = SessionStatus::Expired;
        let _ = save_session_metadata(&session, &dir);
        return Err(format!(
            "fettle: Session {session_id} expired {minutes} minutes ago. Re-run the Write to try again."
        ));
    }

    // Read staged content
    let content_path = dir.join("content");
    let content = fs::read_to_string(&content_path)
        .map_err(|e| format!("fettle: Cannot read staged content: {e}"))?;

    // Write to target
    let target = PathBuf::from(&session.target_path);
    if let Some(parent) = target.parent()
        && !parent.exists()
    {
        fs::create_dir_all(parent).map_err(|e| format!("fettle: Cannot create directory: {e}"))?;
    }

    fs::write(&target, &content)
        .map_err(|e| format!("fettle: Failed to apply session {session_id}: {e}"))?;

    // Update status
    session.status = SessionStatus::Applied;
    let _ = save_session_metadata(&session, &dir);

    Ok(format!(
        "fettle: Applied staged write to {}\n  {} lines changed\n  session {session_id} complete",
        session.target_path, session.diff_summary
    ))
}

/// Discard a staged write.
pub fn discard(session_id: &str) -> Result<String, String> {
    discard_in(session_id, &stage_dir())
}

/// Discard a staged write from a specific directory.
fn discard_in(session_id: &str, base_dir: &Path) -> Result<String, String> {
    let (session, dir) = load_session_in(session_id, base_dir)?;

    if session.status == SessionStatus::Applied {
        return Err(format!("fettle: Session {session_id} was already applied."));
    }

    let target_path = session.target_path.clone();
    fs::remove_dir_all(&dir)
        .map_err(|e| format!("fettle: Cannot remove staging directory: {e}"))?;

    Ok(format!(
        "fettle: Discarded staged write for {target_path}\n  session {session_id} removed"
    ))
}

/// Save updated session metadata.
fn save_session_metadata(session: &StagedSession, dir: &Path) -> Result<(), String> {
    let metadata_path = dir.join("metadata.json");
    let json =
        serde_json::to_string_pretty(session).map_err(|e| format!("Cannot serialize: {e}"))?;
    fs::write(&metadata_path, json).map_err(|e| format!("Cannot write: {e}"))?;
    Ok(())
}

/// Purge expired staged sessions.
pub fn purge_expired_sessions() {
    purge_expired_sessions_in(&stage_dir(), stage_ttl_secs());
}

/// Purge expired staged sessions in a specific directory with specific TTL.
fn purge_expired_sessions_in(dir: &Path, ttl: u64) {
    if !dir.exists() {
        return;
    }

    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    let now = SystemTime::now();

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }

        let metadata_path = path.join("metadata.json");
        if !metadata_path.exists() {
            let _ = fs::remove_dir_all(&path);
            continue;
        }

        if let Ok(metadata) = entry.metadata()
            && let Ok(modified) = metadata.modified()
            && let Ok(age) = now.duration_since(modified)
            && age.as_secs() >= ttl
        {
            let _ = fs::remove_dir_all(&path);
        }
    }
}

/// Summary of a pending staged session for display.
pub struct PendingSession {
    pub session_id: String,
    pub target_path: String,
    pub diff_summary: String,
    pub age: String,
}

/// List pending staged sessions for status display.
pub fn list_pending_sessions() -> Vec<PendingSession> {
    list_pending_sessions_in(&stage_dir())
}

/// List pending staged sessions in a specific directory.
fn list_pending_sessions_in(dir: &Path) -> Vec<PendingSession> {
    if !dir.exists() {
        return Vec::new();
    }

    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return Vec::new(),
    };

    let now = SystemTime::now();
    let mut sessions: Vec<(PendingSession, SystemTime)> = Vec::new();

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }

        let metadata_path = path.join("metadata.json");
        if let Ok(content) = fs::read_to_string(&metadata_path)
            && let Ok(session) = serde_json::from_str::<StagedSession>(&content)
        {
            if session.status != SessionStatus::Pending {
                continue;
            }

            let modified = entry.metadata().and_then(|m| m.modified()).unwrap_or(now);

            let age = now
                .duration_since(modified)
                .map(|d| {
                    let secs = d.as_secs();
                    if secs < 60 {
                        format!("{secs} sec ago")
                    } else {
                        format!("{} min ago", secs / 60)
                    }
                })
                .unwrap_or_else(|_| "unknown".to_string());

            sessions.push((
                PendingSession {
                    session_id: session.session_id,
                    target_path: session.target_path,
                    diff_summary: session.diff_summary,
                    age,
                },
                modified,
            ));
        }
    }

    sessions.sort_by(|a, b| b.1.cmp(&a.1));

    sessions.into_iter().map(|(s, _)| s).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_session_id_is_8_char_hex() {
        let dir = TempDir::new().unwrap();
        let id = generate_session_id_in("/home/user/test.rs", dir.path());
        assert_eq!(id.len(), 8);
        assert!(id.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn test_stage_creates_content_and_metadata() {
        let dir = TempDir::new().unwrap();
        let stage_d = dir.path().join("stage");

        let session_id = stage_write_in(
            "/home/user/test.rs",
            "fn main() {}",
            Some("/backup/path"),
            "+5 -3",
            &stage_d,
        )
        .unwrap();

        let session_dir = stage_d.join(&session_id);
        assert!(session_dir.exists());
        assert!(session_dir.join("content").exists());
        assert!(session_dir.join("metadata.json").exists());

        let content = fs::read_to_string(session_dir.join("content")).unwrap();
        assert_eq!(content, "fn main() {}");

        let meta_str = fs::read_to_string(session_dir.join("metadata.json")).unwrap();
        let meta: StagedSession = serde_json::from_str(&meta_str).unwrap();
        assert_eq!(meta.session_id, session_id);
        assert_eq!(meta.target_path, "/home/user/test.rs");
        assert_eq!(meta.status, SessionStatus::Pending);
        assert_eq!(meta.diff_summary, "+5 -3");
    }

    #[test]
    fn test_confirm_applies_content() {
        let dir = TempDir::new().unwrap();
        let stage_d = dir.path().join("stage");
        let target = dir.path().join("target.rs");

        let session_id = stage_write_in(
            target.to_str().unwrap(),
            "fn main() { println!(\"hello\"); }",
            None,
            "+1 -0",
            &stage_d,
        )
        .unwrap();

        let msg = confirm_in(&session_id, &stage_d).unwrap();
        assert!(msg.contains("Applied staged write"));
        assert_eq!(
            fs::read_to_string(&target).unwrap(),
            "fn main() { println!(\"hello\"); }"
        );
    }

    #[test]
    fn test_confirm_already_applied() {
        let dir = TempDir::new().unwrap();
        let stage_d = dir.path().join("stage");
        let target = dir.path().join("target.rs");

        let session_id =
            stage_write_in(target.to_str().unwrap(), "content", None, "+1 -0", &stage_d).unwrap();

        let _ = confirm_in(&session_id, &stage_d).unwrap();
        let result = confirm_in(&session_id, &stage_d);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("already applied"));
    }

    #[test]
    fn test_discard_removes_session() {
        let dir = TempDir::new().unwrap();
        let stage_d = dir.path().join("stage");

        let session_id =
            stage_write_in("/tmp/test.rs", "content", None, "+1 -0", &stage_d).unwrap();

        let msg = discard_in(&session_id, &stage_d).unwrap();
        assert!(msg.contains("Discarded"));

        let session_dir = stage_d.join(&session_id);
        assert!(!session_dir.exists());
    }

    #[test]
    fn test_confirm_expired_session() {
        let dir = TempDir::new().unwrap();
        let stage_d = dir.path().join("stage");

        let session_id =
            stage_write_in("/tmp/test.rs", "content", None, "+1 -0", &stage_d).unwrap();

        // Manually set the created_at to the past to simulate expiry
        let session_dir = stage_d.join(&session_id);
        let meta_path = session_dir.join("metadata.json");
        let meta_str = fs::read_to_string(&meta_path).unwrap();
        let mut session: StagedSession = serde_json::from_str(&meta_str).unwrap();
        session.created_at = "2020-01-01T00:00:00.000Z".to_string();
        let json = serde_json::to_string_pretty(&session).unwrap();
        fs::write(&meta_path, json).unwrap();

        let result = confirm_in(&session_id, &stage_d);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("expired"));
    }

    #[test]
    fn test_unknown_session() {
        let dir = TempDir::new().unwrap();

        let result = confirm_in("nonexist", dir.path());
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("No session"));
    }

    #[test]
    fn test_purge_removes_expired_sessions() {
        let dir = TempDir::new().unwrap();
        let stage_d = dir.path().join("stage");

        let _ = stage_write_in("/tmp/test1.rs", "content1", None, "+1 -0", &stage_d).unwrap();
        let _ = stage_write_in("/tmp/test2.rs", "content2", None, "+2 -0", &stage_d).unwrap();

        // Purge with TTL of 0 -- everything is expired
        purge_expired_sessions_in(&stage_d, 0);

        let remaining: Vec<_> = fs::read_dir(&stage_d)
            .unwrap()
            .flatten()
            .filter(|e| e.path().is_dir())
            .collect();
        assert_eq!(remaining.len(), 0);
    }

    #[test]
    fn test_list_pending_sessions() {
        let dir = TempDir::new().unwrap();
        let stage_d = dir.path().join("stage");

        let _ = stage_write_in("/tmp/test1.rs", "content1", None, "+1 -0", &stage_d).unwrap();
        let _ = stage_write_in("/tmp/test2.rs", "content2", None, "+2 -0", &stage_d).unwrap();

        let sessions = list_pending_sessions_in(&stage_d);
        assert_eq!(sessions.len(), 2);
    }

    #[test]
    fn test_discard_expired_session_succeeds() {
        let dir = TempDir::new().unwrap();
        let stage_d = dir.path().join("stage");

        let session_id =
            stage_write_in("/tmp/test.rs", "content", None, "+1 -0", &stage_d).unwrap();

        // Manually expire the session
        let session_dir = stage_d.join(&session_id);
        let meta_path = session_dir.join("metadata.json");
        let meta_str = fs::read_to_string(&meta_path).unwrap();
        let mut session: StagedSession = serde_json::from_str(&meta_str).unwrap();
        session.created_at = "2020-01-01T00:00:00.000Z".to_string();
        let json = serde_json::to_string_pretty(&session).unwrap();
        fs::write(&meta_path, json).unwrap();

        // Discarding an expired session should succeed (harmless cleanup)
        let msg = discard_in(&session_id, &stage_d).unwrap();
        assert!(msg.contains("Discarded"));
        assert!(!session_dir.exists());
    }

    #[test]
    fn test_session_id_collision_avoidance() {
        let dir = TempDir::new().unwrap();
        let stage_d = dir.path().join("stage");
        fs::create_dir_all(&stage_d).unwrap();

        // Generate a session ID, then create its directory to simulate a collision
        let id1 = generate_session_id_in("/tmp/test.rs", &stage_d);
        fs::create_dir_all(stage_d.join(&id1)).unwrap();

        // Next generation for the same path should produce a different ID
        let id2 = generate_session_id_in("/tmp/test.rs", &stage_d);
        assert_ne!(id1, id2);
    }
}
