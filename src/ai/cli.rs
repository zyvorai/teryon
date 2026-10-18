//! `teryon ai`: chat and guarded actions from the terminal.
use super::{api, key, tools, Agent, AgentEvent, Decision};
use crate::config;
use anyhow::{bail, Result};
use std::io::{BufRead, IsTerminal, Write};
use std::sync::Arc;
use std::time::Instant;

pub fn settings(cfg: &config::AiConfig) -> api::Settings {
    api::Settings {
        model: cfg.model.clone(),
        max_tokens: cfg.max_tokens,
        base_url: std::env::var("TERYON_AI_BASE_URL")
            .ok()
            .or_else(|| cfg.base_url.clone())
            .unwrap_or_else(|| api::DEFAULT_BASE.into()),
    }
}

pub const NO_KEY: &str = "No API key found. Export ANTHROPIC_API_KEY=sk-ant-… or store one in the Keychain with `teryon ai --set-key`.";

pub fn run(prompt: Vec<String>, set_key: bool, check: bool, command: bool) -> Result<i32> {
    if set_key {
        key::set_interactively()?;
        println!(
            "Key saved in the macOS Keychain (service {}).",
            key::KEYCHAIN_SERVICE
        );
        return Ok(0);
    }
    let cfg = config::load()?.ai;
    cfg.validate()?;
    if !cfg.enabled {
        bail!("the AI assistant is disabled ([ai] enabled = false)");
    }
    let Some(api_key) = key::find() else {
        eprintln!("{NO_KEY}");
        return Ok(1);
    };
    let settings = settings(&cfg);
    if check {
        use api::Model;
        let curl = api::Curl {
            settings: settings.clone(),
            key: api_key.clone(),
        };
        let started = Instant::now();
        let mut reply = String::new();
        let r = curl.stream(
            "Reply with the single word: ok",
            &serde_json::json!([]),
            &[serde_json::json!({"role": "user", "content": "ping"})],
            &mut |e| match e {
                api::Event::Text(t) => reply.push_str(&t),
                api::Event::Error(m) => eprintln!("error: {m}"),
                api::Event::ToolUse { .. } => {}
            },
        );
        return Ok(match r {
            Ok(_) => {
                println!(
                    "OK: key accepted, model {} answered \"{}\" in {} ms{}",
                    settings.model,
                    reply.trim(),
                    started.elapsed().as_millis(),
                    if key::looks_valid(&api_key) {
                        ""
                    } else {
                        " (the key does not look like sk-ant-…)"
                    }
                );
                0
            }
            Err(e) => {
                if e != "request failed" {
                    eprintln!("error: {e}");
                }
                1
            }
        });
    }
    if command {
        if prompt.is_empty() {
            bail!("say what you want, e.g. teryon ai --command find the 5 biggest files here");
        }
        let curl = api::Curl {
            settings,
            key: api_key,
        };
        let cwd = std::env::current_dir()
            .map(|p| p.display().to_string())
            .unwrap_or_default();
        let context = format!(
            "OS: {}\nShell: {}\nDirectory: {cwd}",
            std::env::consts::OS,
            std::env::var("SHELL").unwrap_or_else(|_| "sh".into())
        );
        return Ok(
            match super::suggest::run(&curl, &context, &prompt.join(" ")) {
                Ok(s) => {
                    println!("{}", s.command);
                    eprintln!(
                        "# {} (risk: {}{})",
                        s.explanation,
                        s.risk,
                        s.blocked
                            .map_or(String::new(), |w| format!(", blocked: {w}"))
                    );
                    0
                }
                Err(e) => {
                    eprintln!("error: {e}");
                    1
                }
            },
        );
    }
    let agent = Agent::spawn(
        Box::new(api::Curl {
            settings,
            key: api_key,
        }),
        Arc::new(tools::System),
        Box::new(tools::NoEnv),
        cfg.auto_allow,
    );
    let one_shot = !prompt.is_empty();
    if one_shot {
        return Ok(turn(&agent, &prompt.join(" ")));
    }
    let tty = std::io::stdin().is_terminal();
    let stdin = std::io::stdin();
    loop {
        if tty {
            print!("\x1b[1mteryon ai ›\x1b[0m ");
            std::io::stdout().flush().ok();
        }
        let mut line = String::new();
        if stdin.lock().read_line(&mut line)? == 0 {
            return Ok(0);
        }
        let line = line.trim();
        if matches!(line, "exit" | "quit" | ":q") {
            return Ok(0);
        }
        if !line.is_empty() {
            turn(&agent, line);
        }
    }
}

/// Send one message and print the reply, asking on the terminal before each changing tool.
fn turn(agent: &Agent, text: &str) -> i32 {
    agent.send(text);
    let color = std::io::stdout().is_terminal();
    let dim = |s: &str| {
        if color {
            format!("\x1b[2m{s}\x1b[0m")
        } else {
            s.to_string()
        }
    };
    let mut failed = false;
    let mut mid_line = false;
    while let Ok(ev) = agent.events.recv() {
        match ev {
            AgentEvent::Text(t) => {
                print!("{t}");
                std::io::stdout().flush().ok();
                mid_line = !t.ends_with('\n');
            }
            AgentEvent::ToolRequest {
                id,
                summary,
                needs_approval,
                ..
            } => {
                if mid_line {
                    println!();
                    mid_line = false;
                }
                if !needs_approval {
                    println!("{}", dim(&format!("• {summary}")));
                    continue;
                }
                print!("→ {summary}\n  Allow? [y]es / [n]o / [a]lways this session: ");
                std::io::stdout().flush().ok();
                let mut answer = String::new();
                let _ = std::io::stdin().lock().read_line(&mut answer);
                let d = match answer.trim().to_lowercase().as_str() {
                    "y" | "yes" => Decision::Allow,
                    "a" | "always" => Decision::AllowSession,
                    _ => Decision::Deny,
                };
                agent.decide(&id, d);
            }
            AgentEvent::ToolDone { ok, output, .. } => {
                let first = output
                    .lines()
                    .next()
                    .unwrap_or("")
                    .chars()
                    .take(100)
                    .collect::<String>();
                println!(
                    "{}",
                    dim(&format!("  {} {first}", if ok { "✓" } else { "✗" }))
                );
            }
            AgentEvent::Error(m) => {
                if mid_line {
                    println!();
                    mid_line = false;
                }
                eprintln!("error: {m}");
                failed = true;
            }
            AgentEvent::Done => break,
        }
    }
    if mid_line {
        println!();
    }
    i32::from(failed)
}
