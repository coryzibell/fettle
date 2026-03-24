# fettle

**Put your file tools in fine fettle.** A Claude Code PreToolUse hook that replaces constrained file tools with proper, unrestricted alternatives.

## The Problem

Claude Code's built-in `Read`, `Write`, and `Edit` tools have artificial limits:

- **Read** fails on text files over 25,000 tokens (~1,500 lines of code) or 256KB
- **Read** silently truncates files between 48-126KB to a 2KB preview
- **Write** refuses to write if you haven't `Read` the file first in the current session -- burning tokens on a mandatory read-before-write round trip
- **Edit** refuses to edit if you haven't `Read` the file first in the current session -- same mandatory read-before-edit round trip
- The docs claim lines over 2,000 characters are truncated. [They aren't.](https://github.com/coryzibell/fettle/issues/1)

Images, PDFs, and notebooks are unaffected. The limits only hit text files, which is most of what coding agents work with.

## What fettle Does

fettle installs as a Claude Code `PreToolUse` hook. It intercepts `Read`, `Write`, and `Edit` tool calls transparently:

**Reads:**
- Text files >= 48KB: fettle reads them directly, no token limits, no size caps
- Text files < 48KB: passed through to the builtin (works fine at this size)
- SVG files: treated as text (they are XML, not images)
- Images, PDFs, notebooks: passed through to the builtin (multimodal rendering)

**Writes -- tiered protocol:**
- **New files**: written directly, confirmation returned
- **No changes detected**: skipped with message (byte-level comparison)
- **Small diffs**: backed up, written, diff summary returned
- **Large diffs**: backed up, staged for review, diff displayed, user confirms or discards

**Edits -- no read-before-edit required:**
- fettle reads the file itself, validates the replacement, and applies it
- `old_string` must exist in the file (brief error if not found; use `fettle edit-diagnose` for detailed diagnostics)
- If `replace_all` is false (default), `old_string` must be unique
- After computing the replacement, the result goes through the same diff/backup/tier protocol as Write
- Agents can call Edit without a prior Read -- fettle handles the file access

Agents do not need to change anything. They call `Read`, `Write`, and `Edit` as normal. fettle makes them work better.

## Install

```bash
cargo install fettle
fettle install
```

`fettle install` does two things:

1. **Registers the hook in `~/.claude/settings.json`** (primary method) -- adds a `PreToolUse` hook entry that runs `fettle hook` for `Read`, `Write`, and `Edit` tool calls.
2. **Creates a legacy hook script** at `~/.claude/hooks/pre-tool-use/fettle` for backwards compatibility with older Claude Code versions.

The settings.json entry looks like this:

```json
{
  "hooks": {
    "PreToolUse": [
      {
        "matcher": "Read|Write|Edit",
        "hooks": [
          {
            "type": "command",
            "command": "fettle hook"
          }
        ]
      }
    ]
  }
}
```

If you prefer manual configuration, add that snippet to your `~/.claude/settings.json` instead of running `fettle install`.

To verify installation:

```bash
fettle info
```

`fettle info` reports the status of both installation methods (settings.json and legacy script), along with current configuration.

## How It Works

### The Hook Protocol

Claude Code's [hook system](https://docs.anthropic.com/en/docs/claude-code/hooks) sends a JSON payload to `PreToolUse` hooks on stdin:

```json
{"tool_name": "Write", "tool_input": {"file_path": "/path/to/file", "content": "..."}}
{"tool_name": "Edit", "tool_input": {"file_path": "/path/to/file", "old_string": "...", "new_string": "...", "replace_all": false}}
```

fettle reads this JSON, decides whether to handle the tool call or let the builtin proceed, and responds:

- **Allow** (pass through): exit 0, print nothing to stdout. The builtin tool runs normally.
- **Deny** (fettle handled it): exit 0, print a JSON envelope to stdout with `permissionDecision: "deny"`. The builtin tool does not run.

The "deny = success" convention is central to how fettle works. When fettle writes a file, it denies the builtin Write so that Claude Code does not also try to write it. Claude sees the deny reason as an "error" message, but the message itself confirms the operation succeeded (e.g., `fettle: Wrote /path/to/file (+3 -1)`). This is how the hook protocol is designed to work.

fettle always exits 0. A non-zero exit would cause Claude Code to fail the tool call entirely. Even parse errors fail open -- if fettle cannot understand the hook input, it allows the builtin to proceed.

### Read Interception

When Claude calls `Read`:

1. fettle checks the file extension. Images, PDFs, and notebooks pass through to the builtin (multimodal rendering).
2. SVG files are treated as text (XML content, not raster images).
3. For text files, fettle checks the file size against the threshold (default 48KB).
4. Files under the threshold pass through to the builtin.
5. Files at or above the threshold are read by fettle directly, formatted with `cat -n` style line numbers, and returned as the deny reason. No size limits.

The `offset` and `limit` parameters from the original Read tool are respected.

### Edit Interception

When Claude calls `Edit`, fettle runs through this decision tree:

```
File does not exist?
  --> Deny with error. Edit requires an existing file.

old_string == new_string?
  --> Skip. Return "no changes" message. Done.

old_string not found in file?
  --> Deny with brief error (defense-in-depth; Claude Code catches this first).
      For diagnostics, use: fettle edit-diagnose <file> <search_string>

replace_all is false and old_string appears more than once?
  --> Deny with brief error (defense-in-depth; Claude Code catches this first).

Otherwise:
  --> Apply replacement (all occurrences if replace_all, otherwise the single match)
  --> Feed result through the same Write protocol (diff, backup, tier classification)
```

Edit does **not** require a prior Read. fettle reads the file itself, validates the edit, applies it, and runs the result through the same backup/diff/staging protocol that Write uses. This eliminates the read-before-edit round trip.

### Write Interception

When Claude calls `Write`, fettle runs through this decision tree:

```
File does not exist?
  --> Write directly. Return confirmation. Done.

File exists, content identical (byte comparison)?
  --> Skip. Return "no changes" message. Done.

File exists, content differs:
  --> Create backup of current file
  --> Compute unified diff (Myers algorithm via `similar` crate)
  --> Classify the diff:

      Changed lines <= floor (10)?
        --> Tier 1: Write directly. Return diff summary.

      Changed lines >= ceiling (80)?
        --> Tier 2: Stage content. Show diff. Wait for confirm/discard.

      Change ratio > threshold (40%) AND changed lines > floor?
        --> Tier 2: Stage content. Show diff. Wait for confirm/discard.

      Otherwise:
        --> Tier 1: Write directly. Return diff summary.
```

**Tier 1 (direct write):** The file is written immediately. The backup exists if you need to roll back, but no confirmation is required.

**Tier 2 (staged write):** The proposed content is saved to a staging directory with a session ID. Claude sees the diff and instructions to run `fettle confirm <session-id>` or `fettle discard <session-id>`. The original file is not modified until confirmation.

Special cases:
- Binary files (non-UTF-8): backup + direct write, diff skipped
- Files over 5MB: backup + direct write, diff skipped for performance
- Diffs over 200 lines: truncated in the display (full content is still staged)

## CLI Reference

### `fettle hook`

Run as a Claude Code PreToolUse hook. Reads JSON from stdin, processes the tool call, and writes the hook response to stdout. This is what the installed hook script calls.

```bash
echo '{"tool_name":"Read","tool_input":{"file_path":"/tmp/test.txt"}}' | fettle hook
```

You do not call this manually. The hook script does it.

### `fettle install`

Install fettle as a Claude Code pre-tool-use hook. Registers the hook in `~/.claude/settings.json` (primary) and creates a legacy hook script at `~/.claude/hooks/pre-tool-use/fettle` (backwards compatibility). Safe to re-run.

```bash
fettle install
```

### `fettle uninstall`

Remove fettle hooks from Claude Code configuration. Removes the hook entry from `~/.claude/settings.json` and deletes the legacy hook script at `~/.claude/hooks/pre-tool-use/fettle`. Safe to re-run (idempotent).

```bash
fettle uninstall
```

Does NOT delete backups, staged writes, or the fettle binary. To remove the binary: `cargo uninstall fettle`.

### `fettle info`

Show current configuration, installation status, and the decision tree summary.

```bash
fettle info
```

Output includes: settings.json hook status, legacy script status, read threshold, write thresholds (floor/ceiling/ratio), backup directory, and staging directory.

### `fettle confirm <session-id>`

Apply a staged Tier 2 write. The session ID is shown in the staging message when a large diff is detected.

```bash
fettle confirm a1b2c3d4
```

Sessions expire after 10 minutes (configurable). Expired sessions cannot be confirmed.

### `fettle discard <session-id>`

Throw away a staged write without applying it. Removes the staging directory for that session.

```bash
fettle discard a1b2c3d4
```

### `fettle rollback <backup> [--to <path>]`

Restore a file from a backup. The backup can be specified as a filename (looked up in the backup directory) or a full path.

```bash
# Restore to the original path (read from .meta sidecar)
fettle rollback main.rs.20260312_143022_517

# Restore to a different path
fettle rollback main.rs.20260312_143022_517 --to /tmp/recovered.rs
```

If the `.meta` sidecar file is missing, you must specify `--to`.

### `fettle status`

Show pending staged sessions and recent backups.

```bash
fettle status
```

Example output:

```
Pending staged writes:
  a1b2c3d4  /home/user/project/src/main.rs             +15 -8     2 min ago

Recent backups (last 24h):
  main.rs.20260312_143022_517              /home/user/project/src/main.rs             5 min ago
```

### `fettle read <file> [--offset N] [--limit N]`

Read a file with `cat -n` style line numbers. No size limits. Also usable standalone outside of hook mode.

```bash
fettle read src/main.rs
fettle read big_file.log --offset 500 --limit 100
```

### `fettle write <file>`

Write content from stdin to a file. Creates parent directories if needed. Also usable standalone.

```bash
echo "new content" | fettle write output.txt
```

### `fettle edit-diagnose <file> <search_string>`

Diagnose why an Edit tool search string was not found. When Claude Code's Edit tool reports "String to replace not found", run this command to understand why.

- If the string is found: reports exact match location(s) with line numbers and surrounding context
- If not found: searches for near-matches (first line as substring), shows context, and suggests common causes (whitespace, indentation, line endings)
- If multiple matches: reports all locations and suggests using `replace_all` or adding more context

```bash
fettle edit-diagnose src/main.rs 'fn main() {'
fettle edit-diagnose src/lib.rs 'impl MyStruct'
```

## Configuration

All configuration is via environment variables. No config files.

### Read Threshold

| Variable | Default | Description |
|----------|---------|-------------|
| `FETTLE_READ_THRESHOLD` | `48KB` | File size above which fettle handles reads instead of the builtin. Accepts bytes (`49152`), KB (`48KB`, `48k`), or MB (`1MB`, `1m`). |

### Write Thresholds

The write tier classification uses three values that work together:

| Variable | Default | Description |
|----------|---------|-------------|
| `FETTLE_WRITE_FLOOR` | `10` | Changed lines at or below this count always go Tier 1 (direct write). |
| `FETTLE_WRITE_CEIL` | `80` | Changed lines at or above this count always go Tier 2 (staged write). |
| `FETTLE_WRITE_RATIO` | `0.40` | Between floor and ceiling, if changed lines exceed this fraction of the original file, it goes Tier 2. |

The classification logic: if changed lines <= floor, Tier 1. If changed lines >= ceiling, Tier 2. Otherwise, if the change ratio exceeds the threshold, Tier 2. Otherwise, Tier 1.

### Directories

| Variable | Default | Description |
|----------|---------|-------------|
| `FETTLE_BACKUP_DIR` | `~/.wonka/bench/fettle/backups/` | Where file backups are stored. |
| `FETTLE_STAGE_DIR` | `/tmp/fettle-stage/` | Where staged Tier 2 content is stored. |
| `FETTLE_WRITE_STAGE_TTL` | `600` | Staged session time-to-live in seconds (default: 10 minutes). |

## Backup System

Every write to an existing file (where the content actually changed) creates a backup of the original content before writing.

- **Location:** `~/.wonka/bench/fettle/backups/` (or `FETTLE_BACKUP_DIR`)
- **Naming:** `<filename>.<YYYYMMDD_HHMMSS_mmm>` (e.g., `main.rs.20260312_143022_517`)
- **Metadata:** Each backup has a `.meta` JSON sidecar with the original path, creation timestamp, and file size
- **Retention:** 24 hours. Backups older than 24h are purged opportunistically (during the next backup creation).
- **Limit:** Maximum 100 backup files. Oldest are purged first when the limit is reached.
- **Failure is non-fatal:** If a backup cannot be created (permissions, disk space), the write proceeds anyway with a warning.

## Staging System

Tier 2 writes (large diffs) are staged rather than applied immediately.

- **Location:** `/tmp/fettle-stage/` (or `FETTLE_STAGE_DIR`)
- **Structure:** Each session gets a directory named by its 8-character hex session ID, containing `content` (the proposed file content) and `metadata.json` (target path, backup path, diff summary, status, creation time).
- **TTL:** 10 minutes (or `FETTLE_WRITE_STAGE_TTL`). Expired sessions cannot be confirmed.
- **Cleanup:** Expired sessions are purged opportunistically when new sessions are created.
- **Session states:** `pending` (awaiting confirm/discard), `applied` (confirmed and written), `discarded` (thrown away), `expired` (TTL exceeded).

## File Type Detection

fettle classifies files by extension to decide routing:

| Category | Extensions | Handling |
|----------|-----------|----------|
| Text | `.rs`, `.js`, `.py`, `.json`, `.toml`, `.md`, `.html`, `.css`, `.sql`, `.sh`, and all other unrecognized extensions | fettle handles if above threshold |
| SVG | `.svg` | Treated as text (XML), not routed to multimodal |
| Image | `.png`, `.jpg`, `.jpeg`, `.webp`, `.gif`, `.bmp`, `.ico`, `.tiff`, `.tif` | Passed to builtin (multimodal) |
| PDF | `.pdf` | Passed to builtin (multimodal) |
| Notebook | `.ipynb` | Passed to builtin (special format) |
| Binary | `.so`, `.exe`, `.zip`, `.wasm`, `.pyc`, `.sqlite`, etc. | Passed to builtin |

Files with no extension are assumed to be text.

## Empirical Testing

All limits were [empirically tested](https://github.com/coryzibell/fettle/issues/1), not assumed from documentation (which turned out to be wrong about several things).

| Scenario | Built-in | fettle |
|----------|----------|--------|
| 2,000-line source file | Token error | Works |
| 500KB log file | Size error | Works |
| Write after creating file via shell | "Read it first" | Works |
| Edit without prior Read | "Read it first" | Works |
| 8MB PNG screenshot | Works | Passes through |
| 50-page PDF | Works | Passes through |

## License

Apache 2.0
