use std::path::Path;

/// Categories of files fettle cares about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileCategory {
    /// Text files — code, config, markup, data formats. Fettle handles these.
    Text,
    /// Images — PNG, JPG, WEBP, etc. Claude's builtin handles these (multimodal).
    Image,
    /// SVG — technically an image extension, but it's XML text. Fettle handles it.
    Svg,
    /// PDF documents. Claude's builtin handles these.
    Pdf,
    /// Jupyter notebooks. Claude's builtin handles these (special format).
    Notebook,
    /// Binary files. Not text, not a known multimodal format.
    Binary,
}

impl FileCategory {
    /// Should fettle let the builtin tool handle this file?
    pub fn allow_builtin(self) -> bool {
        matches!(
            self,
            FileCategory::Image | FileCategory::Pdf | FileCategory::Notebook
        )
    }

    /// Is this something fettle reads as text?
    #[allow(dead_code)]
    pub fn is_text(self) -> bool {
        matches!(self, FileCategory::Text | FileCategory::Svg)
    }
}

impl std::fmt::Display for FileCategory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FileCategory::Text => write!(f, "text"),
            FileCategory::Image => write!(f, "image"),
            FileCategory::Svg => write!(f, "svg"),
            FileCategory::Pdf => write!(f, "pdf"),
            FileCategory::Notebook => write!(f, "notebook"),
            FileCategory::Binary => write!(f, "binary"),
        }
    }
}

/// Known image extensions that Claude handles natively (multimodal).
const IMAGE_EXTENSIONS: &[&str] = &[
    "png", "jpg", "jpeg", "webp", "gif", "bmp", "ico", "tiff", "tif",
];

/// Known binary extensions that aren't text.
const BINARY_EXTENSIONS: &[&str] = &[
    // Compiled / object
    "o", "obj", "so", "dylib", "dll", "exe", "bin", "com", // Archives
    "zip", "tar", "gz", "bz2", "xz", "7z", "rar", "zst", // Media (non-image)
    "mp3", "mp4", "wav", "flac", "ogg", "avi", "mkv", "mov", "wmv", // Fonts
    "ttf", "otf", "woff", "woff2", // Other binary
    "wasm", "class", "pyc", "pyo", "sqlite", "db",
];

/// Detect file category from its path (extension-based).
pub fn detect(path: &Path) -> FileCategory {
    let ext = match path.extension().and_then(|e| e.to_str()) {
        Some(e) => e.to_lowercase(),
        None => return FileCategory::Text, // no extension = assume text
    };

    let ext = ext.as_str();

    // SVG first — it's text despite looking like an image format
    if ext == "svg" {
        return FileCategory::Svg;
    }

    // PDF
    if ext == "pdf" {
        return FileCategory::Pdf;
    }

    // Jupyter notebooks
    if ext == "ipynb" {
        return FileCategory::Notebook;
    }

    // Images (multimodal)
    if IMAGE_EXTENSIONS.contains(&ext) {
        return FileCategory::Image;
    }

    // Known binary
    if BINARY_EXTENSIONS.contains(&ext) {
        return FileCategory::Binary;
    }

    // Default: treat as text. This covers all code, config, markup, data formats.
    // Better to try reading a binary file as text (and get garbage) than to refuse
    // to read a text file with an unusual extension.
    FileCategory::Text
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn test_common_text_files() {
        let text_paths = [
            "main.rs",
            "index.js",
            "app.py",
            "config.yaml",
            "data.json",
            "Makefile",
            "Dockerfile",
            ".gitignore",
            "README.md",
            "style.css",
            "page.html",
            "query.sql",
            "build.gradle",
            "pom.xml",
            "go.mod",
            "Cargo.toml",
            "package.json",
            "tsconfig.json",
            "shell.sh",
            "notes.txt",
            "CHANGELOG",
            "LICENSE",
        ];
        for name in &text_paths {
            assert_eq!(
                detect(Path::new(name)),
                FileCategory::Text,
                "{name} should be detected as text"
            );
        }
    }

    #[test]
    fn test_svg_is_text_not_image() {
        assert_eq!(detect(Path::new("diagram.svg")), FileCategory::Svg);
        assert_eq!(detect(Path::new("icon.SVG")), FileCategory::Svg);
        assert!(FileCategory::Svg.is_text());
        assert!(!FileCategory::Svg.allow_builtin());
    }

    #[test]
    fn test_images_allow_builtin() {
        let image_paths = [
            "photo.png",
            "pic.jpg",
            "image.jpeg",
            "hero.webp",
            "anim.gif",
        ];
        for name in &image_paths {
            let cat = detect(Path::new(name));
            assert_eq!(cat, FileCategory::Image, "{name} should be image");
            assert!(cat.allow_builtin(), "{name} should allow builtin");
        }
    }

    #[test]
    fn test_pdf_allows_builtin() {
        assert_eq!(detect(Path::new("doc.pdf")), FileCategory::Pdf);
        assert!(FileCategory::Pdf.allow_builtin());
    }

    #[test]
    fn test_notebook_allows_builtin() {
        assert_eq!(detect(Path::new("analysis.ipynb")), FileCategory::Notebook);
        assert!(FileCategory::Notebook.allow_builtin());
    }

    #[test]
    fn test_binary_detection() {
        let binary_paths = [
            "lib.so",
            "app.exe",
            "archive.zip",
            "font.woff2",
            "module.wasm",
        ];
        for name in &binary_paths {
            assert_eq!(
                detect(Path::new(name)),
                FileCategory::Binary,
                "{name} should be binary"
            );
        }
    }

    #[test]
    fn test_no_extension_is_text() {
        assert_eq!(detect(Path::new("Makefile")), FileCategory::Text);
        assert_eq!(detect(Path::new("CHANGELOG")), FileCategory::Text);
    }

    #[test]
    fn test_case_insensitive() {
        assert_eq!(detect(Path::new("image.PNG")), FileCategory::Image);
        assert_eq!(detect(Path::new("doc.PDF")), FileCategory::Pdf);
        assert_eq!(detect(Path::new("data.JSON")), FileCategory::Text);
    }
}
