use crate::install;

/// Display fettle configuration and status.
pub fn show() -> String {
    let mut out = String::new();

    out.push_str("fettle -- in fine fettle\n\n");

    // Installation status
    let (installed, hook_path) = install::status();
    if installed {
        out.push_str(&format!("Hook: installed at {}\n", hook_path.display()));
    } else {
        out.push_str("Hook: not installed (run `fettle install`)\n");
    }

    // Threshold config
    let threshold = std::env::var("FETTLE_READ_THRESHOLD").unwrap_or_else(|_| "48KB".to_string());
    out.push_str(&format!("Read threshold: {threshold}\n"));
    out.push_str("  (set FETTLE_READ_THRESHOLD to override, e.g. \"64KB\", \"96k\", \"49152\")\n");

    // Decision tree summary
    out.push_str("\nDecision tree:\n");
    out.push_str("  Read + image/PDF/notebook  -> allow builtin (multimodal)\n");
    out.push_str("  Read + SVG                 -> fettle handles (text, not multimodal)\n");
    out.push_str("  Read + text < threshold    -> allow builtin (works fine)\n");
    out.push_str("  Read + text >= threshold   -> fettle reads (no size limit)\n");
    out.push_str("  Write                      -> fettle handles (no read-gate)\n");
    out.push_str("  Other tools                -> allow (pass through)\n");

    out
}
