# Fettle Enhanced Write Protocol -- Architectural Blueprint

*Drafted by Drafty von Blueprint. The napkin is the blueprint.*

---

## 1. System Overview

Fettle currently writes files unconditionally on every Write hook interception.
The enhanced protocol introduces **awareness of what changed** and **protection
against large unreviewed mutations**. Two tiers, one goal: the model always
knows exactly what happened, and large changes get a human-visible diff before
they land.

```
                      Write hook fires
                            |
                    [extract path + content]
                            |
                    [file exists on disk?]
                     /              \
                   NO               YES
                   |                 |
             [new file]        [compute diff]
             write direct       /          \
             return ok      SMALL           LARGE
                            diff            diff
                             |               |
                        [backup]         [backup]
                        [write]          [stage only]
                        [return          [return diff
                         summary]         + session ID
                                          + confirm
                                          instructions]
```

---

## 2. Threshold Design

### The Metric: Changed Lines

Count the number of lines that are insertions or deletions in the diff. A
changed line (modification) counts as 2 (one deletion + one insertion), which
is how unified diffs naturally represent them.

### The Threshold: Adaptive, Not Fixed

A flat "20 lines" threshold is wrong. Rewriting 20 of 25 lines in a file is
a total rewrite. Changing 20 lines in a 2,000-line file is a surgical edit.
Both should be treated differently.

**Rule:**

```
tier = if changed_lines <= ABSOLUTE_FLOOR -> Tier 1 (always small)
       if changed_lines >= ABSOLUTE_CEIL  -> Tier 2 (always large)
       if changed_lines / total_lines > RATIO_THRESHOLD -> Tier 2
       else -> Tier 1
```

**Default values:**

| Parameter | Value | Env Override |
|-----------|-------|-------------|
| `ABSOLUTE_FLOOR` | 10 lines | `FETTLE_WRITE_FLOOR` |
| `ABSOLUTE_CEIL` | 80 lines | `FETTLE_WRITE_CEIL` |
| `RATIO_THRESHOLD` | 0.40 (40%) | `FETTLE_WRITE_RATIO` |

Rationale:
- Under 10 changed lines: always safe to apply directly. This covers the vast
  majority of edits (fix a typo, add an import, tweak a constant).
- Over 80 changed lines: always stage for review regardless of file size.
  This is a substantial rewrite.
- Between 10-80: check the ratio. Changing 30 lines in a 50-line file (60%)
  triggers staging. Changing 30 lines in a 500-line file (6%) does not.
- 40% ratio threshold: if you are rewriting more than 40% of a file, the
  model should see the diff before the write lands.

**New file bypass:** When a file does not exist on disk, there is no diff to
compute and nothing to protect. Write directly. Always Tier 1.

---

## 3. Diff Computation

### Approach: In-Process, No External Dependencies

Use the `similar` crate (MIT licensed, pure Rust, no dependencies worth
worrying about). It provides line-level diffs with unified diff output.
No shelling out to `diff(1)` -- that adds process overhead on every write
hook and introduces a platform dependency.

**Crate:** `similar = "2"` added to `[dependencies]` in Cargo.toml.

### Diff Algorithm

`similar` uses the Myers diff algorithm by default, which is the same
algorithm used by `git diff`. This is correct for our use case.

### Computing the Diff

```rust
// Pseudostructure, not final code
fn compute_diff(old: &str, new: &str) -> DiffResult {
    let diff = similar::TextDiff::from_lines(old, new);
    let stats = DiffStats {
        insertions: /* count of Insert changes */,
        deletions:  /* count of Delete changes */,
        total_old_lines: old.lines().count(),
    };
    let unified = diff.unified_diff()
        .context_radius(3)
        .header(/* old path, new path */)
        .to_string();
    DiffResult { stats, unified }
}
```

### What Gets Shown to Claude

**Tier 1 (small, applied immediately):** A brief summary line, not the full
diff. Token-efficient. The model already knows what it wrote -- it just needs
confirmation that the write landed.

```
fettle: Wrote /path/to/file.rs (2.1KB, 45 lines, +3 -2 ~1 changed)
  backup: ~/.wonka/bench/fettle/backups/file.rs.20260312T143022
```

