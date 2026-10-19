//! Changing options in `config.toml` from the app without losing the user's comments or layout.
use crate::config::Config;
use anyhow::{bail, Result};
use toml_edit::{value, DocumentMut, Item, Table};

/// A value a settings control can write.
#[derive(Clone, Debug, PartialEq)]
pub enum Val {
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
}

fn section<'a>(doc: &'a mut DocumentMut, path: &[&str]) -> Result<&'a mut Table> {
    let mut table = doc.as_table_mut();
    for name in path {
        let entry = table
            .entry(name)
            .or_insert_with(|| Item::Table(Table::new()));
        if !entry.is_table() {
            bail!("`{name}` in the config is not a table");
        }
        table = entry.as_table_mut().unwrap();
    }
    Ok(table)
}

fn to_item(v: &Val) -> Item {
    match v {
        Val::Bool(b) => value(*b),
        Val::Int(i) => value(*i),
        // Two decimals are plenty for a slider and keep the file tidy (0.94, not 0.9399999).
        Val::Float(f) => value((f * 100.).round() / 100.),
        Val::Str(s) => value(s.clone()),
    }
}

/// `text` with `[desktop] key = v` set, comments and ordering kept.
pub fn set_desktop(text: &str, key: &str, v: &Val) -> Result<String> {
    let mut doc: DocumentMut = text.parse()?;
    section(&mut doc, &["desktop"])?.insert(key, to_item(v));
    Ok(doc.to_string())
}
/// `text` without `[desktop] key`, so the default applies again.
pub fn unset_desktop(text: &str, key: &str) -> Result<String> {
    let mut doc: DocumentMut = text.parse()?;
    section(&mut doc, &["desktop"])?.remove(key);
    Ok(doc.to_string())
}
/// `text` with `[desktop.keys] action = "chord"` (`"none"` unbinds).
pub fn set_key(text: &str, action: &str, chord: &str) -> Result<String> {
    let mut doc: DocumentMut = text.parse()?;
    section(&mut doc, &["desktop", "keys"])?.insert(action, value(chord));
    Ok(doc.to_string())
}
/// `text` without a custom binding for `action`.
pub fn unset_key(text: &str, action: &str) -> Result<String> {
    let mut doc: DocumentMut = text.parse()?;
    section(&mut doc, &["desktop", "keys"])?.remove(action);
    Ok(doc.to_string())
}

/// Check that `text` is still a valid TERYON config.
pub fn validate(text: &str) -> Result<()> {
    let cfg: Config = toml::from_str(text)?;
    cfg.desktop.validate()?;
    cfg.ai.validate()?;
    cfg.notify.validate()?;
    cfg.host.validate()?;
    Ok(())
}

/// Read the config file (or the legacy one, or nothing), apply `edit`, check the result and write it back
/// atomically with private permissions. Nothing is written when the edit or the check fails.
pub fn apply(edit: impl FnOnce(&str) -> Result<String>) -> Result<()> {
    let path = crate::config::path()?;
    let current = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            crate::config::legacy_text().unwrap_or_default()
        }
        Err(e) => return Err(e.into()),
    };
    let next = edit(&current)?;
    validate(&next)?;
    if next == current {
        return Ok(());
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("toml.tmp");
    let _ = std::fs::remove_file(&tmp);
    crate::util::private_write(&tmp, next.as_bytes())?;
    std::fs::rename(&tmp, &path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    const SAMPLE: &str = "# my notes\ntheme = \"nord\"\n\n[desktop]\n# bigger\nfont_size = 15.0  # tuned\n\n[hosts.mini]\ndestination = \"mini\"\n";
    #[test]
    fn a_value_is_set_and_comments_and_other_sections_survive() {
        let out = set_desktop(SAMPLE, "window_opacity", &Val::Float(0.9399999)).unwrap();
        assert!(out.contains("# my notes") && out.contains("# bigger") && out.contains("# tuned"));
        assert!(out.contains("window_opacity = 0.94"), "{out}");
        assert!(out.contains("[hosts.mini]"));
        let out = set_desktop(&out, "font_size", &Val::Float(16.)).unwrap();
        assert!(
            out.contains("font_size = 16.0") && out.matches("font_size").count() == 1,
            "{out}"
        );
        validate(&out).unwrap();
    }
    #[test]
    fn missing_sections_are_created_and_values_can_be_removed() {
        let out = set_desktop("theme = \"nord\"\n", "copy_on_select", &Val::Bool(true)).unwrap();
        assert!(out.contains("[desktop]") && out.contains("copy_on_select = true"));
        let back = unset_desktop(&out, "copy_on_select").unwrap();
        assert!(!back.contains("copy_on_select"));
        let keys = set_key("", "palette", "cmd+p").unwrap();
        assert!(keys.contains("[desktop.keys]") && keys.contains(r#"palette = "cmd+p""#));
        validate(&keys).unwrap();
        assert!(!unset_key(&keys, "palette").unwrap().contains("palette"));
    }
    #[test]
    fn bad_values_are_rejected_before_anything_is_written() {
        let out = set_desktop(SAMPLE, "font_size", &Val::Float(500.)).unwrap();
        assert!(validate(&out).is_err());
        let dup = set_key("", "palette", "cmd+d").unwrap();
        assert!(validate(&dup).is_err(), "cmd+d is already split_right");
        assert!(set_desktop("[desktop]\nx = 1\n[", "a", &Val::Bool(true)).is_err());
    }
}
