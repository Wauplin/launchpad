//! What a session sees at start: its instructions (the harness's user-level instructions file, written by
//! `Harness::contained_memory`) and its first prompt, in labelled parts. The real start (`contain::apply`,
//! `Sessions::resolve`) and "What it sees at start" (Home's `launchpad_api::bots::start_preview`) build them with
//! the same functions; only the paths differ (the preview names `$LAUNCHPAD_KB` and Home's CLI command).

use serde::Serialize;
use ts_rs::TS;

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

pub fn part(key: &str, label: &str, text: String) -> StartPart {
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