Format: `+N` insertions, `-N` deletions, `~N` is a convenience count of
modified lines (min of insertions, deletions -- lines that were changed
rather than purely added/removed). This is compact and informative.

**Tier 2 (large, staged for confirmation):** The full unified diff, plus
instructions. The diff IS the value here -- the model needs to see what
will change without re-reading the entire file.

```
fettle: Staged write for /path/to/file.rs (session: a1b2c3)
  +47 -32 lines changed (62% of file)
  backup: ~/.wonka/bench/fettle/backups/file.rs.20260312T143022

--- /path/to/file.rs (current)
+++ /path/to/file.rs (proposed)
@@ -10,7 +10,9 @@
 unchanged context
-old line
+new line
+another new line
 more context
...

To apply: run `fettle confirm a1b2c3` via Bash
To discard: run `fettle discard a1b2c3` via Bash
```

### Diff Context Radius

3 lines of context around each hunk. This is the git default and provides
enough orientation without bloating the output. The `similar` crate supports
configurable context radius natively.

### Binary Files

If the file exists and its content is not valid UTF-8, skip diffing entirely.
Write directly (Tier 1 behavior) with a note: `fettle: Wrote binary file
/path/to/file (N bytes)`. Binary diffs are meaningless to the model. No
backup for binary files -- they are typically generated artifacts.

Actually, reconsider: binary files written by Claude Code are rare and
usually intentional (e.g., writing a small binary test fixture). Backing
them up is cheap. Do back them up if the file previously existed.

---

## 4. Backup System

### Location

```
~/.wonka/bench/fettle/backups/
```

This follows the factory's bench tier (session scratch, cleared between
sessions). Backups are ephemeral protection, not version control.

### Naming Scheme

```
{filename}.{timestamp}
```

Where timestamp is `YYYYMMDD_HHMMSS_NNN` (NNN = milliseconds for uniqueness
within the same second). The filename is the basename only -- the full
original path is stored in a sidecar metadata file.

Example:
```
~/.wonka/bench/fettle/backups/
  main.rs.20260312_143022_471
  main.rs.20260312_143022_471.meta    <- JSON: {"original": "/home/user/project/src/main.rs"}
```

The `.meta` sidecar is tiny (one JSON object) and enables rollback without
guessing which `/src/main.rs` this backup belongs to.

### Retention Policy

- **Max age:** 24 hours. Backups older than 24h are deleted on next write.
- **Max count:** 100 backups total. If exceeded, oldest are purged first.
- **Purge trigger:** Every write operation. Before creating a new backup,
  scan the backup directory and remove stale entries. This is a fast `readdir`
  + timestamp parse -- no meaningful overhead.
- **No background daemon.** Purging is opportunistic, happens inline during
  write operations. If fettle is never called again, old backups sit harmlessly
  until manually cleaned.

### Backup Flow

1. Check if file exists on disk.
2. If yes: read current content, write to backup path, write `.meta` sidecar.
3. If backup write fails: log warning but proceed with the write. Backup
   failure should never block a write. The backup is a safety net, not a gate.

---

## 5. Staging System (Tier 2)

### Location

```
/tmp/fettle-stage/
```

Using `/tmp` because staged content is truly ephemeral -- it matters for
minutes, not hours. System temp cleanup handles abandoned sessions. This also
avoids cluttering user home directories with transient data.

### Session Structure

```
/tmp/fettle-stage/
  {session-id}/
    content          <- the proposed file content
    metadata.json    <- session metadata
```

### Session ID

8-character lowercase hex string derived from the current timestamp and file
path. Not cryptographically random -- just unique enough to avoid collisions
within a single user's concurrent sessions.

Generation: hash (e.g., FNV or simple truncated hash) of
`"{timestamp_nanos}:{file_path}"`, take first 8 hex chars. No external crate
needed -- Rust's standard library can do this with basic byte manipulation,
or use the `fastrand` crate already implied by the dependency tree.

Actually, looking at Cargo.toml, there are no random crates in deps. Use
timestamp nanos + path bytes, XOR-fold to 32 bits, format as hex. Simple,
deterministic-enough, no new dependencies.

### Metadata

