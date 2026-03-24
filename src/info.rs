use crate::backup;
use crate::install;
use crate::stage;

/// Display fettle configuration and status.
pub fn show() -> String {
    let mut out = String::new();

    out.push_str("fettle -- in fine fettle\n\n");

    // Installation status: settings.json method
    let settings_installed = install::settings_json_installed();
    if settings_installed {
        out.push_str("Hook (settings.json): installed\n");
    } else {
        out.push_str("Hook (settings.json): not found\n");
    }

    // Installation status: legacy script method
    let (script_installed, hook_path) = install::script_installed();
    if script_installed {
        out.push_str(&format!(
            "Hook (script):        installed at {}\n",
            hook_path.display()
        ));
    } else {
        out.push_str("Hook (script):        not found\n");
    }

    // Hint if neither is installed
    if !settings_installed && !script_installed {
        out.push_str("  (run `fettle install` to set up hooks)\n");
    }

    // Threshold config
    let threshold = std::env::var("FETTLE_READ_THRESHOLD").unwrap_or_else(|_| "48KB".to_string());
    out.push_str(&format!("Read threshold: {threshold}\n"));
    out.push_str("  (set FETTLE_READ_THRESHOLD to override, e.g. \"64KB\", \"96k\", \"49152\")\n");

    // Write thresholds
    let floor = std::env::var("FETTLE_WRITE_FLOOR").unwrap_or_else(|_| "10".to_string());
    let ceil = std::env::var("FETTLE_WRITE_CEIL").unwrap_or_else(|_| "80".to_string());
    let ratio = std::env::var("FETTLE_WRITE_RATIO").unwrap_or_else(|_| "0.40".to_string());
    out.push_str(&format!(
        "Write thresholds: floor={floor}, ceil={ceil}, ratio={ratio}\n"
    ));

    // Directories
    out.push_str(&format!("Backup dir: {}\n", backup::backup_dir().display()));
    out.push_str(&format!("Staging dir: {}\n", stage::stage_dir().display()));

    // Decision tree summary
    out.push_str("\nDecision tree:\n");
    out.push_str("  Read + image/PDF/notebook  -> allow builtin (multimodal)\n");
    out.push_str("  Read + SVG                 -> fettle handles (text, not multimodal)\n");
    out.push_str("  Read + text < threshold    -> allow builtin (works fine)\n");
    out.push_str("  Read + text >= threshold   -> fettle reads (no size limit)\n");
    out.push_str("  Write + new file           -> fettle writes directly\n");
    out.push_str("  Write + small diff         -> fettle writes, backs up original\n");
    out.push_str("  Write + large diff         -> fettle stages, shows diff, waits for confirm\n");
    out.push_str("  Edit (any)                 -> fettle reads file, validates, applies edit\n");
    out.push_str("                                (same backup/diff/tier protocol as Write)\n");
    out.push_str("  Other tools                -> allow (pass through)\n");

    out
}
