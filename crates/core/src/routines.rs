//! The parts of routines that sessions and bots use: a run's instructions, the `no_update` reply and the
//! trigger forms. Routines themselves (`crons`) are in `launchpad-automation`.
use crate::model::RoutineLaunch;

/// The JSON forms of a trigger, for agents that propose routines.
pub const TRIGGER_FORMS: &str = r#"{"kind": "schedule"} | {"kind": "github", "repo": "owner/name" or null for the workspace's repositories, "events": ["review_requested" | "mentioned" | "assigned" | "ci_activity" | "release" | "any"]} | {"kind": "session_done", "folder": "<folder id>" or null for any folder of the workspace, "include_bots": false (true: bot answers count too; by default only other agents' turns do)} | {"kind": "failure", "events": ["session" | "burst" | "routine" | "schedule" | "runner" | "home_restart" | "quota" | "any"]} (Launchpad's own failures in the workspace, plus runners, Home restarts, missing quota readings and Burst jobs without a session: one run lists every new failure with its ids and reason)"#;
/// The final message of a run with nothing to report.
pub const NO_UPDATE: &str = "no_update";

/// Whether a final message means "nothing to report" (`no_update`, alone, maybe quoted or with a period).
pub fn is_no_update(message: Option<&str>) -> bool {
    message.is_some_and(|m| m.trim().trim_matches(|c: char| c == '`' || c == '*' || c == '"' || c == '.' || c.is_whitespace()).eq_ignore_ascii_case(NO_UPDATE))
}

/// The instructions a run adds to the routine's prompt.
pub fn instruction(r: &RoutineLaunch) -> String {
    let mut out = format!("This is a background run of the routine “{}” ({}). Nobody is watching it.", r.title, r.cause);
    if !r.context.trim().is_empty() {
        out.push_str(&format!(" What started it:\n\n{}\n", r.context.trim_end()));
    }
    out.push_str("\nPrepare work for the owner's review (branches, Local PRs, suggestions, drafts in the Outbox) unless these instructions explicitly ask you to publish. When you finish, write a short report for the owner. If there is nothing worth reporting, reply with exactly `no_update` and nothing else: Launchpad then stays silent.");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_update_is_the_whole_final_message() {
        for m in ["no_update", " `no_update` ", "**no_update**", "NO_UPDATE.", "\"no_update\""] {
            assert!(is_no_update(Some(m)), "{m}");
        }
        for m in ["", "no_update: the build is red", "Nothing new, no_update", "noupdate"] {
            assert!(!is_no_update(Some(m)), "{m}");
        }
        assert!(!is_no_update(None));
    }
}