```json
{
  "session_id": "a1b2c3d4",
  "target_path": "/home/user/project/src/main.rs",
  "backup_path": "~/.wonka/bench/fettle/backups/main.rs.20260312_143022_471",
  "created_at": "2026-03-12T14:30:22.471Z",
  "diff_summary": "+47 -32",
  "status": "pending"
}
```

Status values: `pending`, `applied`, `discarded`, `expired`.

### Expiry

- **TTL:** 10 minutes. If not confirmed within 10 minutes, the session is
  considered abandoned.
- **Expiry check:** On `fettle confirm` or `fettle discard`, check timestamp.
  If expired, return an error: `"Session a1b2c3d4 expired (created 12 minutes
  ago). Re-run the Write to generate a new session."`.
- **Cleanup:** On every staging operation, scan `/tmp/fettle-stage/` and
  remove directories older than 10 minutes. Same opportunistic purge strategy
  as backups.

### Concurrency

What if two writes target the same file simultaneously? This is unlikely in
practice (Claude Code is single-threaded in its tool dispatch) but must be
handled:

- Each session has a unique ID tied to its timestamp. Two writes to the same
  file produce two sessions. The backup taken by the second write captures
  whatever the first write left behind. This is correct -- each session's
  backup reflects the state at the moment that session was created.
- `fettle confirm` reads the staged content and applies it atomically
  (write to temp file in same directory, then rename). If the file changed
  between staging and confirmation, that is the human/model's problem to
  detect -- fettle is not a merge tool.

---

## 6. CLI Subcommands

### Existing Commands (unchanged)

```
fettle read <file> [--offset N] [--limit N]
fettle write <file>            # reads content from stdin
fettle install
fettle info
```

### New Commands

#### `fettle confirm <session-id>`

Apply a staged write.

```
$ fettle confirm a1b2c3d4
fettle: Applied staged write to /home/user/project/src/main.rs
  +47 -32 lines changed
  session a1b2c3d4 complete
```

Error cases:
- Unknown session: `"fettle: No session 'xyz'. It may have expired or been discarded."`
- Expired session: `"fettle: Session a1b2c3d4 expired 8 minutes ago. Re-run the Write to try again."`
- Already applied: `"fettle: Session a1b2c3d4 was already applied."`
- Write failure: `"fettle: Failed to apply session a1b2c3d4: permission denied"` (the staged content is preserved so the user can retry or recover).

#### `fettle discard <session-id>`

Discard a staged write without applying.

```
$ fettle discard a1b2c3d4
fettle: Discarded staged write for /home/user/project/src/main.rs
  session a1b2c3d4 removed
```

This removes the staged content. The backup remains (backups have their own
retention policy independent of sessions).

#### `fettle rollback <backup-path-or-pattern>`

Restore a file from backup. This is intentionally explicit -- you provide the
backup file path (shown in write confirmation messages) or a glob pattern.

```
$ fettle rollback ~/.wonka/bench/fettle/backups/main.rs.20260312_143022_471
fettle: Restored /home/user/project/src/main.rs from backup
  backup: main.rs.20260312_143022_471
```

Also accept a shorthand: just the backup filename without the full path.

```
$ fettle rollback main.rs.20260312_143022_471
fettle: Restored /home/user/project/src/main.rs from backup
```

This reads the `.meta` sidecar to find the original path. If the sidecar is
missing, error: `"fettle: Cannot rollback -- no metadata for this backup.
Specify the target path: fettle rollback <backup> --to <path>"`.

Add `--to <path>` flag to override the restore target (useful if the original
path no longer makes sense).

#### `fettle status`

Show pending staged sessions and recent backups.

```
$ fettle status
Pending staged writes:
  a1b2c3d4  /home/user/src/main.rs     +47 -32   2 min ago
  e5f6a7b8  /home/user/src/lib.rs      +12 -3    45 sec ago

Recent backups (last 24h):
  main.rs.20260312_143022_471      /home/user/src/main.rs     2 min ago
  lib.rs.20260312_142955_102       /home/user/src/lib.rs      3 min ago
```

This is a diagnostic command. Useful when Claude or the user wants to see
what is pending or what can be rolled back.

---

## 7. Data Structures

### New Structs

