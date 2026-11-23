//! The spotlight's writes to Home (decision D-WRITE-ACTIONS: bot messages and launches only) and its hand-offs to
//! the browser. Each runs in its own task; the engine registers what to watch before it sends.

use serde_json::{Value, json};

use crate::home::{Home, HomeError};
use crate::links::Links;

/// `POST /api/bots/{id}/message`: a new thread (`thread` None) or a reply in one, accepted at once (`accept`);
/// `key` (the Idempotency-Key) becomes the new session's ID, or the message's `client_id`. Answers the receipt's
/// thread.
pub async fn ask(home: &Home, bot: &str, text: &str, thread: Option<&str>, key: &str) -> Result<String, HomeError> {
    let body = match thread {
        None => json!({"text": text, "new_thread": true, "accept": true}),
        Some(t) => json!({"text": text, "thread": t, "accept": true}),
    };
    let receipt = home.write(reqwest::Method::POST, &format!("/api/bots/{bot}/message"), &body, Some(key)).await?;
    Ok(receipt.get("thread").and_then(Value::as_str).unwrap_or_default().to_string())
}

/// What the quick launch sends (`POST /api/sessions`): the prompt, a profile, a folder (a temporary one when None)
/// with a fresh worktree that keeps its changes (`when_done`), Home's defaults for the rest. Never publishes on
/// GitHub (`draft_pr`/`open_pr` are not offered) and never enables paid usage.
pub fn launch_spec(text: &str, profile: &crate::model::Profile, folder: Option<(&str, bool)>, when_done: &str, workspace: &str) -> Value {
    let (target, worktree) = match folder {
        Some((id, true)) => (json!({"kind": "project", "project_id": id}), json!({"base": null, "when_done": when_done})),
        Some((id, false)) => (json!({"kind": "project", "project_id": id}), Value::Null),
        None => (json!({"kind": "temp"}), Value::Null),
    };
    json!({
        "text": text, "harness": profile.harness, "profile": profile.id, "target": target, "worktree": worktree,
        "workspace_id": workspace, "allow_paid_usage": false, "github_refs": [], "presets": [],
    })
}

/// The quick launch's choices of When done for a Git folder (the publishing ones stay in the browser).
pub const WHEN_DONE: &[(&str, &str)] = &[("keep", "Keep the branch"), ("local_pr", "Open a Local PR"), ("merge", "Merge when done"), ("read_only", "Read only")];

pub async fn launch(home: &Home, spec: &Value, key: &str) -> Result<crate::model::Session, HomeError> {
    let session = home.write(reqwest::Method::POST, "/api/sessions", spec, Some(key)).await?;
    serde_json::from_value(session).map_err(|e| HomeError { failure: crate::conn::Failure::Other, message: format!("Unexpected answer from Home: {e}"), status: 0 })
}

/// The composer draft with the text added (Continue in the browser): appended after the draft's own text, so nothing
/// the browser had is lost, and opened (`open`). Answers the body to save and the revision it starts from.
pub fn draft_body(saved: &Value, text: &str) -> (Value, u64) {
    let revision = saved.get("revision").and_then(Value::as_u64).unwrap_or(0);
    let mut draft = if saved.is_object() { saved.clone() } else { json!({}) };
    let o = draft.as_object_mut().expect("object");
    for k in ["revision", "base_revision", "force", "client"] {
        o.remove(k);
    }
    let spec = o.entry("spec").or_insert_with(|| json!({}));
    if !spec.is_object() {
        *spec = json!({});
    }
    let old = spec.get("text").and_then(Value::as_str).unwrap_or("").trim_end().to_string();
    spec["text"] = Value::String(if old.is_empty() { text.to_string() } else { format!("{old}\n\n{text}") });
    o.insert("open".into(), Value::Bool(true));
    o.insert("minimized".into(), Value::Bool(false));
    o.insert("base_revision".into(), json!(revision));
    o.insert("force".into(), Value::Bool(false));
    o.insert("client".into(), Value::String("launchpad-desktop".into()));
    (draft, revision)
}

/// Continue in the browser for a launch: saves Home's composer draft of the workspace (`PUT /api/launch/draft`,
/// respecting its revision: a newer draft is read again once) and answers the composer's address.
pub async fn handoff(home: &Home, links: &Links, workspace: &str, text: &str) -> Result<String, HomeError> {
    let path = format!("/api/launch/draft?workspace={}", url::form_urlencoded::byte_serialize(workspace.as_bytes()).collect::<String>());
    for attempt in 0..2 {
        let saved: Value = home.get(&path).await?;
        let (body, _) = draft_body(&saved, text);
        match home.write(reqwest::Method::PUT, &path, &body, None).await {
            Ok(_) => return Ok(links.page("/", &[("open", "launch")])),
            Err(e) if e.status == 409 && attempt == 0 => continue,
            Err(e) => return Err(e),
        }
    }
    unreachable!("two attempts")
}

/// The fallback when the draft cannot be saved: the composer with the text in the address (short texts only).
pub fn share_url(links: &Links, text: &str) -> Option<String> {
    (text.chars().count() <= 2000).then(|| links.page("/", &[("share_text", text)]))
}

/// A pasted link or short ID resolved by Home (`GET /api/refs/resolve`): its page in its own workspace.
pub async fn resolve(home: &Home, text: &str) -> Result<String, HomeError> {
    let q: String = url::form_urlencoded::Serializer::new(String::new()).append_pair("ref", text).finish();
    let target: Value = home.get(&format!("/api/refs/resolve?{q}")).await?;
    let path = target.get("path").and_then(Value::as_str).unwrap_or("/");
    let ws = target.get("workspace_id").and_then(Value::as_str).unwrap_or("");
    Ok(Links::new(&home.link.origin, ws).path(path))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn draft_keeps_the_browsers_text_and_revision() {
        let (body, rev) = draft_body(&Value::Null, "fix the tests");
        assert_eq!((rev, body["spec"]["text"].as_str(), body["open"].as_bool(), body["base_revision"].as_u64()), (0, Some("fix the tests"), Some(true), Some(0)));
        let saved = json!({"spec": {"text": "earlier\n", "profile": "p1"}, "open": false, "revision": 7, "conditions": []});
        let (body, rev) = draft_body(&saved, "more");
        assert_eq!(rev, 7);
        assert_eq!(body["spec"]["text"], "earlier\n\nmore");
        assert_eq!(body["spec"]["profile"], "p1");
        assert_eq!(body["base_revision"], 7);
        assert!(body.get("revision").is_none());
    }

    #[test]
    fn launch_spec_shapes() {
        let p = crate::model::Profile { id: "p1".into(), name: "Sub".into(), harness: "claude".into() };
        let git = launch_spec("go", &p, Some(("f1", true)), "local_pr", "w1");
        assert_eq!(git["target"], json!({"kind": "project", "project_id": "f1"}));
        assert_eq!(git["worktree"]["when_done"], "local_pr");
        assert_eq!(git["allow_paid_usage"], false);
        assert_eq!(launch_spec("go", &p, Some(("f2", false)), "keep", "w1")["worktree"], Value::Null);
        assert_eq!(launch_spec("go", &p, None, "keep", "w1")["target"], json!({"kind": "temp"}));
        assert!(share_url(&Links::new("http://h", "w"), &"x".repeat(2001)).is_none());
        assert_eq!(share_url(&Links::new("http://h", "w"), "a b").unwrap(), "http://h/?share_text=a+b&ws=w");
    }
}
