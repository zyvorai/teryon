//! Looking for a newer release: asks GitHub for the latest published release and compares versions.
//! It only tells you (and opens the release page when you ask); installing stays a deliberate step,
//! because the signed, notarized `.dmg`/`.pkg` is what you want to run, not a download the app made itself.
use anyhow::{bail, Context, Result};
use std::process::Command;

const API: &str = "https://api.github.com/repos/zyvorai/teryon/releases/latest";
const PAGE: &str = "https://github.com/zyvorai/teryon/releases";
/// How often the automatic check (`check_updates`) asks, at most.
const EVERY_SECS: u64 = 24 * 3600;

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct Release {
    pub version: String,
    /// The release's page; always on github.com/zyvorai/teryon.
    pub url: String,
}

/// `(major, minor, patch)` of `1.2.3` or `v1.2.3`; pre-release and build suffixes are not versions we offer.
pub fn parse_version(text: &str) -> Option<(u64, u64, u64)> {
    let mut parts = text
        .trim()
        .strip_prefix('v')
        .unwrap_or(text.trim())
        .split('.');
    let v = (
        parts.next()?.parse().ok()?,
        parts.next()?.parse().ok()?,
        parts.next()?.parse().ok()?,
    );
    parts.next().is_none().then_some(v)
}
/// Whether `latest` is a newer release than `current`.
pub fn is_newer(current: &str, latest: &str) -> bool {
    match (parse_version(current), parse_version(latest)) {
        (Some(c), Some(l)) => l > c,
        _ => false,
    }
}
/// The release described by GitHub's "latest release" JSON. Drafts and pre-releases are not offered, and
/// the page link is only trusted when it points at this project.
pub fn parse_release(json: &str) -> Result<Release> {
    let v: serde_json::Value = serde_json::from_str(json).context("not a release")?;
    if v["draft"].as_bool() == Some(true) || v["prerelease"].as_bool() == Some(true) {
        bail!("the latest release is a draft or pre-release");
    }
    let tag = v["tag_name"].as_str().context("no tag_name")?;
    parse_version(tag).with_context(|| format!("`{tag}` is not a version"))?;
    let url = v["html_url"]
        .as_str()
        .filter(|u| u.starts_with("https://github.com/zyvorai/teryon/"))
        .unwrap_or(PAGE);
    Ok(Release {
        version: tag.trim_start_matches('v').to_string(),
        url: url.to_string(),
    })
}

fn fetch(url: &str) -> Result<String> {
    let out = Command::new("curl")
        .args([
            "-fsSL",
            "--max-time",
            "8",
            "-H",
            "Accept: application/vnd.github+json",
        ])
        .args(["-H", "User-Agent: teryon"])
        .arg(url)
        .output()
        .context("start curl")?;
    if !out.status.success() {
        bail!(
            "could not reach {url}: {}",
            String::from_utf8_lossy(&out.stderr)
                .lines()
                .next()
                .unwrap_or("failed")
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// The newest published release, if it is newer than the version running now.
/// `TERYON_UPDATE_URL` points the check somewhere else (used by the tests).
pub fn check() -> Result<Option<Release>> {
    let url = std::env::var("TERYON_UPDATE_URL").unwrap_or_else(|_| API.to_string());
    let release = parse_release(&fetch(&url)?)?;
    Ok(is_newer(env!("CARGO_PKG_VERSION"), &release.version).then_some(release))
}

fn stamp() -> Result<std::path::PathBuf> {
    Ok(crate::util::home()?.join(".teryon/update-check"))
}
/// Whether the automatic check has not asked within the last day.
pub fn due(now: u64) -> bool {
    stamp()
        .ok()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|t| t.trim().parse::<u64>().ok())
        .is_none_or(|last| now.saturating_sub(last) >= EVERY_SECS)
}
/// Remember that a check happened at `now`.
pub fn mark(now: u64) {
    if let Ok(p) = stamp() {
        let _ = crate::util::write_private_atomic(&p, now.to_string().as_bytes());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare_numerically() {
        assert!(is_newer("0.19.0", "v0.20.0"));
        assert!(is_newer("0.9.9", "0.10.0"), "numbers, not text");
        assert!(!is_newer("0.19.0", "0.19.0"));
        assert!(!is_newer("0.19.1", "0.19.0"));
        for odd in ["", "1.2", "1.2.3.4", "1.2.3-rc1", "x.y.z"] {
            assert!(!is_newer("0.1.0", odd), "{odd}");
        }
    }
    #[test]
    fn a_release_is_read_from_github_json() {
        let r = parse_release(
            r#"{"tag_name":"v0.20.0","html_url":"https://github.com/zyvorai/teryon/releases/tag/v0.20.0","draft":false,"prerelease":false}"#,
        )
        .unwrap();
        assert_eq!(r.version, "0.20.0");
        assert!(r.url.ends_with("/tag/v0.20.0"));
        // A link to somewhere else is not trusted.
        let r =
            parse_release(r#"{"tag_name":"v1.0.0","html_url":"https://evil.example/x"}"#).unwrap();
        assert_eq!(r.url, PAGE);
        for bad in [
            "not json",
            r#"{"tag_name":"v1.0.0","prerelease":true}"#,
            r#"{"tag_name":"v1.0.0","draft":true}"#,
            r#"{"tag_name":"nightly"}"#,
            "{}",
        ] {
            assert!(parse_release(bad).is_err(), "{bad}");
        }
    }
}