```rust
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
    /// Returns 0.0 for empty files (prevents division by zero).
    pub fn change_ratio(&self) -> f64 {
        if self.old_line_count == 0 {
            0.0
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
    /// New file, no existing content.
    NewFile,
    /// Small diff -- apply immediately.
    DirectWrite { diff: DiffResult },
    /// Large diff -- stage for confirmation.
    StagedWrite { diff: DiffResult },
}

/// A staged write session, persisted as metadata.json.
#[derive(Serialize, Deserialize)]
pub struct StagedSession {
    pub session_id: String,
    pub target_path: String,
    pub backup_path: Option<String>,
    pub created_at: String,  // ISO 8601
    pub diff_summary: String,
    pub status: SessionStatus,
}

#[derive(Serialize, Deserialize, PartialEq)]
pub enum SessionStatus {
    Pending,
    Applied,
    Discarded,
    Expired,
}

/// Backup metadata sidecar.
#[derive(Serialize, Deserialize)]
pub struct BackupMeta {
    pub original_path: String,
    pub created_at: String,  // ISO 8601
    pub size_bytes: u64,
}
```

### Threshold Configuration

```rust
/// Write threshold configuration, loaded from env or defaults.
pub struct WriteThresholds {
    pub absolute_floor: usize,   // default 10
    pub absolute_ceil: usize,    // default 80
    pub ratio_threshold: f64,    // default 0.40
}

impl WriteThresholds {
    pub fn from_env() -> Self { /* read FETTLE_WRITE_* env vars */ }

    pub fn classify(&self, diff: &DiffResult) -> WriteTier {
        let changed = diff.changed_lines();
        if changed <= self.absolute_floor {
            WriteTier::DirectWrite { /* ... */ }
        } else if changed >= self.absolute_ceil {
            WriteTier::StagedWrite { /* ... */ }
        } else if diff.change_ratio() > self.ratio_threshold {
            WriteTier::StagedWrite { /* ... */ }
        } else {
            WriteTier::DirectWrite { /* ... */ }
        }
    }
}
```

---

## 8. Module Layout

### Current Structure

```
src/
  main.rs       -- entry point, hook mode vs CLI mode dispatch
  cli.rs        -- clap CLI definition
  hook.rs       -- hook input parsing, decision tree
  read.rs       -- file reading with line numbers
  write.rs      -- file writing (simple)
  filetype.rs   -- file category detection
  info.rs       -- info display
  install.rs    -- hook installation
```

### Proposed Structure

```
src/
  main.rs       -- entry point (add confirm/discard/rollback/status dispatch)
  cli.rs        -- clap CLI definition (add new subcommands)
  hook.rs       -- hook input parsing, decision tree (modify process_write)
  read.rs       -- file reading (unchanged)
  write.rs      -- file writing, ENHANCED: diff, backup, staging logic
  filetype.rs   -- file category detection (unchanged)
  info.rs       -- info display (update decision tree description)
  install.rs    -- hook installation (unchanged)
  diff.rs       -- NEW: diff computation, DiffResult, threshold logic
  backup.rs     -- NEW: backup creation, purging, rollback
  stage.rs      -- NEW: session staging, confirmation, discard, expiry
```

Three new modules. Each has a single, clear responsibility. No existing module
changes its primary purpose -- they gain new call sites but not new identities.

### Dependency Flow

```
main.rs
  |-> cli.rs (parse args)
  |-> hook.rs (hook mode)
        |-> read.rs (Read interception, unchanged)
        |-> write.rs (Write interception, enhanced)
              |-> diff.rs (compute diff, classify tier)
              |-> backup.rs (create backup)
              |-> stage.rs (stage content for Tier 2)
  |-> stage.rs (confirm/discard commands)
  |-> backup.rs (rollback command, status command)
```

---

## 9. Hook Flow -- Detailed Decision Tree

### `process_write` in hook.rs (enhanced)

