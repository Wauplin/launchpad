//! "Open in terminal": hands a session to the terminal chosen in Settings (ARCHITECTURE.md, 6.5).

use anyhow::{Context, Result, bail};
use serde::Serialize;
use ts_rs::TS;

use crate::App;
use crate::model::{HarnessKind, Session};

/// Built-in terminals: argument lists where `{cwd}` is replaced and `{command}` expands to the command.
pub const TERMINALS: &[(&str, &str, &[&str])] = &[
    ("terminator", "Terminator", &["terminator", "--working-directory={cwd}", "-x", "{command}"]),
    ("ghostty", "Ghostty", &["ghostty", "--working-directory={cwd}", "-e", "{command}"]),
    ("gnome-terminal", "GNOME Terminal", &["gnome-terminal", "--working-directory={cwd}", "--", "{command}"]),
];

#[derive(Clone, Debug, Serialize, TS)]
pub struct TerminalOption {
    pub id: String,
    pub name: String,
    pub template: String,
    pub found: bool,
}

pub fn options(app: &App) -> Vec<TerminalOption> {
    TERMINALS
        .iter()
        .map(|(id, name, tpl)| TerminalOption {
            id: id.to_string(),
            name: name.to_string(),
            template: tpl.join(" "),
            found: app.env.which(tpl[0]).is_some(),
        })
        .collect()
}

/// The command that resumes the session in a TUI.
fn tui_command(app: &App, s: &Session, is_temp: bool) -> Result<Vec<String>> {
    let id = s.harness_session_id.clone().context("the session has no conversation yet")?;
    Ok(match s.harness {
        HarnessKind::Claude => {
            let mut c = vec![];
            if is_temp {
                // Skips the trust dialog in folders Launchpad created (docs/spikes/5-claude-trust-prompt.md).
                c.extend(["env".into(), "CLAUDE_CODE_SANDBOXED=1".into()]);
            }
            c.extend(["claude".into(), "--resume".into(), id, "--dangerously-skip-permissions".into()]);
            c
        }
        HarnessKind::Codex => vec![
            "codex".into(),
            "resume".into(),
            id,
            "--dangerously-bypass-approvals-and-sandbox".into(),
            "-c".into(),
            "check_for_update_on_startup=false".into(),
            "-c".into(),
            format!("projects.\"{}\".trust_level=\"trusted\"", s.cwd),
        ],
    })
    .map(|mut c: Vec<String>| {
        // Absolute path, since the terminal may not have the same PATH.
        if let Some(p) = app.env.which(&c[if c[0] == "env" { 2 } else { 0 }]) {
            let i = if c[0] == "env" { 2 } else { 0 };
            c[i] = p.to_string_lossy().to_string();
        }
        c
    })
}

pub async fn open(app: &App, s: &Session) -> Result<()> {
    let is_temp = match &s.project_id {
        Some(id) => crate::projects::get(app, id).await?.is_some_and(|p| p.is_temp),
        None => false,
    };
    let command = tui_command(app, s, is_temp)?;
    let cfg = app.config.get().terminal;
    let template: Vec<String> = if cfg.app == "custom" {
        cfg.custom.clone()
    } else {
        TERMINALS.iter().find(|(id, _, _)| *id == cfg.app).map(|(_, _, t)| t.iter().map(|s| s.to_string()).collect()).unwrap_or_default()
    };
    if template.is_empty() {
        bail!("no terminal configured (Settings → General)");
    }
    let mut argv: Vec<String> = Vec::new();
    for part in template {
        if part == "{command}" {
            argv.extend(command.iter().cloned());
        } else {
            argv.push(part.replace("{cwd}", &s.cwd).replace("{command}", &shell_join(&command)));
        }
    }
    tracing::info!("open in terminal: {}", argv.join(" "));
    let mut cmd = crate::procs::command(&app.env, &argv[0], Some(std::path::Path::new(&s.cwd)));
    cmd.args(&argv[1..]).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
    // The terminal must outlive the daemon.
    cmd.kill_on_drop(false);
    unsafe {
        cmd.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }
    cmd.spawn().with_context(|| format!("start {}", argv[0]))?;
    Ok(())
}

fn shell_join(argv: &[String]) -> String {
    argv.iter()
        .map(|a| if a.chars().all(|c| c.is_ascii_alphanumeric() || "-_./=:".contains(c)) { a.clone() } else { format!("'{}'", a.replace('\'', "'\\''")) })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Whether a TUI process still runs this session (checked before re-attaching).
pub fn tui_running(_app: &App, s: &Session) -> bool {
    let Some(id) = &s.harness_session_id else { return false };
    let Ok(dir) = std::fs::read_dir("/proc") else { return false };
    for entry in dir.flatten() {
        let Ok(cmdline) = std::fs::read(entry.path().join("cmdline")) else { continue };
        let args: Vec<&[u8]> = cmdline.split(|b| *b == 0).collect();
        let has_id = args.iter().any(|a| *a == id.as_bytes());
        let is_resume = args.iter().any(|a| *a == b"--resume" || *a == b"resume");
        if has_id && is_resume && !args.iter().any(|a| *a == b"-p") {
            return true;
        }
    }
    false
}
