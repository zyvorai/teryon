use anyhow::{bail, Context, Result};
use clap::{CommandFactory, Parser, Subcommand};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    thread,
    time::{Duration, Instant},
};
use teryon::{
    brand, config, daemon,
    protocol::*,
    theme::Theme,
    transport::{Rpc, Target},
    ui,
};

#[derive(Parser)]
#[command(
    version,
    about = "TERYON — your remote workspace survives the connection",
    long_about = "Persistent macOS/Linux terminal workspaces over OpenSSH. A per-user daemon owns shells on the remote machine. Network reconnection never replays pending input."
)]
struct Cli {
    #[arg(
        long,
        global = true,
        help = "Absolute path to the remote teryon binary"
    )]
    remote_bin: Option<String>,
    /// With no command: open the desktop app instead of the terminal UI after picking a host.
    #[arg(long)]
    desktop: bool,
    #[command(subcommand)]
    command: Option<Commands>,
}
#[derive(Subcommand)]
enum Commands {
    /// Open the native GPU desktop application (build with --features desktop).
    #[cfg(feature = "desktop")]
    Desktop {
        #[arg(default_value = "local")]
        host: String,
        #[arg(short, long, default_value = "default")]
        workspace: String,
        #[arg(long)]
        readonly: bool,
        #[arg(long)]
        cwd: Option<String>,
        #[arg(long)]
        theme: Option<String>,
    },
    /// Open the colorful terminal workspace (Ctrl+Space then ? for help).
    Connect {
        #[arg(default_value = "local")]
        host: String,
        #[arg(short, long, default_value = "default")]
        workspace: String,
        #[arg(long)]
        readonly: bool,
        #[arg(long)]
        cwd: Option<String>,
        #[arg(long)]
        theme: Option<String>,
        /// Skip the startup banner.
        #[arg(long)]
        no_splash: bool,
        /// If this workspace is brand new, reopen the panes (directories and names) the desktop app last saved for it.
        #[arg(long)]
        restore: bool,
    },
    /// List persistent remote workspaces.
    Sessions {
        #[arg(default_value = "local")]
        host: String,
    },
    /// Remove a detached workspace; live processes require explicit --force.
    Remove {
        #[arg(default_value = "local")]
        host: String,
        #[arg(short, long)]
        workspace: String,
        #[arg(long)]
        force: bool,
    },
    /// List panes in a workspace without taking keyboard control.
    Panes {
        #[arg(default_value = "local")]
        host: String,
        #[arg(short, long, default_value = "default")]
        workspace: String,
    },
    /// Export retained plain terminal output to stdout.
    Capture {
        #[arg(default_value = "local")]
        host: String,
        #[arg(short, long, default_value = "default")]
        workspace: String,
        #[arg(long, default_value_t = 1)]
        pane: u64,
    },
    /// Record a pane to an asciicast v2 file (screen snapshots; play with `asciinema play`).
    Record {
        #[arg(default_value = "local")]
        host: String,
        #[arg(short, long, default_value = "default")]
        workspace: String,
        #[arg(long, default_value_t = 1)]
        pane: u64,
        /// Output .cast file; must not exist.
        #[arg(short, long)]
        output: PathBuf,
        /// Stop after this many seconds (default: until the pane exits or Ctrl-C).
        #[arg(long)]
        seconds: Option<u64>,
    },
    /// Run a durable task in a new pane. Arguments are passed without a shell.
    Exec {
        #[arg(default_value = "local")]
        host: String,
        #[arg(short, long, default_value = "tasks")]
        workspace: String,
        #[arg(long)]
        cwd: Option<String>,
        #[arg(long)]
        wait: bool,
        #[arg(long, default_value_t = 3600)]
        timeout: u64,
        /// Ring the bell and show a desktop notification when the task finishes (needs --wait).
        #[arg(long, requires = "wait")]
        notify: bool,
        #[arg(required = true, last = true)]
        command: Vec<String>,
    },
    /// Send explicitly supplied input; the workspace must have no other controller.
    Send {
        host: String,
        #[arg(short, long, default_value = "default")]
        workspace: String,
        #[arg(long, default_value_t = 1)]
        pane: u64,
        #[arg(long)]
        enter: bool,
        text: String,
    },
    /// Forward a loopback port, e.g. 3000:3000. Runs in foreground until Ctrl+C.
    Forward {
        host: String,
        ports: String,
        #[arg(long)]
        reconnect: bool,
    },
    /// Resumable upload using rsync 3+ (required on both machines).
    Push {
        host: String,
        source: String,
        destination: String,
        #[arg(long)]
        limit_kbps: Option<u32>,
    },
    /// Resumable download using rsync 3+ (required on both machines).
    Pull {
        host: String,
        source: String,
        destination: String,
        #[arg(long)]
        limit_kbps: Option<u32>,
    },
    /// Upload and verify a binary built for the remote OS/architecture.
    Deploy {
        host: String,
        #[arg(long)]
        binary: PathBuf,
    },
    /// Verify daemon reachability and report its actual platform.
    Doctor {
        #[arg(default_value = "local")]
        host: String,
        /// Upload --binary and retry when the remote daemon cannot be reached.
        #[arg(long, requires = "binary")]
        fix: bool,
        /// Binary built for the remote OS/architecture (used only with --fix).
        #[arg(long)]
        binary: Option<PathBuf>,
        /// Do not ask before deploying.
        #[arg(long, short = 'y')]
        yes: bool,
    },
    /// Print the shell integration script (OSC 133 prompt marks + OSC 7 cwd) for zsh, bash or fish.
    ShellInit {
        #[arg(value_parser = ["zsh", "bash", "fish"])]
        shell: String,
    },
    /// Command history behind the shell integration (used by the zsh widgets).
    #[command(hide = true, subcommand)]
    Hist(HistCommand),
    /// Colour spans for a command line, as `START END STYLE` lines (used by the zsh widgets).
    #[command(hide = true)]
    Highlight {
        #[arg(last = true)]
        line: Vec<String>,
    },
    /// Fuzzy picker for shell widgets: history, files, directories or lines from stdin.
    #[command(hide = true)]
    Pick {
        /// Pick from command history.
        #[arg(long, conflicts_with_all = ["files", "dirs"])]
        history: bool,
        /// Pick a file below --cwd.
        #[arg(long, conflicts_with = "dirs")]
        files: bool,
        /// Pick a directory below --cwd.
        #[arg(long)]
        dirs: bool,
        #[arg(long, default_value = ".")]
        cwd: PathBuf,
        /// Initial query.
        #[arg(long, default_value = "")]
        query: String,
    },
    /// A two-line zsh prompt: directory, git, language, last command's duration and status.
    #[command(hide = true)]
    Prompt {
        #[arg(long, default_value_t = 0)]
        status: i32,
        #[arg(long, default_value_t = 0)]
        ms: u64,
        #[arg(long, default_value = ".")]
        cwd: String,
    },
    /// Frequently used directories behind `z` (used by the zsh functions).
    #[command(hide = true, subcommand)]
    Jump(JumpCommand),
    /// Guided first-run setup: tools, SSH hosts, reachability, theme.
    Setup {
        /// Accept every default without asking.
        #[arg(long, short = 'y')]
        yes: bool,
    },
    /// Chat with Claude and let it open apps or SSH terminals (asks before anything changes). Needs ANTHROPIC_API_KEY.
    Ai {
        /// What to ask; leave empty for an interactive chat.
        prompt: Vec<String>,
        /// Store your API key in the macOS Keychain (the secret is typed into `security`, never into TERYON).
        #[arg(long)]
        set_key: bool,
        /// Verify the key with one tiny request.
        #[arg(long)]
        check: bool,
        /// Turn the request into one shell command and print it (nothing is run).
        #[arg(long)]
        command: bool,
    },
    /// Model Context Protocol server: lets Claude Code, Codex and other agents use persistent TERYON panes.
    Mcp {
        /// Host used when a tool call names none (`local`, a configured host or an SSH alias).
        #[arg(long, default_value = "local")]
        host: String,
        /// The workspace this server owns: the only one it writes to.
        #[arg(long, short, default_value = "mcp")]
        workspace: String,
        /// Allow new_pane, send_keys and close_pane (reading is always allowed).
        #[arg(long)]
        allow_write: bool,
        /// Do not redact secrets from terminal text returned to the agent.
        #[arg(long)]
        no_redact: bool,
        /// Print the setup line for Claude Code and a Claude Desktop JSON snippet, then exit.
        #[arg(long)]
        print_config: bool,
    },
    /// Show the desktop configuration, or validate it with --check.
    Config {
        /// Exit with an error if `[desktop]` in config.toml is invalid.
        #[arg(long)]
        check: bool,
        /// List every desktop action with its key binding (defaults plus your overrides).
        #[arg(long, conflicts_with = "check")]
        keys: bool,
    },
    /// List themes or display their color samples.
    Themes {
        #[arg(long)]
        preview: bool,
        /// Import a color scheme (iTerm2 .itermcolors, Alacritty .toml, Ghostty or Kitty config) as a theme.
        #[arg(long, value_name = "FILE", conflicts_with_all = ["preview", "export"])]
        import: Option<PathBuf>,
        /// Name for the imported theme (default: the file name).
        #[arg(long = "as", value_name = "NAME", requires = "import")]
        import_as: Option<String>,
        /// Replace an existing imported theme of the same name.
        #[arg(long, requires = "import")]
        force: bool,
        /// Print a terminal color scheme for NAME (a built-in or a theme file); only `wezterm` is supported.
        #[arg(long, value_name = "TERMINAL", requires = "name", value_parser = ["wezterm"])]
        export: Option<String>,
        /// Theme to export.
        name: Option<String>,
    },
    /// Print a shell completion script (bash, zsh, fish, elvish, powershell).
    Completions { shell: clap_complete::Shell },
    /// Start the workspace a project describes in `.teryon.toml`: its panes, directories and commands.
    Up {
        /// Host to start it on.
        #[arg(default_value = "local")]
        host: String,
        /// The recipe file.
        #[arg(long, default_value = ".teryon.toml")]
        file: PathBuf,
        /// Workspace name (default: the recipe's, else the directory's name).
        #[arg(long)]
        workspace: Option<String>,
    },
    /// Wake a sleeping host with a wake-on-LAN packet (set `mac` on the host in the config).
    Wake {
        /// A configured host alias.
        host: String,
    },
    /// Add `Host` aliases from ~/.ssh/config to the TERYON config (never edits SSH config or copies keys).
    ImportSsh {
        /// OpenSSH config to read.
        #[arg(long)]
        file: Option<PathBuf>,
        /// Show what would be added without writing.
        #[arg(long)]
        dry_run: bool,
    },
    /// Shorthand for `connect local --workspace NAME` (or another host with --host).
    Attach {
        workspace: String,
        #[arg(long, default_value = "local")]
        host: String,
        #[arg(long)]
        readonly: bool,
        #[arg(long)]
        theme: Option<String>,
        #[arg(long)]
        no_splash: bool,
    },
    /// List the AI coding agents (Claude Code, Codex, ...) in every workspace and whether each is working or waiting for you.
    Agents {
        /// Hosts to look at (default: local).
        hosts: Vec<String>,
        /// Look at this machine and every configured host.
        #[arg(long)]
        all: bool,
    },
    /// Overview of every configured host: reachability, system, workspaces and panes.
    Dashboard {
        /// Include this machine (starts the local service if needed).
        #[arg(long)]
        local: bool,
        /// Refresh every SECS seconds until Ctrl+C.
        #[arg(long, value_name = "SECS")]
        watch: Option<u64>,
    },
    /// Show the TERYON banner, version and active theme.
    About {
        #[arg(long)]
        theme: Option<String>,
    },
    /// Write a starter config without overwriting an existing file.
    Init,
    /// Save pane directories/titles, or restore them as NEW shells (no command replay).
    Layout {
        #[command(subcommand)]
        action: LayoutCommand,
    },
    /// Stop the daemon. Live panes require --force and will be terminated.
    Stop {
        #[arg(default_value = "local")]
        host: String,
        #[arg(long)]
        force: bool,
    },
    /// Run the per-user session daemon in foreground (normally started automatically).
    Daemon,
    /// Internal SSH stdio bridge. Do not print banners on its stdout.
    #[command(hide = true)]
    Relay,
}
#[derive(Subcommand)]
enum HistCommand {
    /// Record one finished command.
    Add {
        #[arg(long, default_value_t = 0)]
        exit: i32,
        #[arg(long, default_value_t = 0)]
        ms: u64,
        #[arg(long, default_value = "")]
        cwd: String,
        #[arg(last = true)]
        cmd: Vec<String>,
    },
    /// Print the command to ghost after PREFIX (nothing if none).
    Suggest {
        #[arg(long, default_value = "")]
        cwd: String,
        #[arg(last = true)]
        prefix: Vec<String>,
    },
}
#[derive(Subcommand)]
enum JumpCommand {
    /// Record a visit to PATH.
    Add { path: String },
    /// Print the best directory for the keywords (exit 1 if none).
    Query {
        #[arg(long, default_value = "")]
        cwd: String,
        #[arg(last = true)]
        keywords: Vec<String>,
    },
    /// Print every known directory, best first.
    List {
        #[arg(long, default_value = "")]
        cwd: String,
        #[arg(last = true)]
        keywords: Vec<String>,
    },
}
#[derive(Subcommand)]
enum LayoutCommand {
    Save {
        host: String,
        #[arg(short, long, default_value = "default")]
        workspace: String,
        file: PathBuf,
    },
    Open {
        host: String,
        #[arg(short, long)]
        workspace: String,
        file: PathBuf,
    },
}
fn target(host: String, remote_bin: Option<String>) -> Result<Target> {
    config::target(&host, remote_bin)
}
/// Bell + OSC 9 for terminals, plus a macOS notification (message passed as argv, never interpolated).
fn notify_done(label: &str, code: u32) {
    let short: String = label.chars().take(60).collect();
    let message = if code == 0 {
        format!("Task finished: {short}")
    } else {
        format!("Task failed ({code}): {short}")
    };
    eprint!("\x07\x1b]9;{message}\x07");
    teryon::util::notify(&message);
}
/// Open the desktop app; command line beats the remembered theme, which beats the config file.
#[cfg(feature = "desktop")]
fn run_desktop(
    host: String,
    workspace: String,
    readonly: bool,
    cwd: Option<String>,
    theme: Option<String>,
    bin: Option<String>,
) -> Result<()> {
    let cfg = config::load()?;
    // A fresh window gets a workspace of its own, unless one was asked for by name.
    let fresh = cfg.desktop.fresh_window && workspace == "default" && !readonly;
    let workspace = if fresh {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        if let Ok(t) = target(host.clone(), bin.clone()) {
            remove_orphan_windows(&t);
        }
        format!("win-{now}-{}", std::process::id())
    } else {
        teryon::recent::record(&host, &workspace);
        workspace
    };
    let settings = teryon::settings::DesktopSettings::load();
    let name = theme
        .or_else(|| settings.theme.clone())
        .unwrap_or_else(|| cfg.theme.clone());
    let theme = Theme::load_native(&name)?;
    teryon::desktop::run(
        target(host, bin)?,
        workspace,
        readonly,
        cwd,
        theme,
        settings,
        fresh,
    )
}
#[cfg(feature = "desktop")]
/// Fresh-window workspaces left behind by a crash: named `win-*` and with nobody attached.
fn remove_orphan_windows(target: &Target) {
    let Ok(mut rpc) = target.connect() else {
        return;
    };
    let Ok(Response::Workspaces { items }) = rpc.call(Request::List) else {
        return;
    };
    for w in items
        .into_iter()
        .filter(|w| w.name.starts_with("win-") && !w.controlled)
    {
        let _ = rpc.call(Request::Remove {
            workspace: w.name,
            force: true,
        });
    }
}
/// Plain `teryon`: pick a host/workspace (recents, config, ~/.ssh/config) and connect.
fn start(bin: Option<String>, want_desktop: bool) -> Result<i32> {
    use std::io::IsTerminal;
    if !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
        Cli::command().print_help()?;
        println!();
        return Ok(2);
    }
    let cfg = config::load()?;
    let theme = Theme::load(&cfg.theme)?;
    let hosts: Vec<String> = cfg.hosts.keys().cloned().collect();
    let ssh = teryon::util::home()
        .ok()
        .and_then(|h| std::fs::read_to_string(h.join(".ssh/config")).ok())
        .map(|t| config::ssh_hosts(&t))
        .unwrap_or_default();
    let entries = teryon::start::build_entries(&hosts, &teryon::recent::load(), &ssh);
    let lister_bin = bin.clone();
    let mut lister = move |host: &str| -> Result<Vec<String>> {
        let mut rpc = target(host.to_string(), lister_bin.clone())?.connect()?;
        match rpc.call(Request::List)? {
            Response::Workspaces { items } => Ok(items.into_iter().map(|w| w.name).collect()),
            _ => bail!("unexpected response"),
        }
    };
    let mut waker = |host: &str| -> Result<String> {
        let cfg = config::load()?;
        let h = cfg
            .hosts
            .get(host)
            .ok_or_else(|| anyhow::anyhow!("not a configured host"))?;
        let mac = h
            .mac
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("no `mac` set in [hosts.{host}]"))?;
        teryon::wake::wake(mac, h.broadcast.as_deref())?;
        Ok(format!(
            "Sent a wake packet to {host}; give it a few seconds"
        ))
    };
    let Some(choice) = teryon::start::pick(entries, &theme, &mut lister, &mut waker)? else {
        return Ok(0);
    };
    let desktop = want_desktop || cfg.start == config::StartMode::Desktop;
    if desktop {
        #[cfg(feature = "desktop")]
        {
            run_desktop(choice.host, choice.workspace, false, None, None, bin)?;
            return Ok(0);
        }
        #[cfg(not(feature = "desktop"))]
        bail!("this build has no desktop app; rebuild with --features desktop");
    }
    teryon::recent::record(&choice.host, &choice.workspace);
    connect_tui(
        target(choice.host, bin)?,
        choice.workspace,
        false,
        None,
        None,
        false,
    )?;
    Ok(0)
}
/// Rename pane 1 and create the remaining panes of `layout` (directories and titles only).
fn apply_layout(rpc: &mut Rpc, layout: &teryon::layout::Layout, first_pane: u64) -> Result<()> {
    for (i, p) in layout.panes.iter().enumerate() {
        let id = if i == 0 {
            first_pane
        } else {
            let Response::Created { pane } = rpc.call(Request::NewPane {
                cwd: Some(p.cwd.clone()),
                command: vec![],
                rows: 24,
                cols: 100,
            })?
            else {
                bail!("unexpected response");
            };
            pane
        };
        rpc.call(Request::Rename {
            pane: id,
            title: p.title.clone(),
        })?;
    }
    Ok(())
}
/// `connect --restore`: set up a fresh workspace from the layout the desktop app saved for it.
fn restore_saved_layout(host: &str, workspace: &str, bin: Option<String>) -> Result<()> {
    let path = teryon::layout::auto_path(host, workspace)?;
    let Ok(layout) = teryon::layout::load(&path) else {
        eprintln!("No saved layout for {host} / {workspace}; starting normally.");
        return Ok(());
    };
    let mut rpc = attach(
        &target(host.to_string(), bin)?,
        workspace.to_string(),
        false,
        None,
    )?;
    let views = panes(&mut rpc)?;
    if !teryon::layout::is_fresh(&views) || layout.panes.len() < 2 {
        eprintln!("This workspace is already in use; nothing restored.");
        return Ok(());
    }
    apply_layout(&mut rpc, &layout, views[0].id)?;
    eprintln!(
        "Restored {} panes (directories and names only).",
        layout.panes.len()
    );
    Ok(())
}
fn connect_tui(
    target: Target,
    workspace: String,
    readonly: bool,
    cwd: Option<String>,
    theme: Option<String>,
    no_splash: bool,
) -> Result<()> {
    let cfg = config::load()?;
    let theme = Theme::load(theme.as_deref().unwrap_or(&cfg.theme))?;
    if brand::splash_wanted(no_splash || !cfg.splash) {
        print!("{}", brand::banner(&theme, true));
        thread::sleep(Duration::from_millis(700));
    }
    ui::run(target, workspace, readonly, cwd, theme, cfg.mouse)
}
/// Ask a yes/no question on the terminal; anything but y/yes is a no.
fn confirm(question: &str) -> Result<bool> {
    use std::io::Write;
    eprint!("{question} [y/N] ");
    std::io::stderr().flush()?;
    let mut line = String::new();
    std::io::stdin().read_line(&mut line)?;
    Ok(matches!(
        line.trim().to_ascii_lowercase().as_str(),
        "y" | "yes"
    ))
}
fn attach(target: &Target, workspace: String, readonly: bool, cwd: Option<String>) -> Result<Rpc> {
    let mut rpc = target.connect()?;
    rpc.call(Request::Attach {
        workspace,
        readonly,
        rows: 24,
        cols: 100,
        cwd,
        takeover: false,
    })?;
    Ok(rpc)
}
fn panes(rpc: &mut Rpc) -> Result<Vec<PaneView>> {
    if let Response::State { panes } = rpc.call(Request::Poll {
        known: BTreeMap::new(),
        diff: false,
    })? {
        Ok(panes)
    } else {
        bail!("unexpected response")
    }
}
fn capture(rpc: &mut Rpc, pane: u64) -> Result<String> {
    if let Response::Captured { text, .. } = rpc.call(Request::Capture { pane })? {
        Ok(text)
    } else {
        bail!("unexpected capture response")
    }
}
fn run() -> Result<i32> {
    let cli = Cli::parse();
    let bin = cli.remote_bin;
    let Some(command) = cli.command else {
        return start(bin, cli.desktop);
    };
    match command {
        #[cfg(feature = "desktop")]
        Commands::Desktop {
            host,
            workspace,
            readonly,
            cwd,
            theme,
        } => run_desktop(host, workspace, readonly, cwd, theme, bin)?,
        Commands::Daemon => daemon::run()?,
        Commands::Relay => daemon::relay()?,
        Commands::About { theme } => {
            let cfg = config::load()?;
            let theme = Theme::load(theme.as_deref().unwrap_or(&cfg.theme))?;
            print!("{}", brand::banner(&theme, brand::color_wanted()));
            println!(
                "teryon {} · theme {}",
                env!("CARGO_PKG_VERSION"),
                theme.name
            );
        }
        Commands::Init => {
            let path = config::init()?;
            if brand::splash_wanted(false) {
                print!(
                    "{}",
                    brand::banner(&Theme::load(&config::load()?.theme)?, true)
                );
            }
            println!("Created {}", path.display());
        }
        Commands::Connect {
            host,
            workspace,
            readonly,
            cwd,
            theme,
            no_splash,
            restore,
        } => {
            teryon::recent::record(&host, &workspace);
            if restore && !readonly {
                restore_saved_layout(&host, &workspace, bin.clone())?;
            }
            connect_tui(
                target(host, bin)?,
                workspace,
                readonly,
                cwd,
                theme,
                no_splash,
            )?
        }
        Commands::Attach {
            workspace,
            host,
            readonly,
            theme,
            no_splash,
        } => {
            teryon::recent::record(&host, &workspace);
            connect_tui(
                target(host, bin)?,
                workspace,
                readonly,
                None,
                theme,
                no_splash,
            )?
        }
        Commands::Agents { hosts, all } => {
            let mut names = hosts;
            if all {
                names.push("local".into());
                names.extend(config::load()?.hosts.keys().cloned());
            } else if names.is_empty() {
                names.push("local".into());
            }
            names.dedup();
            let mut backend = teryon::mcp::Real {
                remote_bin: bin.clone(),
            };
            let mut shown = 0;
            for host in names {
                use teryon::mcp::Backend;
                let workspaces = match backend.workspaces(&host) {
                    Ok(w) => w,
                    Err(e) => {
                        eprintln!("{host}: unreachable ({e:#})");
                        continue;
                    }
                };
                for w in workspaces {
                    for (id, kind, status) in
                        teryon::mcp::agent_rows(&mut backend, &host, &w.name).unwrap_or_default()
                    {
                        println!(
                            "{host}\t{}\tpane {id}\t{}\t{}",
                            w.name,
                            kind.name(),
                            status.label()
                        );
                        shown += 1;
                    }
                }
            }
            if shown == 0 {
                println!("No AI agents running.");
            }
        }
        Commands::Dashboard { local, watch } => {
            let cfg = config::load()?;
            loop {
                let mut hosts: Vec<(String, Option<String>)> = cfg
                    .hosts
                    .iter()
                    .map(|(name, h)| (name.clone(), h.remote_bin.clone().or(bin.clone())))
                    .collect();
                if local {
                    hosts.insert(0, ("local".into(), bin.clone()));
                }
                let rows: Vec<_> = thread::scope(|s| {
                    let jobs: Vec<_> = hosts
                        .iter()
                        .map(|(name, remote_bin)| {
                            s.spawn(move || {
                                teryon::dashboard::probe(
                                    name,
                                    target(name.clone(), remote_bin.clone()),
                                )
                            })
                        })
                        .collect();
                    jobs.into_iter().filter_map(|j| j.join().ok()).collect()
                });
                if watch.is_some() {
                    print!("\x1b[2J\x1b[H");
                }
                if rows.is_empty() {
                    println!("No hosts configured. Run `teryon init` or `teryon import-ssh`, or use --local.");
                } else {
                    print!(
                        "{}",
                        teryon::dashboard::format_table(&rows, brand::color_wanted())
                    );
                }
                match watch {
                    Some(secs) => thread::sleep(Duration::from_secs(secs.max(1))),
                    None => break,
                }
            }
        }
        Commands::Completions { shell } => {
            clap_complete::generate(shell, &mut Cli::command(), "teryon", &mut std::io::stdout())
        }
        Commands::Up {
            host,
            file,
            workspace,
        } => {
            let text = std::fs::read_to_string(&file)
                .map_err(|e| anyhow::anyhow!("cannot read {}: {e}", file.display()))?;
            let recipe = teryon::recipe::parse(&text)?;
            let base = std::fs::canonicalize(&file)?
                .parent()
                .map(std::path::Path::to_path_buf)
                .unwrap_or_default();
            let name = teryon::recipe::workspace_name(&recipe, workspace.as_deref(), &base)?;
            let mut rpc = target(host.clone(), bin)?.connect()?;
            let ids = teryon::recipe::up_with(
                &mut |r| rpc.call(r),
                &recipe,
                &name,
                &base,
                host == "local",
                &mut |line| eprintln!("{line}…"),
            )?;
            println!(
                "Started {} pane(s) in workspace {name}. Attach with: teryon connect {host} --workspace {name}",
                ids.len()
            );
        }
        Commands::Wake { host } => {
            let cfg = config::load()?;
            let h = cfg
                .hosts
                .get(&host)
                .ok_or_else(|| anyhow::anyhow!("no host `{host}` in the config"))?;
            let mac = h.mac.as_deref().ok_or_else(|| {
                anyhow::anyhow!("host `{host}` has no `mac = \"aa:bb:cc:dd:ee:ff\"` in the config")
            })?;
            teryon::wake::wake(mac, h.broadcast.as_deref())?;
            println!("Sent a wake-on-LAN packet for {host}; give it a few seconds to boot");
        }
        Commands::ImportSsh { file, dry_run } => {
            let path = match file {
                Some(f) => f,
                None => teryon::util::home()?.join(".ssh/config"),
            };
            let names = config::ssh_hosts(&std::fs::read_to_string(&path)?);
            if dry_run {
                let known = config::load()?.hosts;
                for n in names.iter().filter(|n| !known.contains_key(*n)) {
                    println!("would add {n}");
                }
            } else {
                let added = config::add_hosts(&names)?;
                println!(
                    "Added {} host alias(es) to {}",
                    added.len(),
                    config::path()?.display()
                );
                for n in added {
                    println!("  {n}");
                }
            }
        }
        Commands::Sessions { host } => {
            let response = target(host, bin)?.connect()?.call(Request::List)?;
            println!("{}", serde_json::to_string_pretty(&response)?);
        }
        Commands::Remove {
            host,
            workspace,
            force,
        } => {
            target(host, bin)?
                .connect()?
                .call(Request::Remove { workspace, force })?;
            println!("Workspace removed");
        }
        Commands::Panes { host, workspace } => {
            let mut rpc = attach(&target(host, bin)?, workspace, true, None)?;
            for p in panes(&mut rpc)? {
                println!(
                    "{}\t{}\tpid={:?}\texit={:?}\t{}",
                    p.id, p.title, p.pid, p.exited, p.cwd
                );
            }
        }
        Commands::Capture {
            host,
            workspace,
            pane,
        } => {
            let mut rpc = attach(&target(host, bin)?, workspace, true, None)?;
            println!("{}", capture(&mut rpc, pane)?);
        }
        Commands::Record {
            host,
            workspace,
            pane,
            output,
            seconds,
        } => {
            let mut rpc = attach(&target(host, bin)?, workspace, true, None)?;
            let mut known = BTreeMap::new();
            let mut writer: Option<teryon::record::CastWriter<std::fs::File>> = None;
            let start = Instant::now();
            let unix = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_secs());
            loop {
                let Response::State { panes } = rpc.call(Request::Poll {
                    known: known.clone(),
                    diff: false,
                })?
                else {
                    bail!("unexpected response")
                };
                let Some(view) = panes.iter().find(|p| p.id == pane) else {
                    bail!("pane {pane} not found");
                };
                known.insert(pane.to_string(), view.revision);
                if let Some(screen) = &view.screen {
                    if writer.is_none() {
                        writer = Some(teryon::record::CastWriter::create(
                            &output, view.cols, view.rows, unix,
                        )?);
                        eprintln!(
                            "Recording pane {pane} to {} (Ctrl-C to stop)",
                            output.display()
                        );
                    }
                    if let Some(w) = writer.as_mut() {
                        w.frame(start.elapsed().as_secs_f64(), screen)?;
                    }
                }
                if view.exited.is_some() || seconds.is_some_and(|s| start.elapsed().as_secs() >= s)
                {
                    break;
                }
                thread::sleep(Duration::from_millis(100));
            }
            if writer.is_none() {
                bail!("pane {pane} produced no screen to record");
            }
        }
        Commands::Send {
            host,
            workspace,
            pane,
            text,
            enter,
        } => {
            let mut rpc = attach(&target(host, bin)?, workspace, false, None)?;
            let mut data = text.into_bytes();
            if enter {
                data.push(b'\r');
            }
            rpc.call(Request::Input { pane, seq: 1, data })?;
        }
        Commands::Exec {
            host,
            workspace,
            cwd,
            wait,
            timeout,
            notify,
            command,
        } => {
            let label = command.join(" ");
            let mut rpc = attach(&target(host, bin)?, workspace, false, cwd.clone())?;
            let Response::Created { pane } = rpc.call(Request::NewPane {
                cwd,
                command,
                rows: 40,
                cols: 120,
            })?
            else {
                bail!("unexpected response");
            };
            eprintln!("Task started in pane {pane}");
            if wait {
                let deadline = Instant::now() + Duration::from_secs(timeout);
                loop {
                    if let Some(p) = panes(&mut rpc)?.into_iter().find(|p| p.id == pane) {
                        if let Some(code) = p.exited {
                            thread::sleep(Duration::from_millis(100));
                            println!("{}", capture(&mut rpc, pane)?);
                            if notify {
                                notify_done(&label, code);
                            }
                            return Ok(code.min(255) as i32);
                        }
                    }
                    if Instant::now() >= deadline {
                        bail!("wait timed out; remote task remains running in pane {pane}");
                    }
                    thread::sleep(Duration::from_millis(150));
                }
            }
        }
        Commands::Forward {
            host,
            ports,
            reconnect,
        } => {
            let (local, remote) = ports
                .split_once(':')
                .context("use LOCAL_PORT:REMOTE_PORT")?;
            let local = local.parse()?;
            let remote = remote.parse()?;
            let t = target(host, bin)?;
            if local == 0 || remote == 0 {
                bail!("port 0 is not supported");
            }
            loop {
                let result = t.forward(local, remote);
                if !reconnect {
                    result?;
                    break;
                }
                if let Err(e) = result {
                    eprintln!("{e}; retrying in 2s");
                }
                thread::sleep(Duration::from_secs(2));
            }
        }
        Commands::Push {
            host,
            source,
            destination,
            limit_kbps,
        } => target(host, bin)?.transfer(true, &source, &destination, limit_kbps)?,
        Commands::Pull {
            host,
            source,
            destination,
            limit_kbps,
        } => target(host, bin)?.transfer(false, &source, &destination, limit_kbps)?,
        Commands::Deploy { host, binary } => {
            target(host, bin)?.deploy(&binary)?;
            println!("Deployment verified");
        }
        Commands::Doctor {
            host,
            fix,
            binary,
            yes,
        } => {
            let t = target(host, bin)?;
            let mut rpc = match t.connect() {
                Ok(rpc) => rpc,
                Err(err) => {
                    eprintln!("Cannot reach a compatible daemon on {}: {err:#}", t.host);
                    let (true, Some(binary)) = (fix, binary) else {
                        bail!("re-run with --fix --binary PATH (a build for the remote OS/architecture) to install one");
                    };
                    if !yes
                        && !confirm(&format!(
                            "Upload {} to {} and retry?",
                            binary.display(),
                            t.host
                        ))?
                    {
                        bail!("cancelled; nothing was changed");
                    }
                    t.deploy(&binary)?;
                    t.connect()
                        .context("daemon still unreachable after deployment")?
                }
            };
            let response = rpc.call(Request::Info)?;
            println!("{}", serde_json::to_string_pretty(&response)?);
            println!(
                "Protocol {VERSION}; authenticated transport OK; persistent daemon reachable."
            );
            println!("Reboot/crash recovery does not preserve live processes. rsync 3+ is required for transfers.");
        }
        Commands::Stop { host, force } => {
            target(host, bin)?
                .connect()?
                .call(Request::Shutdown { force })?;
            println!("Daemon stopped");
        }
        Commands::ShellInit { shell } => {
            let script = teryon::shellinit::script(&shell).context("unsupported shell")?;
            if shell == "zsh" {
                // A broken config must not break shell start-up: fall back to the defaults.
                let cfg = config::load().map(|c| c.shell).unwrap_or_default();
                print!("{}", teryon::shellinit::preamble(&cfg));
            }
            print!("{script}")
        }
        Commands::Hist(HistCommand::Add { exit, ms, cwd, cmd }) => {
            let at = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_secs());
            teryon::history::add(&teryon::history::Entry {
                cmd: cmd.join(" "),
                cwd,
                exit,
                ms,
                at,
            })?;
        }
        Commands::Hist(HistCommand::Suggest { cwd, prefix }) => {
            let entries = teryon::history::load_tail();
            if let Some(s) = teryon::history::suggest(&entries, &prefix.join(" "), &cwd) {
                println!("{s}");
            }
        }
        Commands::Pick {
            history,
            files,
            dirs,
            cwd,
            query,
        } => {
            use std::io::Read;
            let cwd = cwd.canonicalize().unwrap_or(cwd);
            let (items, scoped, label) = if history {
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0, |d| d.as_secs());
                let home = std::env::var("HOME").unwrap_or_default();
                let items = teryon::picker::history_items(
                    &teryon::history::load_all(),
                    &cwd.to_string_lossy(),
                    &home,
                    now,
                );
                (items, true, "history")
            } else if dirs {
                (teryon::picker::walk(&cwd, true), false, "directory")
            } else if files {
                let items = teryon::picker::git_files(&cwd)
                    .unwrap_or_else(|| teryon::picker::walk(&cwd, false));
                (items, false, "file")
            } else {
                let mut text = String::new();
                std::io::stdin().read_to_string(&mut text)?;
                let items = text.lines().map(teryon::picker::Item::plain).collect();
                (items, false, "pick")
            };
            match teryon::picker::run(items, &query, scoped, label)? {
                Some((text, run)) => {
                    println!("{text}");
                    if run {
                        std::process::exit(10);
                    }
                }
                None => std::process::exit(1),
            }
        }
        Commands::Prompt { status, ms, cwd } => {
            let home = std::env::var("HOME").unwrap_or_default();
            print!("{}", teryon::prompt::prompt(status, ms, &cwd, &home));
        }
        Commands::Jump(cmd) => {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_secs());
            match cmd {
                JumpCommand::Add { path } => teryon::frecent::add(&path, now)?,
                JumpCommand::Query { cwd, keywords } => {
                    teryon::frecent::seed_if_missing();
                    let hits =
                        teryon::frecent::rank(&teryon::frecent::load(), &keywords, &cwd, now);
                    match hits.first() {
                        Some(best) => println!("{best}"),
                        None => std::process::exit(1),
                    }
                }
                JumpCommand::List { cwd, keywords } => {
                    teryon::frecent::seed_if_missing();
                    for hit in teryon::frecent::rank(&teryon::frecent::load(), &keywords, &cwd, now)
                    {
                        println!("{hit}");
                    }
                }
            }
        }
        Commands::Highlight { line } => {
            let line = line.join(" ");
            for span in teryon::highlight::spans(
                &line,
                &teryon::highlight::is_command,
                &teryon::highlight::path_exists,
            ) {
                println!("{} {} {}", span.start, span.end, span.kind.style());
            }
        }
        Commands::Setup { yes } => {
            let mut io = teryon::setup::Console { assume_yes: yes };
            teryon::setup::run(&mut io, &mut teryon::setup::Real { bin })?;
        }
        Commands::Ai {
            prompt,
            set_key,
            check,
            command,
        } => {
            return teryon::ai::cli::run(prompt, set_key, check, command);
        }
        Commands::Mcp {
            host,
            workspace,
            allow_write,
            no_redact,
            print_config,
        } => {
            if print_config {
                let exe =
                    std::env::current_exe().map_or("teryon".into(), |p| p.display().to_string());
                print!("{}", teryon::mcp::print_config(&exe, allow_write));
                return Ok(0);
            }
            let allowed = teryon::mcp::allowed_hosts();
            if !allowed.contains(&host) {
                bail!("host {host:?} is not `local`, a configured host or an SSH alias");
            }
            let mut server = teryon::mcp::Server {
                backend: teryon::mcp::Real { remote_bin: bin },
                opts: teryon::mcp::Options {
                    host,
                    workspace,
                    allow_write,
                    redact: !no_redact,
                    allowed_hosts: allowed,
                },
            };
            teryon::mcp::serve(
                &mut server,
                std::io::stdin().lock(),
                std::io::stdout().lock(),
            )?;
        }
        Commands::Config { check, keys } => {
            let path = config::path()?;
            println!(
                "Config file: {} ({})",
                path.display(),
                if path.exists() {
                    "found"
                } else {
                    "not created yet; `teryon init` writes an example"
                }
            );
            match config::load_desktop() {
                Ok(_) if check => println!("OK: the [desktop] section is valid"),
                Ok(desktop) if keys => {
                    let (map, _) = teryon::keymap::Keymap::build(&desktop.keys);
                    for action in teryon::keymap::ACTION_NAMES {
                        let chord = map.hint(action);
                        println!(
                            "{action:<22} {}",
                            if chord.is_empty() { "-" } else { &chord }
                        );
                    }
                }
                Ok(desktop) => {
                    // f32 values print like 1.4500000476837158; round for display.
                    fn tidy(v: &mut toml::Value) {
                        match v {
                            toml::Value::Float(f) => *f = (*f * 1000.).round() / 1000.,
                            toml::Value::Table(t) => t.iter_mut().for_each(|(_, v)| tidy(v)),
                            _ => {}
                        }
                    }
                    let mut value = toml::Value::try_from(&desktop)?;
                    tidy(&mut value);
                    let mut root = toml::map::Map::new();
                    root.insert("desktop".into(), value);
                    print!("{}", toml::to_string_pretty(&toml::Value::Table(root))?)
                }
                Err(e) => {
                    eprintln!("error: {e:#}");
                    return Ok(1);
                }
            }
        }
        Commands::Themes {
            import: Some(file),
            import_as,
            force,
            ..
        } => {
            let text = std::fs::read_to_string(&file)?;
            let stem = file
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("imported")
                .to_lowercase()
                .replace(' ', "-");
            let name = import_as.unwrap_or(stem);
            if !teryon::theme::valid_user_name(&name) {
                bail!("theme names use letters, digits, - and _ (and cannot reuse a built-in name): {name}");
            }
            let file_name = file.file_name().and_then(|n| n.to_str()).unwrap_or("");
            let Some(format) = teryon::themeimport::detect(file_name, &text) else {
                bail!("could not tell which terminal this color scheme is from");
            };
            let (mut spec, notes) = teryon::themeimport::import(&text, format, &name)?;
            spec.name = name.clone();
            let path = teryon::theme::user_theme_path(&name).context("theme path")?;
            if path.exists() && !force {
                bail!(
                    "{} already exists; use --force to replace it",
                    path.display()
                );
            }
            teryon::util::write_private_atomic(&path, toml::to_string(&spec)?.as_bytes())?;
            Theme::load(&name)?;
            for n in notes {
                eprintln!("note: {n}");
            }
            println!(
                "Imported {format:?} scheme as theme \"{name}\" ({})",
                path.display()
            );
            println!("Use it with: teryon connect HOST --theme {name}   or set theme = \"{name}\" in config.toml");
        }
        Commands::Themes {
            export: Some(_),
            name: Some(name),
            ..
        } => print!("{}", Theme::load(&name)?.to_wezterm_lua()),
        Commands::Themes { preview, .. } => {
            for name in teryon::theme::all_names() {
                let name = name.as_str();
                let t = Theme::load(name)?;
                if preview {
                    fn rgb(c: ratatui::style::Color) -> (u8, u8, u8) {
                        match c {
                            ratatui::style::Color::Rgb(r, g, b) => (r, g, b),
                            _ => (255, 255, 255),
                        }
                    }
                    let (r, g, b) = rgb(t.background);
                    let (fr, fg, fb) = rgb(t.foreground);
                    print!(
                        "\x1b[48;2;{r};{g};{b}m\x1b[38;2;{fr};{fg};{fb}m  {:16} ",
                        t.name
                    );
                    for c in [t.accent, t.success, t.warning] {
                        let (r, g, b) = rgb(c);
                        print!("\x1b[38;2;{r};{g};{b}m ● ");
                    }
                    println!("\x1b[0m");
                } else {
                    println!("{name}");
                }
            }
        }
        Commands::Layout { action } => {
            match action {
                LayoutCommand::Save {
                    host,
                    workspace,
                    file,
                } => {
                    let mut rpc = attach(&target(host, bin)?, workspace, true, None)?;
                    let layout = teryon::layout::Layout::from_panes(&panes(&mut rpc)?);
                    teryon::util::private_write(&file, &serde_json::to_vec_pretty(&layout)?)?;
                    println!("Saved {}", file.display());
                }
                LayoutCommand::Open {
                    host,
                    workspace,
                    file,
                } => {
                    let layout = teryon::layout::load(&file)?;
                    let t = target(host, bin)?;
                    let mut rpc = t.connect()?;
                    if let Response::Workspaces { items } = rpc.call(Request::List)? {
                        if items.iter().any(|w| w.name == workspace) {
                            bail!("restore requires a new workspace name");
                        }
                    }
                    rpc.call(Request::Attach {
                        workspace,
                        readonly: false,
                        rows: 24,
                        cols: 100,
                        cwd: Some(layout.panes[0].cwd.clone()),
                        takeover: false,
                    })?;
                    apply_layout(&mut rpc, &layout, 1)?;
                    println!("Restored directories and titles as new shells; no old commands were rerun.");
                }
            }
        }
    }
    Ok(0)
}
fn main() {
    match run() {
        Ok(code) => std::process::exit(code),
        Err(e) => {
            eprintln!("teryon: {e:#}");
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;
    #[test]
    fn command_tree_is_consistent() {
        Cli::command().debug_assert();
    }
    #[test]
    fn argv_task_and_explicit_send_parse() {
        assert!(Cli::try_parse_from([
            "teryon",
            "exec",
            "local",
            "--wait",
            "--",
            "/bin/echo",
            "hello"
        ])
        .is_ok());
        assert!(Cli::try_parse_from(["teryon", "send", "mini", "--enter", "echo hello"]).is_ok());
        assert!(Cli::try_parse_from([
            "teryon",
            "layout",
            "save",
            "mini",
            "-w",
            "dev",
            "layout.json"
        ])
        .is_ok());
    }
}