```
1. Extract file_path from tool_input.
   - Missing? Return deny with error message. (unchanged)

2. Extract content from tool_input.
   - Missing? Return deny with error message. (unchanged)

3. Check if file exists on disk.
   |
   +-- NO (new file):
   |     a. Write content to file (create parent dirs as needed).
   |     b. Return deny: "fettle: Wrote {path} ({size}, {lines} lines) [new file]"
   |
   +-- YES (existing file):
         a. Read current content from disk.
         |
         +-- Read fails (permissions, etc):
         |     Write anyway (current behavior). No backup (can't read = can't backup).
         |     Return deny: "fettle: Wrote {path} ({size}, {lines} lines) [no backup: read failed]"
         |
         +-- Current content == proposed content:
         |     No-op. Don't write. Don't backup.
         |     Return deny: "fettle: No changes to {path} (content identical)"
         |
         +-- Content differs:
               b. Compute diff (diff.rs).
               c. Run opportunistic backup purge (backup.rs).
               d. Create backup of current file (backup.rs).
               |
               +-- Backup fails:
               |     Log warning, proceed anyway. Backup failure is not fatal.
               |
               e. Classify diff tier (diff.rs).
               |
               +-- Tier 1 (DirectWrite):
               |     f. Write new content to file.
               |     g. Return deny:
               |        "fettle: Wrote {path} ({size}, {lines} lines, {+N -M} changed)
               |         backup: {backup_filename}"
               |
               +-- Tier 2 (StagedWrite):
                     f. Run opportunistic stage purge (stage.rs).
                     g. Stage content to /tmp/fettle-stage/{session-id}/ (stage.rs).
                     h. Return deny:
                        "fettle: Staged write for {path} (session: {id})
                         {+N -M} lines changed ({pct}% of file)
                         backup: {backup_filename}

                        {unified diff output}

                        To apply: run `fettle confirm {id}` via Bash
                        To discard: run `fettle discard {id}` via Bash"
```

### The permission decision is "deny"

The code uses `permissionDecision: "deny"`. This is deliberate and correct.
When fettle handles a Write, it has already either written the file (Tier 1)
or staged it (Tier 2). Returning "deny" tells Claude Code not to also run
its builtin Write tool, which would cause a double-write.

Claude sees the deny reason as an "error" message, but the message itself
confirms the operation succeeded (e.g., `fettle: Wrote /path (+3 -1)`).
The "error" IS the success confirmation. This is how the hook protocol is
designed to work -- the deny reason is the communication channel.

For Tier 2 (staged writes), the deny reason contains the diff and
instructions to run `fettle confirm` or `fettle discard`. The file is NOT
written until confirmed. The deny prevents the builtin from writing it
prematurely.

---

## 10. The `fettle confirm` Flow

```
1. Parse session ID from CLI args.

2. Load /tmp/fettle-stage/{session-id}/metadata.json.
   - Directory doesn't exist? "No session '{id}'."
   - metadata.json missing? "Corrupted session '{id}'."

3. Check status field.
   - "applied"? "Session {id} was already applied."
   - "discarded"? "Session {id} was already discarded."

4. Check expiry.
   - Parse created_at, compare to now.
   - Older than 10 minutes? "Session {id} expired N minutes ago."
     Update status to "expired" in metadata. Return error.

5. Read staged content from /tmp/fettle-stage/{session-id}/content.

6. Write content to target_path.
   - Create parent dirs if needed (same logic as write.rs).
   - On failure: return error, keep staged content for retry.

7. Update metadata status to "applied".

8. Print confirmation: "fettle: Applied staged write to {path}"
```

---

## 11. The `fettle rollback` Flow

```
1. Resolve backup path:
   - Full path provided? Use it directly.
   - Filename only? Look in ~/.wonka/bench/fettle/backups/.
   - Not found? Error.

2. Read .meta sidecar.
   - Missing? Error unless --to flag provided.
   - Present? Extract original_path.

3. Determine restore target:
   - --to flag overrides original_path.
   - No --to and no .meta? Error: "Specify --to <path>".

4. Read backup content.

5. Write to restore target.
   - Note: does NOT create a counter-backup. Rollback is a deliberate
     reversal. If you want to undo the rollback, the staged content
     (if applicable) or git history is there.

6. Print confirmation: "fettle: Restored {path} from backup"
```

---

## 12. Integration with main.rs

### Updated CLI enum

```rust
pub enum Command {
    Read { file, offset, limit },      // existing
    Write { file },                     // existing
    Install,                            // existing
    Info,                               // existing
    Confirm { session_id: String },     // NEW
    Discard { session_id: String },     // NEW
    Rollback {                          // NEW
        backup: String,
        to: Option<PathBuf>,
    },
    Status,                             // NEW
}
```

### Updated main.rs dispatch

