//! Desktop preferences remembered between launches (`~/.teryon/desktop.toml`, mode 0600).
use crate::util;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::{
    fs::OpenOptions,
    io::Write,
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DesktopSettings {
    pub width: Option<f32>,
    pub height: Option<f32>,
    pub theme: Option<String>,
    pub font_size: Option<f32>,
    pub layout: Option<String>,
    /// The first-run hint has been shown.
    pub welcomed: Option<bool>,
}

impl DesktopSettings {
    pub fn path() -> Result<PathBuf> {
        Ok(util::home()?.join(".teryon/desktop.toml"))
    }
    /// Parse settings; anything unreadable or out of range is dropped rather than reported.
    pub fn parse(text: &str) -> Self {
        let mut s: Self = toml::from_str(text).unwrap_or_default();
        s.width = s.width.filter(|w| (320.0..=8000.).contains(w));
        s.height = s.height.filter(|h| (240.0..=8000.).contains(h));
        s.font_size = s.font_size.filter(|f| (9.0..=36.).contains(f));
        s
    }
    pub fn load() -> Self {
        Self::path()
            .and_then(|p| Ok(std::fs::read_to_string(p)?))
            .map(|t| Self::parse(&t))
            .unwrap_or_default()
    }
    /// Atomically replace the file (temp file with 0600, then rename).
    pub fn save_to(&self, path: &Path) -> Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("toml.tmp");
        let mut f = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&tmp)?;
        f.write_all(toml::to_string(self)?.as_bytes())?;
        drop(f);
        std::fs::rename(tmp, path)?;
        Ok(())
    }
    pub fn save(&self) -> Result<()> {
        self.save_to(&Self::path()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn round_trips_and_survives_garbage() {
        let dir = std::env::temp_dir().join(format!("teryon-settings-{}", std::process::id()));
        let path = dir.join("desktop.toml");
        let s = DesktopSettings {
            width: Some(1000.),
            height: Some(700.),
            theme: Some("nord".into()),
            font_size: Some(14.),
            layout: Some("grid".into()),
            welcomed: Some(true),
        };
        s.save_to(&path).unwrap();
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            DesktopSettings::parse(&std::fs::read_to_string(&path).unwrap()),
            s
        );
        assert_eq!(
            DesktopSettings::parse("not = [toml"),
            DesktopSettings::default()
        );
        assert_eq!(
            DesktopSettings::parse("width = 99999.0\nfont_size = 1.0").width,
            None
        );
        std::fs::remove_dir_all(dir).unwrap();
    }
}
