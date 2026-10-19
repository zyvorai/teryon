//! Finding the monospaced fonts installed on this machine, for the desktop font picker. Only each file's
//! table directory and its `post` table header are read, so scanning hundreds of fonts is quick.
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

/// Largest font file offered (some CJK and color fonts are huge).
const MAX_FILE: u64 = 16 << 20;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FontFile {
    pub path: PathBuf,
    /// File name without the extension; this is what `font_family` can be set to.
    pub stem: String,
    /// A readable name: the stem without `-Regular`, dashes as spaces.
    pub label: String,
}

fn u16_at(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_be_bytes(b.get(at..at + 2)?.try_into().ok()?))
}
fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

/// Whether the font's `post` table says it is fixed-pitch; `None` when the file is not a font we can read.
/// For a collection (`.ttc`) the first font decides.
pub fn is_fixed_pitch<R: Read + Seek>(r: &mut R) -> Option<bool> {
    let mut head = [0u8; 16];
    r.seek(SeekFrom::Start(0)).ok()?;
    r.read_exact(&mut head).ok()?;
    let base = if &head[..4] == b"ttcf" {
        u64::from(u32_at(&head, 12)?)
    } else {
        0
    };
    let mut dir = [0u8; 12];
    r.seek(SeekFrom::Start(base)).ok()?;
    r.read_exact(&mut dir).ok()?;
    let tag = u32_at(&dir, 0)?;
    if !matches!(tag, 0x0001_0000 | 0x4F54_544F | 0x7472_7565) {
        return None;
    }
    let tables = usize::from(u16_at(&dir, 4)?);
    if tables == 0 || tables > 128 {
        return None;
    }
    let mut records = vec![0u8; tables * 16];
    r.read_exact(&mut records).ok()?;
    let post = records.chunks(16).find(|rec| &rec[..4] == b"post")?;
    let offset = u64::from(u32_at(post, 8)?);
    let mut table = [0u8; 16];
    r.seek(SeekFrom::Start(offset)).ok()?;
    r.read_exact(&mut table).ok()?;
    Some(u32_at(&table, 12)? != 0)
}

/// Plain upright weight: no bold, italic, light, narrow or other style words in the file name.
pub fn is_regular_style(stem: &str) -> bool {
    let l = stem.to_lowercase();
    ![
        "bold",
        "italic",
        "oblique",
        "light",
        "thin",
        "medium",
        "semi",
        "black",
        "heavy",
        "extra",
        "condensed",
        "narrow",
        "ultra",
        "demi",
        "retina",
    ]
    .iter()
    .any(|w| l.contains(w))
}

/// `JetBrainsMono-Regular` -> `JetBrainsMono`; `Courier New` stays.
pub fn label_for(stem: &str) -> String {
    let s = stem
        .strip_suffix("-Regular")
        .or_else(|| stem.strip_suffix("Regular"))
        .unwrap_or(stem);
    s.replace(['-', '_'], " ").trim().to_string()
}

/// The folders fonts are searched in: system, local and the user's.
pub fn font_dirs(home: Option<&Path>) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = vec![
        "/System/Library/Fonts".into(),
        "/Library/Fonts".into(),
        "/usr/share/fonts".into(),
    ];
    if let Some(h) = home {
        dirs.push(h.join("Library/Fonts"));
        dirs.push(h.join(".fonts"));
        dirs.push(h.join(".local/share/fonts"));
    }
    dirs
}