The `run_hook_mode` function changes only in that `process_write` in hook.rs
now calls into the new diff/backup/stage modules. The hook mode entry point
itself is unchanged.

The `run_cli_mode` function gains four new match arms for confirm, discard,
rollback, and status. These are straightforward dispatches to stage.rs and
backup.rs functions.

The bare `fettle hook` entry (line 25-27) is unchanged. The detection logic
(stdin vs args) is unchanged.

---

## 13. Edge Cases

### File Deleted Between Hook Fire and Write

The hook fires, fettle reads the file for diffing, but by the time it writes,
the file is gone. This is astronomically unlikely in normal use (Claude Code
dispatches hooks synchronously) but possible if an external process deletes
the file.

**Handling:** The write call itself will create the file (fs::write creates if
absent). This is correct -- the model asked to write the file, so write it.
The diff was computed against the now-deleted content, which means the backup
captured the content that was deleted. Everything is consistent.

### Symlinks

Follow symlinks for reading (to compute diff against real content) and for
writing (write to the real target, not the link). Backups store the real path
in the `.meta` sidecar. `std::fs::read` and `std::fs::write` follow symlinks
by default in Rust, so this requires no special handling.

### Permission Issues

- Cannot read existing file for diff: Write anyway without diff/backup.
  Return a note in the confirmation message.
- Cannot create backup directory: Log warning, proceed without backup.
- Cannot write the file itself: Return error (this is fatal, same as current
  behavior).
- Cannot create stage directory: Fall back to Tier 1 (direct write). Stage
  failure should not block a write -- degrade gracefully.

### Concurrent Writes to Same File

As discussed in section 5: each write gets its own session and backup. No
locking. No merge. This is correct for the single-agent use case and
fail-safe for multi-agent edge cases.

### Very Large Files

Files over, say, 10MB: the diff computation itself could be slow. In practice,
Claude Code is unlikely to write 10MB files, but if it does, `similar`'s
Myers algorithm is O(ND) where N is file length and D is edit distance. For
large files with small diffs, this is fast. For large files with large diffs
(essentially a full rewrite), it could be slow.

**Mitigation:** If the file is over 5MB, skip diffing entirely. Write
directly, backup the original, note in the message that diff was skipped
due to size. This is a pragmatic escape hatch.

### Empty Files

An existing empty file being written to: the diff is all insertions. Follow
the normal threshold logic -- if the new content is over 80 lines, stage it.
This is probably overcautious for writing to an empty file, but consistency
matters more than cleverness here.

### Non-UTF-8 Content

If the proposed content (from tool_input) or the existing file content is not
valid UTF-8, skip diffing. Write directly with backup. Note in the message
that diff was skipped (binary content). `similar` operates on `&str`, so we
must validate UTF-8 before passing content to it. The tool_input content is
already a JSON string (so it is UTF-8), but the existing file on disk might
not be.

---

## 14. New Dependencies

### Required

- `similar = "2"` -- line diff computation. Pure Rust, no transitive deps
  worth worrying about. Well-maintained (by mitsuhiko).

### Not Required

- No random crate needed. Session IDs use timestamp-based hashing.
- No chrono/time crate needed. Use `std::time::SystemTime` for timestamps,
  format manually or use basic `UNIX_EPOCH` offset arithmetic for ISO 8601.
  The formatting is simple enough to not warrant a dependency.
- No fs-extra or similar. `std::fs` covers everything needed.

---

## 15. Token Efficiency Considerations

The whole point of fettle intercepting writes is to save tokens and remove
friction. The enhanced protocol must not introduce token bloat.

### Tier 1 Messages

Two lines max. The model already knows what it wrote. It just needs
confirmation plus backup location.

```
fettle: Wrote src/main.rs (2.1KB, 45 lines, +3 -2 changed)
  backup: main.rs.20260312_143022_471
```

### Tier 2 Messages

The diff is the unavoidable cost. But it replaces what would otherwise be
a full file re-read (the model would need to read the file to see if its
write worked). The diff is strictly smaller than the full file content for
any non-trivial file. Context radius of 3 keeps the diff focused.

The instructions ("To apply: run ...") are 2 lines. The session ID is 8
characters. Minimal overhead.

### No-Change Detection

