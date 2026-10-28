//! Per-directory environment variables: an `[env]` table in a project's `.teryon.toml`, loaded when the
//! shell enters the directory and put back when it leaves. Nothing loads until you run
//! `teryon env allow` in that directory, and any change to the table asks again.
use crate::util;
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

const MAX_VARS: usize = 64;
const MAX_VALUE: usize = 4096;
const MAX_PATHS: usize = 16;
/// Names a project may not set: they change how the shell or the dynamic linker runs everything else.
const BLOCKED: [&str; 11] = [
    "PATH",
    "IFS",
    "ENV",
    "BASH_ENV",
    "PROMPT_COMMAND",
    "ZDOTDIR",
    "SHELL",
    "HOME",
    "LD_PRELOAD",
    "LD_LIBRARY_PATH",
    "CDPATH",
];

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Env {
    pub vars: BTreeMap<String, String>,
    /// Absolute directories put in front of `PATH`.
    pub path_prepend: Vec<String>,
}

pub struct Found {
    pub root: PathBuf,
    pub env: Env,
}

#[derive(Deserialize)]
struct File {
    env: Option<toml::Table>,
}

pub fn valid_name(name: &str) -> bool {
    let mut c = name.chars();
    c.next()
        .is_some_and(|f| f.is_ascii_alphabetic() || f == '_')
        && c.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// The `[env]` table of a `.teryon.toml` (`None` when it has none). `root` is the directory holding the
/// file, which relative `path_prepend` entries are taken from.
pub fn parse(text: &str, root: &Path) -> Result<Option<Env>> {
    let file: File = toml::from_str(text).context("cannot read .teryon.toml")?;
    let Some(table) = file.env else {
        return Ok(None);
    };
    let mut env = Env::default();
    for (key, value) in table {
        if key == "path_prepend" {
            let Some(list) = value.as_array() else {
                bail!("env.path_prepend must be a list of directories");
            };
            if list.len() > MAX_PATHS {
                bail!("env.path_prepend may have at most {MAX_PATHS} entries");
            }
            for item in list {
                let dir = item
                    .as_str()
                    .filter(|d| !d.is_empty() && !d.contains([':', '\0', '\n']))
                    .context("env.path_prepend entries must be directories without ':'")?;
                let abs = if Path::new(dir).is_absolute() {
                    PathBuf::from(dir)
                } else {
                    root.join(dir)
                };
                env.path_prepend.push(abs.to_string_lossy().into_owned());
            }
            continue;
        }
        if !valid_name(&key) {
            bail!("env.{key}: names use letters, digits and _");
        }
        if BLOCKED.contains(&key.as_str())
            || key.starts_with("DYLD_")
            || key.starts_with("__TERYON_")
        {
            bail!(
                "env.{key} cannot be set from a project{}",
                if key == "PATH" {
                    " (use path_prepend)"
                } else {
                    ""
                }
            );
        }
        let text = match value {
            toml::Value::String(s) => s,
            toml::Value::Integer(i) => i.to_string(),
            toml::Value::Boolean(b) => b.to_string(),
            _ => bail!("env.{key} must be a string, number or true/false"),
        };
        if text.len() > MAX_VALUE || text.contains('\0') {
            bail!("env.{key} is too long or contains a NUL");
        }
        env.vars.insert(key, text);
    }
    if env.vars.len() > MAX_VARS {
        bail!("at most {MAX_VARS} variables per project");
    }
    Ok((!env.vars.is_empty() || !env.path_prepend.is_empty()).then_some(env))
}

/// The nearest `.teryon.toml` at or above `start` that has an `[env]` table and belongs to you (a file
/// someone else could have written is ignored).
pub fn find(start: &Path) -> Result<Option<Found>> {
    // SAFETY: geteuid has no preconditions.
    let me = unsafe { libc::geteuid() };
    for dir in start.ancestors() {
        let file = dir.join(".teryon.toml");
        let Ok(meta) = std::fs::metadata(&file) else {
            continue;
        };
        if !meta.is_file() || meta.uid() != me || meta.len() > 1 << 20 {
            continue;
        }
        let text = std::fs::read_to_string(&file)
            .with_context(|| format!("cannot read {}", file.display()))?;
        if let Some(env) = parse(&text, dir).with_context(|| format!("{}", file.display()))? {
            return Ok(Some(Found {
                root: dir.to_path_buf(),
                env,
            }));
        }
    }
    Ok(None)
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub root: String,
    pub env: Env,
}

fn allow_path() -> Result<PathBuf> {
    Ok(util::home()?.join(".teryon/env-allowed.jsonl"))
}
pub fn load_allowed() -> Vec<Entry> {
    allow_path()
        .and_then(|p| Ok(std::fs::read_to_string(p)?))
        .map(|t| {
            t.lines()
                .filter_map(|l| serde_json::from_str(l).ok())
                .collect()
        })
        .unwrap_or_default()
}
fn save_allowed(entries: &[Entry]) -> Result<()> {
    let mut text = String::new();
    for e in entries {
        text.push_str(&serde_json::to_string(e)?);
        text.push('\n');
    }
    util::write_private_atomic(&allow_path()?, text.as_bytes())
}
/// Whether exactly this table was approved for this directory.
pub fn is_allowed(found: &Found, allowed: &[Entry]) -> bool {
    let root = found.root.to_string_lossy();
    allowed.iter().any(|e| e.root == root && e.env == found.env)
}
pub fn allow(found: &Found) -> Result<()> {
    let root = found.root.to_string_lossy().into_owned();
    let mut all = load_allowed();
    all.retain(|e| e.root != root);
    all.push(Entry {
        root,
        env: found.env.clone(),
    });
    save_allowed(&all)
}
/// Forget the approval for `root`; whether there was one.
pub fn deny(root: &Path) -> Result<bool> {
    let root = root.to_string_lossy();
    let mut all = load_allowed();
    let before = all.len();
    all.retain(|e| e.root != root);
    save_allowed(&all)?;
    Ok(all.len() != before)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shell {
    Posix,
    Fish,
}

fn quote_fish(v: &str) -> String {
    format!("'{}'", v.replace('\\', "\\\\").replace('\'', "\\'"))
}

/// A short stand-in for "which table of which directory is loaded", to notice changes cheaply.
pub fn key(found: &Found) -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    let json = serde_json::to_string(&found.env).unwrap_or_default();
    for b in found.root.to_string_lossy().bytes().chain(json.bytes()) {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{:016x}", h)
}

/// Shell code to put back what a loaded table changed (`names` as the shell remembered them).
pub fn unload(shell: Shell, names: &[String]) -> String {
    let mut out = String::new();
    for n in names.iter().filter(|n| valid_name(n)) {
        match shell {
            Shell::Posix => out.push_str(&format!(
                "if [ -n \"$__TERYON_HAD_{n}\" ]; then export {n}=\"$__TERYON_OLD_{n}\"; else unset {n}; fi; unset __TERYON_OLD_{n} __TERYON_HAD_{n}\n"
            )),
            Shell::Fish => out.push_str(&format!(
                "if set -q __TERYON_HAD_{n}; set -gx {n} $__TERYON_OLD_{n}; else; set -e {n}; end; set -e __TERYON_OLD_{n} __TERYON_HAD_{n}\n"
            )),
        }
    }
    match shell {
        Shell::Posix => out.push_str("unset __TERYON_ENV_KEY __TERYON_ENV_NAMES\n"),
        Shell::Fish => out.push_str("set -e __TERYON_ENV_KEY __TERYON_ENV_NAMES\n"),
    }
    out
}

/// Shell code that sets what `found` describes, remembering what it replaces.
pub fn load(shell: Shell, found: &Found) -> String {
    let mut names: Vec<&str> = found.env.vars.keys().map(String::as_str).collect();
    if !found.env.path_prepend.is_empty() {
        names.push("PATH");
    }
    let save = |n: &str| {
        match shell {
        Shell::Posix => format!(
            "if [ -n \"${{{n}+x}}\" ]; then __TERYON_OLD_{n}=\"${n}\"; __TERYON_HAD_{n}=1; else __TERYON_HAD_{n}=; fi\n"
        ),
        Shell::Fish => format!(
            "if set -q {n}; set -g __TERYON_OLD_{n} ${n}; set -g __TERYON_HAD_{n} 1; else; set -e __TERYON_HAD_{n}; end\n"
        ),
    }
    };
    let mut out = String::new();
    for (n, v) in &found.env.vars {
        out.push_str(&save(n));
        match shell {
            Shell::Posix => out.push_str(&format!("export {n}={}\n", util::shell_quote(v))),
            Shell::Fish => out.push_str(&format!("set -gx {n} {}\n", quote_fish(v))),
        }
    }
    if !found.env.path_prepend.is_empty() {
        out.push_str(&save("PATH"));
        match shell {
            Shell::Posix => {
                let dirs = found.env.path_prepend.join(":");
                out.push_str(&format!(
                    "export PATH={}\":$PATH\"\n",
                    util::shell_quote(&dirs)
                ));
            }
            Shell::Fish => {
                let dirs: Vec<String> = found
                    .env
                    .path_prepend
                    .iter()
                    .map(|d| quote_fish(d))
                    .collect();
                out.push_str(&format!("set -gx PATH {} $PATH\n", dirs.join(" ")));
            }
        }
    }
    let (k, ns) = (key(found), names.join(" "));
    match shell {
        Shell::Posix => out.push_str(&format!(
            "export __TERYON_ENV_KEY={}\nexport __TERYON_ENV_NAMES={}\n",
            util::shell_quote(&k),
            util::shell_quote(&ns)
        )),
        Shell::Fish => out.push_str(&format!(
            "set -gx __TERYON_ENV_KEY {}\nset -gx __TERYON_ENV_NAMES {}\n",
            quote_fish(&k),
            quote_fish(&ns)
        )),
    }
    out
}

/// What a shell should run after changing to `cwd`, given the key and names it remembers. Messages for
/// the user go in the second field. Empty script: nothing to do.
pub fn hook(
    shell: Shell,
    cwd: &Path,
    loaded_key: &str,
    loaded_names: &[String],
    allowed: &[Entry],
) -> (String, Vec<String>) {
    let mut notes = Vec::new();
    let found = match find(cwd) {
        Ok(f) => f,
        Err(e) => {
            notes.push(format!("teryon: {e:#}"));
            None
        }
    };
    let wanted = match found {
        Some(f) if is_allowed(&f, allowed) => Some(f),
        Some(f) => {
            if loaded_key != key(&f) {
                notes.push(format!(
                    "teryon: {} sets {} variable(s) for this project; run `teryon env allow` to load them",
                    f.root.join(".teryon.toml").display(),
                    f.env.vars.len() + usize::from(!f.env.path_prepend.is_empty())
                ));
            }
            None
        }
        None => None,
    };
    let unchanged = match &wanted {
        Some(f) => loaded_key == key(f),
        None => loaded_key.is_empty(),
    };
    if unchanged {
        return (String::new(), notes);
    }
    let mut script = String::new();
    if !loaded_key.is_empty() {
        script.push_str(&unload(shell, loaded_names));
    }
    if let Some(f) = &wanted {
        script.push_str(&load(shell, f));
        notes.push(format!(
            "teryon: loaded {} variable(s) from {}",
            f.env.vars.len() + usize::from(!f.env.path_prepend.is_empty()),
            f.root.join(".teryon.toml").display()
        ));
    }
    (script, notes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    fn root() -> PathBuf {
        PathBuf::from("/proj")
    }
    fn found(text: &str) -> Found {
        Found {
            root: root(),
            env: parse(text, &root()).unwrap().unwrap(),
        }
    }

    #[test]
    fn the_env_table_gives_variables_and_absolute_path_entries() {
        let f = found("[env]\nAPI = \"http://x\"\nDEBUG = true\nN = 3\npath_prepend = [\"bin\", \"/opt/tool\"]\n");
        assert_eq!(f.env.vars["API"], "http://x");
        assert_eq!(f.env.vars["DEBUG"], "true");
        assert_eq!(f.env.vars["N"], "3");
        assert_eq!(f.env.path_prepend, ["/proj/bin", "/opt/tool"]);
        assert_eq!(parse("[[pane]]\nname = \"a\"\n", &root()).unwrap(), None);
        assert_eq!(parse("[env]\n", &root()).unwrap(), None);
    }

    #[test]
    fn dangerous_and_malformed_entries_are_refused() {
        for bad in [
            "[env]\nPATH = \"/x\"\n",
            "[env]\nLD_PRELOAD = \"/x\"\n",
            "[env]\nDYLD_INSERT_LIBRARIES = \"/x\"\n",
            "[env]\n\"a b\" = \"x\"\n",
            "[env]\n__TERYON_ENV_KEY = \"x\"\n",
            "[env]\nA = [1]\n",
            "[env]\npath_prepend = \"bin\"\n",
            "[env]\npath_prepend = [\"a:b\"]\n",
        ] {
            assert!(parse(bad, &root()).is_err(), "{bad}");
        }
    }

    #[test]
    fn approval_covers_exactly_the_approved_table() {
        let a = found("[env]\nX = \"1\"\n");
        let entry = Entry {
            root: "/proj".into(),
            env: a.env.clone(),
        };
        assert!(is_allowed(&a, std::slice::from_ref(&entry)));
        assert!(!is_allowed(
            &found("[env]\nX = \"2\"\n"),
            std::slice::from_ref(&entry)
        ));
        assert!(!is_allowed(&a, &[]));
    }

    /// Run `script` pieces in one sh and print the variables.
    fn sh(pieces: &[&str]) -> String {
        let out = Command::new("sh")
            .arg("-c")
            .arg(pieces.join("\n"))
            .env_remove("KEEP")
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    #[test]
    fn loading_sets_variables_and_unloading_restores_what_was_there() {
        let f = found("[env]\nKEEP = \"it's $mine\"\nNEWVAR = \"v\"\npath_prepend = [\"bin\"]\n");
        let names = vec!["KEEP".to_string(), "NEWVAR".to_string(), "PATH".to_string()];
        let out = sh(&[
            "KEEP=before; PATH=/usr/bin:/bin",
            &load(Shell::Posix, &f),
            "echo \"in:$KEEP|$NEWVAR|$PATH|$__TERYON_ENV_NAMES\"",
            &unload(Shell::Posix, &names),
            "echo \"out:$KEEP|${NEWVAR-unset}|$PATH|${__TERYON_ENV_KEY-gone}\"",
        ]);
        assert_eq!(
            out,
            "in:it's $mine|v|/proj/bin:/usr/bin:/bin|KEEP NEWVAR PATH\nout:before|unset|/usr/bin:/bin|gone\n"
        );
    }

    #[test]
    fn the_hook_loads_when_allowed_asks_when_not_and_unloads_on_leaving() {
        let dir = tempfile::tempdir().unwrap();
        let proj = dir.path().canonicalize().unwrap();
        std::fs::write(proj.join(".teryon.toml"), "[env]\nA = \"1\"\n").unwrap();
        let sub = proj.join("deep");
        std::fs::create_dir(&sub).unwrap();
        let f = find(&sub).unwrap().unwrap();
        assert_eq!(f.root, proj);

        let (script, notes) = hook(Shell::Posix, &sub, "", &[], &[]);
        assert!(
            script.is_empty() && notes[0].contains("teryon env allow"),
            "{notes:?}"
        );

        let allowed = vec![Entry {
            root: proj.to_string_lossy().into_owned(),
            env: f.env.clone(),
        }];
        let (script, notes) = hook(Shell::Posix, &sub, "", &[], &allowed);
        assert!(script.contains("export A='1'") && notes[0].contains("loaded 1 variable"));
        let k = key(&f);
        assert_eq!(
            hook(Shell::Posix, &sub, &k, &["A".into()], &allowed).0,
            "",
            "unchanged"
        );

        let outside = tempfile::tempdir().unwrap();
        let (script, _) = hook(Shell::Posix, outside.path(), &k, &["A".into()], &allowed);
        assert!(script.contains("else unset A;") && !script.contains("export A='1'"));
    }

    #[test]
    fn fish_code_uses_fish_syntax() {
        let f = found("[env]\nX = \"a'b\"\npath_prepend = [\"bin\"]\n");
        let text = load(Shell::Fish, &f);
        assert!(
            text.contains("set -gx X 'a\\'b'") && text.contains("set -gx PATH '/proj/bin' $PATH")
        );
        assert!(unload(Shell::Fish, &["X".into()]).contains("set -e X"));
    }
}
