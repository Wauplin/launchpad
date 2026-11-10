//! What a session sees at start: its instructions (the harness's user-level instructions file, written by
//! `Harness::contained_memory`) and its first prompt, in labelled parts. The real start (`contain::apply`,
//! `Sessions::resolve`) and "What it sees at start" (`preview`, on Home) build them with the same functions; only
//! the paths differ (the preview names `$LAUNCHPAD_KB` and Home's CLI command).

use anyhow::{Context, Result};
use serde::Serialize;
use ts_rs::TS;

use crate::App;
use crate::model::{LaunchSpec, Target};

/// One labelled part of what a session sees at start.
#[derive(Clone, Debug, Serialize, TS, PartialEq)]
pub struct StartPart {
    pub key: String,
    pub label: String,
    pub text: String,
}

/// "What it sees at start": the instructions and the first prompt of a launch, without starting anything.
#[derive(Clone, Debug, Serialize, TS)]
pub struct StartPreview {
    /// The harness's instructions file (CLAUDE.md or AGENTS.md in the session's harness home).
    pub instructions: Vec<StartPart>,
    /// The first prompt.
    pub prompt: Vec<StartPart>,
}

/// The instructions of a session without a workspace.
pub const NO_KB: &str = "Your harness's own memory is off in Launchpad, and this session has no workspace, so no knowledge base: do not write memory files.\n";

/// What the instructions are made of.
pub struct Inputs {
    /// The knowledge base and its INDEX.md (`kb::instructions`), or `NO_KB`.
    pub kb: String,
    /// A bot's memory (`bot_memory::compose`); empty for other sessions.
    pub memory: crate::bot_memory::Loaded,
    /// What the session may do with the Launchpad CLI (`launchpad_cli::cli_reference`), when it has the tool.
    pub cli_reference: Option<String>,
    /// The attribution sentence (`attribution::instruction`).
    pub attribution: String,
}

fn part(key: &str, label: &str, text: String) -> StartPart {
    StartPart { key: key.into(), label: label.into(), text }
}

/// The parts of a session's instructions, in order. A cut memory ends them with the line that says so.
pub fn instruction_parts(i: Inputs) -> Vec<StartPart> {
    let mut out = vec![part("kb", "Knowledge base and its INDEX.md", i.kb)];
    if !i.memory.text.is_empty() {
        out.push(part("memory", "Bot memory", i.memory.text));
    }
    if let Some(r) = i.cli_reference {
        out.push(part("cli", "Launchpad CLI reference", r));
    }
    out.push(part("attribution", "Attribution", i.attribution));
    if let Some(cut) = i.memory.cut {
        out.push(part("memory-cut", "Memory cut", cut));
    }
    out
}

/// The instructions file: the parts, one line apart.
pub fn join(parts: &[StartPart]) -> String {
    parts.iter().map(|p| p.text.as_str()).collect::<Vec<_>>().join("\n")
}

/// The labelled parts of a prompt from where each one starts (`marks`: key, label, byte offset, in order); empty
/// parts are left out.
pub fn slices(prompt: &str, marks: &[(&str, &str, usize)]) -> Vec<StartPart> {
    let mut out = vec![];
    for (i, (key, label, start)) in marks.iter().enumerate() {
        let end = marks.get(i + 1).map_or(prompt.len(), |m| m.2);
        let text = prompt.get(*start..end).unwrap_or_default().trim_start_matches('\n');
        if !text.trim().is_empty() {
            out.push(part(key, label, text.to_string()));
        }
    }
    out
}