If proposed content equals existing content, return a one-line "no changes"
message. This is a token savings for the common case where the model
re-writes a file it already wrote. No backup created, no diff computed.

---

## 16. Info Display Update

The `info.rs` decision tree display should be updated to reflect the new
write behavior:

```
Decision tree:
  Read + image/PDF/notebook  -> allow builtin (multimodal)
  Read + SVG                 -> fettle handles (text, not multimodal)
  Read + text < threshold    -> allow builtin (works fine)
  Read + text >= threshold   -> fettle reads (no size limit)
  Write + new file           -> fettle writes directly
  Write + small diff         -> fettle writes, backs up original
  Write + large diff         -> fettle stages, shows diff, waits for confirm
  Other tools                -> allow (pass through)
```

---

## 17. Testing Strategy

### Unit Tests (in each new module)

**diff.rs:**
- Identical content returns zero changes.
- Single line change counts correctly.
- Pure insertions counted correctly.
- Pure deletions counted correctly.
- Threshold classification: floor, ceiling, ratio boundaries.
- Empty old file (all insertions).
- Empty new file (all deletions).

**backup.rs:**
- Backup creates file and sidecar.
- Backup naming includes timestamp.
- Purge removes files older than 24h.
- Purge respects max count.
- Rollback restores from backup using sidecar.
- Rollback with --to flag.
- Missing sidecar error.

**stage.rs:**
- Session creation writes content and metadata.
- Confirm applies content correctly.
- Confirm on expired session returns error.
- Confirm on already-applied session returns error.
- Discard removes session.
- Purge removes expired sessions.
- Session ID generation produces 8-char hex.

### Integration Tests (in hook.rs or tests/)

- Write to new file: no diff, no backup, immediate write.
- Write with small diff: direct write, backup created.
- Write with large diff: staged, not written, session created.
- Confirm after staging: file written, session marked applied.
- Write identical content: no-op message, no backup.

---

## 18. Migration / Backward Compatibility

The enhanced write protocol is backward compatible in the following sense:

- The hook returns `permissionDecision: "deny"` with a reason string that
  serves as the success confirmation (see section 9 for rationale).
- The reason string format changes (adds diff info, backup paths) but the
  model already parses these as informational messages.
- New CLI subcommands (confirm, discard, rollback, status) don't conflict
  with existing ones.
- The `fettle write` CLI command (standalone, not hook mode) should also
  gain backup/diff behavior for consistency, but this is a lower priority.
  The standalone CLI is used less often than the hook path.

### Environment Variable Namespace

All new env vars use the `FETTLE_WRITE_` prefix:
- `FETTLE_WRITE_FLOOR` -- absolute floor (default 10)
- `FETTLE_WRITE_CEIL` -- absolute ceiling (default 80)
- `FETTLE_WRITE_RATIO` -- ratio threshold (default 0.40)
- `FETTLE_WRITE_STAGE_TTL` -- stage TTL in seconds (default 600)
- `FETTLE_BACKUP_DIR` -- backup directory override
- `FETTLE_STAGE_DIR` -- stage directory override

Existing `FETTLE_READ_THRESHOLD` is unchanged.

---

## 19. Open Questions (for the implementer)

1. **Should `fettle write` (CLI mode) also do backups?** Probably yes for
   consistency, but it adds complexity to a path that is rarely used. Could
   be deferred.

2. **Should the no-change detection do a byte-level comparison or line-level?**
   Byte-level is faster and catches whitespace-only differences. Recommend
   byte-level for the no-change check, line-level for the actual diff.

3. **Should Tier 2 show the FULL unified diff or cap it?** If a diff is 500
   lines long, that is a lot of tokens. Consider capping the displayed diff
   at, say, 200 lines and noting "... N more lines of diff truncated."
   The staged content is always available in full via the session.

4. **What about `Edit` tool calls?** Claude Code also has an Edit tool. Fettle
   currently does not intercept it. The Edit tool sends a diff-like input
   (old_string/new_string), not full file content. This is a separate concern
   and should not be mixed into this design.

---

*squints at the napkin one more time*

The abstraction holds. Three new modules, three new subcommands, one new
dependency. The existing architecture bends to accommodate this without
breaking. The threshold logic is adaptive rather than fixed, which is the
right amount of clever without being too clever.

The napkin is ready. Hand it to the foundry.