/// Monospaced, regular-weight fonts in `dirs` (two levels deep), sorted by label, one per label.
pub fn scan(dirs: &[PathBuf]) -> Vec<FontFile> {
    let mut found: Vec<FontFile> = Vec::new();
    let mut stack: Vec<(PathBuf, u8)> = dirs.iter().cloned().map(|d| (d, 0)).collect();
    while let Some((dir, depth)) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if depth < 2 {
                    stack.push((path, depth + 1));
                }
                continue;
            }
            let ext = path
                .extension()
                .map(|e| e.to_string_lossy().to_lowercase())
                .unwrap_or_default();
            if !matches!(ext.as_str(), "ttf" | "otf" | "ttc") {
                continue;
            }
            let Some(stem) = path.file_stem().map(|s| s.to_string_lossy().into_owned()) else {
                continue;
            };
            if !is_regular_style(&stem) {
                continue;
            }
            let Ok(meta) = std::fs::metadata(&path) else {
                continue;
            };
            if meta.len() > MAX_FILE {
                continue;
            }
            let Ok(mut file) = std::fs::File::open(&path) else {
                continue;
            };
            if is_fixed_pitch(&mut file) == Some(true) {
                found.push(FontFile {
                    label: label_for(&stem),
                    path,
                    stem,
                });
            }
        }
    }
    found.sort_by(|a, b| {
        a.label
            .to_lowercase()
            .cmp(&b.label.to_lowercase())
            .then(a.path.cmp(&b.path))
    });
    found.dedup_by(|a, b| a.label.eq_ignore_ascii_case(&b.label));
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    /// A minimal sfnt with one `post` table whose isFixedPitch field is `fixed`.
    fn font(fixed: u32, ttc: bool) -> Vec<u8> {
        let mut out = Vec::new();
        let base = if ttc { 16u32 } else { 0 };
        if ttc {
            out.extend_from_slice(b"ttcf");
            out.extend_from_slice(&[0, 1, 0, 0]);
            out.extend_from_slice(&1u32.to_be_bytes());
            out.extend_from_slice(&base.to_be_bytes());
        }
        out.extend_from_slice(&0x0001_0000u32.to_be_bytes());
        out.extend_from_slice(&1u16.to_be_bytes());
        out.extend_from_slice(&[0; 6]);
        out.extend_from_slice(b"post");
        out.extend_from_slice(&[0; 4]);
        out.extend_from_slice(&(base + 28).to_be_bytes());
        out.extend_from_slice(&32u32.to_be_bytes());
        let mut post = [0u8; 32];
        post[12..16].copy_from_slice(&fixed.to_be_bytes());
        out.extend_from_slice(&post);
        out
    }
    #[test]
    fn the_post_table_tells_monospace_from_proportional() {
        assert_eq!(is_fixed_pitch(&mut Cursor::new(font(1, false))), Some(true));
        assert_eq!(
            is_fixed_pitch(&mut Cursor::new(font(0, false))),
            Some(false)
        );
        assert_eq!(
            is_fixed_pitch(&mut Cursor::new(font(1, true))),
            Some(true),
            "a collection"
        );
        assert_eq!(
            is_fixed_pitch(&mut Cursor::new(b"not a font at all".to_vec())),
            None
        );
        assert_eq!(is_fixed_pitch(&mut Cursor::new(Vec::new())), None);
    }
    #[test]
    fn only_plain_regular_files_are_offered_and_labels_read_well() {
        for ok in [
            "JetBrainsMono-Regular",
            "Menlo",
            "Courier New",
            "HackNerdFontMono-Regular",
        ] {
            assert!(is_regular_style(ok), "{ok}");
        }
        for no in [
            "Menlo-Bold",
            "Fira Code Light",
            "X-Italic",
            "Mono-SemiBold",
            "Y Condensed",
        ] {
            assert!(!is_regular_style(no), "{no}");
        }
        assert_eq!(label_for("JetBrainsMono-Regular"), "JetBrainsMono");
        assert_eq!(label_for("Hack Regular"), "Hack");
        assert_eq!(label_for("Fira_Code"), "Fira Code");
    }
    #[test]
    fn scanning_a_folder_keeps_monospace_regular_fonts_once_per_label() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Mono One-Regular.ttf"), font(1, false)).unwrap();
        std::fs::write(dir.path().join("Mono One-Bold.ttf"), font(1, false)).unwrap();
        std::fs::write(dir.path().join("Sans.ttf"), font(0, false)).unwrap();
        std::fs::write(dir.path().join("notes.txt"), b"hi").unwrap();
        std::fs::create_dir(dir.path().join("sub")).unwrap();
        std::fs::write(dir.path().join("sub").join("Mono One.otf"), font(1, false)).unwrap();
        std::fs::write(dir.path().join("Zed.ttc"), font(1, true)).unwrap();
        let found = scan(&[dir.path().to_path_buf()]);
        let labels: Vec<&str> = found.iter().map(|f| f.label.as_str()).collect();
        assert_eq!(labels, ["Mono One", "Zed"], "{labels:?}");
    }
    #[test]
    fn this_machines_fonts_scan_without_trouble() {
        let home = std::env::var_os("HOME").map(PathBuf::from);
        let found = scan(&font_dirs(home.as_deref()));
        assert!(found.iter().all(|f| f.path.exists() && !f.label.is_empty()));
    }
}