/// Home: what a session of this launch would see at start, built like a real start (`contain::apply` for the
/// instructions, `Sessions::preview` for the prompt) from Home's records and stores.
pub async fn preview(app: &App, spec: &LaunchSpec) -> Result<StartPreview> {
    let harness = spec.harness.context("pick a profile")?;
    let prompt = app.sessions.preview(app, spec).await?.parts;
    let project_id = match &spec.target {
        Target::Project { project_id } | Target::Predraft { project_id } => Some(project_id.clone()),
        _ => None,
    };
    let folder_ws = match &project_id {
        Some(p) => crate::projects::get(app, p).await?.and_then(|p| p.workspace_id),
        None => None,
    };
    let workspace_id = folder_ws.or_else(|| spec.workspace_id.clone());
    let cli = app.paths.cli_command();
    let kb = match &workspace_id {
        Some(ws) => crate::kb::instructions_on_home(app, ws).await,
        None => NO_KB.into(),
    };
    let memory = match &spec.bot {
        Some(b) => {
            let tree = crate::plain::tree(app, &crate::bots::store(&b.id)).await?;
            crate::bot_memory::compose(&crate::bot_memory::store_notes(app, &tree.files).await, &cli)
        }
        None => Default::default(),
    };
    let req = crate::vault::LeaseRequest {
        session_id: String::new(),
        generation: 0,
        harness,
        login: None,
        workspace_id: workspace_id.clone(),
        tools_off: spec.tools_off.clone(),
        tools_on: spec.tools_on.clone(),
        project_id,
        bot_id: spec.bot.as_ref().map(|b| b.id.clone()),
    };
    let cli_reference = crate::vault::launchpad_access(app, &req).await?.map(|a| crate::launchpad_cli::cli_reference(&cli, workspace_id.as_deref(), a));
    let attribution = crate::attribution::instruction(&crate::attribution::text(spec.bot.as_ref().map(|b| b.name.as_str()), harness.display()));
    Ok(StartPreview { instructions: instruction_parts(Inputs { kb, memory, cli_reference, attribution }), prompt })
}

/// Home: what the bot's next new session would see at start (its next message stands for the typed text).
pub async fn bot_preview(app: &App, bot_id: &str) -> Result<StartPreview> {
    let bot = crate::bots::get(app, bot_id).await?;
    let thread = crate::bots::thread(app, bot_id);
    let spec = crate::bots::next_spec(app, &bot, &thread, "(your next message)".into(), vec![], Default::default()).await?;
    preview(app, &spec).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parts_join_like_the_instructions_file_and_a_cut_memory_ends_them() {
        let memory = crate::bot_memory::Loaded { text: "# Your memory\n".into(), cut: Some("cut line\n".into()), loaded: 100, total: 240 };
        let parts = instruction_parts(Inputs { kb: "kb\n".into(), memory, cli_reference: Some("cli\n".into()), attribution: "attr\n".into() });
        assert_eq!(parts.iter().map(|p| p.key.as_str()).collect::<Vec<_>>(), ["kb", "memory", "cli", "attribution", "memory-cut"]);
        assert_eq!(join(&parts), "kb\n\n# Your memory\n\ncli\n\nattr\n\ncut line\n");
        assert!(join(&parts).trim_end().ends_with("cut line"));
        let plain = instruction_parts(Inputs { kb: "kb\n".into(), memory: Default::default(), cli_reference: None, attribution: "attr\n".into() });
        assert_eq!(join(&plain), "kb\n\nattr\n");
    }

    #[test]
    fn prompt_slices_follow_their_marks() {
        let prompt = "Hello\n\nSelected skills:\n- x\n\nFinal.";
        let (skills, last) = (prompt.find("\n\nSelected").unwrap(), prompt.find("\n\nFinal").unwrap());
        let marks = [("message", "M", 0), ("skills", "S", skills), ("traces", "T", last), ("final", "F", last)];
        let parts = slices(prompt, &marks);
        assert_eq!(parts.iter().map(|p| (p.key.as_str(), p.text.as_str())).collect::<Vec<_>>(), [("message", "Hello"), ("skills", "Selected skills:\n- x"), ("final", "Final.")]);
        assert_eq!(parts.iter().map(|p| p.text.as_str()).collect::<Vec<_>>().join("\n\n"), prompt);
    }
}
