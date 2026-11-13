//! The feedback ledger (K1): what happened to each part of an agent's suggestion and how the owner changed it before
//! it reached GitHub. Capture only: a later Feedback view and a gardener bot read it; nothing here decides anything.
//!
//! One record per part (`body`, or `line:<path>:<line>:<side>` for one line comment of a review) in `suggestion_ledger`
//! of state.db on Home, created when the suggestion is created (with the agent's original text), then events:
//! `shown` (the extension showed it 1.5 s), `inserted` (Insert into GitHub's box), `posted_by_hand` (the extension saw
//! GitHub's own form submitted, with the final text: a claim), `confirmed` (Home found the owner's comment with that
//! text on GitHub: the proof), `unconfirmed` (no match within an hour; never counts as feedback),
//! `posted_by_launchpad` (Launchpad's Post), `dismissed` (one line comment), `discarded`, `outdated` (the pull
//! request's head moved past a review's commit).
use std::time::Duration;

use anyhow::{Result, ensure};
use launchpad_core::{App, attribution, util::now_ms};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use ts_rs::TS;

use crate::suggestions::{self, ReplyTo, Suggestion, SuggestionKind, SuggestionTarget};

/// Without a matching comment on GitHub this long after the claim, a posted-by-hand record stays unconfirmed.
const CONFIRM_WITHIN_MS: i64 = 60 * 60 * 1000;
/// A comment counts as the claimed text when it keeps at least this share of it.
const MATCH_RATIO: f64 = 0.85;
const MAX_EVENTS: usize = 50;

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct LedgerEvent {
    /// shown, inserted, posted_by_hand, confirmed, posted_by_launchpad, dismissed, discarded, outdated, unconfirmed.
    pub event: String,
    #[ts(type = "number")]
    pub at: i64,
}

/// How much the owner changed the agent's text (the attribution line left out on both sides).
#[derive(Clone, Debug, Serialize, Deserialize, TS, PartialEq)]
pub struct EditSize {
    /// Characters inserted, deleted or replaced (Levenshtein distance).
    pub chars_changed: u32,
    /// Words inserted, deleted or replaced.
    pub words_changed: u32,
    /// 1 − chars_changed / the longer text's length: 1 when kept as written.
    pub ratio_kept: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct LedgerRecord {
    pub suggestion_id: String,
    /// `body`, or `line:<path>:<line>:<side>` for one line comment of a review.
    pub part: String,
    pub kind: SuggestionKind,
    /// The bot whose session wrote it.
    pub bot: Option<String>,
    pub bot_id: Option<String>,
    pub session_id: Option<String>,
    #[ts(optional)]
    pub repo: Option<String>,
    #[ts(type = "number | null")]
    pub number: Option<u64>,
    pub reply_to: Option<ReplyTo>,
    /// The agent's text when it suggested it.
    pub original: String,
    /// What reached GitHub (posted by hand or by Launchpad), with its attribution line when it was kept.
    #[serde(rename = "final")]
    #[ts(rename = "final")]
    pub final_text: Option<String>,
    pub edit: Option<EditSize>,
    /// The attribution line Insert added was still there when it was posted (null: not posted by hand).
    pub attribution_kept: Option<bool>,
    pub posted_url: Option<String>,
    pub events: Vec<LedgerEvent>,
    #[ts(type = "number")]
    pub created_at: i64,
    #[ts(type = "number")]
    pub updated_at: i64,
}

impl LedgerRecord {
    fn has(&self, event: &str) -> bool {
        self.events.iter().any(|e| e.event == event)
    }
    fn last(&self, event: &str) -> Option<i64> {
        self.events.iter().rev().find(|e| e.event == event).map(|e| e.at)
    }
    fn push(&mut self, event: &str) {
        let at = now_ms();
        if self.events.len() >= MAX_EVENTS {
            self.events.remove(1);
        }
        self.events.push(LedgerEvent { event: event.into(), at });
        self.updated_at = at;
    }
    /// A claim waits for Home's confirmation.
    fn pending(&self) -> bool {
        self.last("posted_by_hand").is_some_and(|p| self.events.iter().all(|e| !(e.at >= p && matches!(e.event.as_str(), "confirmed" | "unconfirmed"))))
    }
    fn finish(&mut self, final_text: &str) {
        let kept = attribution::unsigned(final_text);
        self.edit = Some(edit_size(self.original.trim(), kept.trim()));
        self.final_text = Some(final_text.to_string());
    }
}

/// The part name of a line comment.
pub fn line_part(c: &suggestions::LineComment) -> String {
    format!("line:{}:{}:{}", c.path, c.line, c.side)
}

fn new_record(app: &App, s: &Suggestion, part: &str, original: &str) -> LedgerRecord {
    let bot = s.session_id.as_deref().and_then(|id| app.sessions.get(id)).and_then(|x| x.launch_spec.bot);
    let (repo, number, reply_to) = match &s.target {
        SuggestionTarget::GitHub { repo, number, reply_to } => (Some(repo.clone()), Some(*number), reply_to.clone()),
        SuggestionTarget::Predraft { .. } => (None, None, None),
    };
    let at = now_ms();
    LedgerRecord {
        suggestion_id: s.id.clone(),
        part: part.into(),
        kind: s.kind.clone(),
        bot: bot.as_ref().map(|b| b.name.clone()),
        bot_id: bot.map(|b| b.id),
        session_id: s.session_id.clone(),
        repo,
        number,
        reply_to,
        original: original.into(),
        final_text: None,
        edit: None,
        attribution_kept: None,
        posted_url: None,
        events: vec![],
        created_at: at,
        updated_at: at,
    }
}

/// The parts of a suggestion with the agent's text of each.
fn parts(s: &Suggestion) -> Vec<(String, String)> {
    let mut out = vec![];
    if !s.body.trim().is_empty() {
        out.push(("body".to_string(), s.body.clone()));
    }
    out.extend(s.comments.iter().map(|c| (line_part(c), c.body.clone())));
    out
}

/// One writer at a time: every change reads, changes and writes a record.
static WRITE: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

async fn load(app: &App, suggestion: Option<&str>) -> Result<Vec<LedgerRecord>> {
    let id = suggestion.map(str::to_string);
    app.state_db
        .call(move |c| {
            let rows = match &id {
                Some(id) => c.prepare("SELECT json FROM suggestion_ledger WHERE suggestion_id = ?1 ORDER BY rowid")?.query_map([id], |r| r.get::<_, String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?,
                None => c.prepare("SELECT json FROM suggestion_ledger ORDER BY updated_at DESC LIMIT 2000")?.query_map([], |r| r.get::<_, String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?,
            };
            Ok(rows.into_iter().filter_map(|j| serde_json::from_str(&j).ok()).collect())
        })
        .await
}

async fn store(app: &App, records: Vec<LedgerRecord>) -> Result<()> {
    if records.is_empty() {
        return Ok(());
    }
    type Row = (String, String, Option<String>, Option<i64>, bool, i64, String);
    let rows: Vec<Row> = records
        .iter()
        .map(|r| Ok((r.suggestion_id.clone(), r.part.clone(), r.repo.as_ref().map(|x| x.to_lowercase()), r.number.map(|n| n as i64), r.pending(), r.updated_at, serde_json::to_string(r)?)))
        .collect::<Result<_>>()?;
    app.state_db
        .call(move |c| {
            let tx = c.transaction()?;
            for (id, part, repo, number, pending, at, text) in rows {
                tx.execute(
                    "INSERT INTO suggestion_ledger (suggestion_id, part, repo, number, pending, updated_at, json) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                     ON CONFLICT (suggestion_id, part) DO UPDATE SET repo = ?3, number = ?4, pending = ?5, updated_at = ?6, json = ?7",
                    rusqlite::params![id, part, repo, number, pending, at, text],
                )?;
            }
            tx.commit()?;
            Ok(())
        })
        .await?;
    app.bus.publish("suggestions", "ledger", json!({"ids": records.iter().map(|r| r.suggestion_id.clone()).collect::<Vec<_>>()}));
    Ok(())
}

/// Changes the records of `s` (created from the suggestion when missing) with `change(record)`; true keeps a change.
async fn update(app: &App, s: &Suggestion, only: Option<&str>, mut change: impl FnMut(&mut LedgerRecord) -> bool) -> Result<()> {
    let _w = WRITE.lock().await;
    let mut have = load(app, Some(&s.id)).await?;
    let mut changed = vec![];
    for (part, text) in parts(s) {
        if only.is_some_and(|o| o != part) {
            continue;
        }
        let mut r = match have.iter().position(|r| r.part == part) {
            Some(i) => have.swap_remove(i),
            None => new_record(app, s, &part, &text),
        };
        if change(&mut r) {
            changed.push(r);
        }
    }
    // Parts the suggestion no longer has (line comments the owner removed) keep their records.
    for mut r in have {
        if only.is_none_or(|o| o == r.part) && change(&mut r) {
            changed.push(r);
        }
    }
    store(app, changed).await
}

fn logged(what: &str, r: Result<()>) {
    if let Err(e) = r {
        tracing::warn!("feedback ledger ({what}): {e:#}");
    }
}

/// A new suggestion: one record per part with the agent's text.
pub async fn drafted(app: &App, s: &Suggestion) {
    logged("drafted", update(app, s, None, |_| true).await);
}

pub async fn discarded(app: &App, s: &Suggestion) {
    logged("discarded", update(app, s, None, |r| {
        if r.has("discarded") || r.has("posted_by_launchpad") || r.has("dismissed") {
            return false;
        }
        r.push("discarded");
        true
    }).await);
}

/// The PR head moved past the commit a review was written for: once per line comment.
pub async fn outdated(app: &App, s: &Suggestion) {
    logged("outdated", update(app, s, None, |r| {
        if !r.part.starts_with("line:") || r.has("outdated") {
            return false;
        }
        r.push("outdated");
        true
    }).await);
}

/// Launchpad's Post: each part as posted (the owner may have edited it), or dismissed when the owner removed it.
pub async fn posted_by_launchpad(app: &App, s: &Suggestion) {
    let reply = match &s.target {
        SuggestionTarget::GitHub { reply_to: Some(r), .. } => r.prefix(),
        _ => String::new(),
    };
    let now: Vec<(String, String)> = parts(s);
    let url = s.url.clone();
    logged("posted", update(app, s, None, |r| {
        if r.has("posted_by_launchpad") {
            return false;
        }
        match now.iter().find(|(p, _)| *p == r.part) {
            Some((_, text)) => {
                let text = if r.part == "body" { format!("{reply}{text}") } else { text.clone() };
                r.edit = Some(edit_size(r.original.trim(), text.trim()));
                r.final_text = Some(text);
                r.posted_url = url.clone();
                r.push("posted_by_launchpad");
            }
            None => r.push("dismissed"),
        }
        true
    }).await);
}

fn check_part(s: &Suggestion, part: &str) -> Result<()> {
    ensure!(parts(s).iter().any(|(p, _)| p == part), "this suggestion has no part {part}");
    ensure!(matches!(s.target, SuggestionTarget::GitHub { .. }), "only suggestions for GitHub items go into GitHub's boxes");
    Ok(())
}

/// The extension showed the suggestion for 1.5 s (recorded once per part).
pub async fn shown(app: &App, id: &str, part: &str) -> Result<()> {
    let s = suggestions::get(app, id).await?;
    check_part(&s, part)?;
    update(app, &s, Some(part), |r| {
        if r.has("shown") {
            return false;
        }
        r.push("shown");
        true
    })
    .await
}

/// Insert into GitHub's box: claims the suggestion like Post does (Launchpad's Post then says "Inserted on GitHub"),
/// so it is not posted twice. `text`: what went into the box, without the attribution line (an issue-comment reply
/// starts with its link), compared later with what was posted.
pub async fn inserted(app: &App, id: &str, part: &str, text: Option<String>) -> Result<Suggestion> {
    let lock = app.suggestion_locks.get(id);
    let _guard = lock.lock().await;
    let mut s = suggestions::get(app, id).await?;
    check_part(&s, part)?;
    ensure!(matches!(s.status.as_str(), "draft" | "inserted"), "{}", match s.status.as_str() {
        "posted" | "posted_by_hand" => "already posted",
        _ => "publication was attempted from Launchpad: check GitHub first",
    });
    if part == "body" && s.status != "inserted" {
        s.status = "inserted".into();
        suggestions::save(app, &s).await?;
    }
    let text = text.filter(|t| !t.trim().is_empty()).map(|t| t.chars().take(70_000).collect::<String>());
    update(app, &s, Some(part), |r| {
        if let Some(t) = &text {
            r.original = attribution::unsigned(t).to_string();
        }
        r.push("inserted");
        true
    })
    .await?;
    Ok(s)
}

/// The extension saw GitHub's own form submitted with the inserted text: a claim with the final text, which Home then
/// looks for on GitHub (`confirm`).
pub async fn posted_by_hand(app: &App, id: &str, part: &str, text: String) -> Result<()> {
    ensure!(!text.trim().is_empty() && text.len() <= 262_144, "the posted text must hold 1–262144 bytes");
    let lock = app.suggestion_locks.get(id);
    let _guard = lock.lock().await;
    let mut s = suggestions::get(app, id).await?;
    check_part(&s, part)?;
    ensure!(matches!(s.status.as_str(), "inserted" | "posted_by_hand" | "draft"), "this suggestion was posted from Launchpad");
    if part == "body" && s.status != "posted_by_hand" {
        s.status = "posted_by_hand".into();
        suggestions::save(app, &s).await?;
    }
    let kept = attribution::unsigned(&text).len() != text.trim_end().len();
    update(app, &s, Some(part), |r| {
        r.finish(&text);
        r.attribution_kept = Some(kept);
        r.posted_url = None;
        r.push("posted_by_hand");
        true
    })
    .await?;
    if let SuggestionTarget::GitHub { repo, number, .. } = &s.target {
        spawn_checks(app, repo.clone(), *number);
    }
    Ok(())
}

/// One line comment of a suggested review was dismissed (the extension's ghost comment, D2): recorded, and removed from
/// the draft so Launchpad's Post leaves it out.
pub async fn dismiss_comment(app: &App, id: &str, part: &str) -> Result<()> {
    ensure!(part.starts_with("line:"), "only line comments are dismissed one by one");
    let lock = app.suggestion_locks.get(id);
    let _guard = lock.lock().await;
    let mut s = suggestions::get(app, id).await?;
    check_part(&s, part)?;
    update(app, &s, Some(part), |r| {
        if r.has("dismissed") {
            return false;
        }
        r.push("dismissed");
        true
    })
    .await?;
    if s.status == "draft" {
        s.comments.retain(|c| line_part(c) != part);
        suggestions::save(app, &s).await?;
    }
    Ok(())
}

/// The ledger, newest first, or the records of one suggestion.
pub async fn list(app: &App, suggestion: Option<&str>) -> Result<Vec<LedgerRecord>> {
    load(app, suggestion).await
}

/// After a claim, Home refreshes the item a few times (20 s, 2 min, 10 min, then just after the hour), so its
/// comments are read and the claim confirmed or left unconfirmed. Each refresh runs `confirm`.
fn spawn_checks(app: &App, repo: String, number: u64) {
    let app = app.clone();
    tokio::spawn(async move {
        for wait in [20, 100, 480, 3_060] {
            tokio::time::sleep(Duration::from_secs(wait)).await;
            if !matches!(waiting(&app, &repo, number).await, Ok(true)) {
                return;
            }
            if let Err(e) = crate::items::detail(&app, &repo, number, true).await {
                tracing::warn!("feedback ledger: refreshing {repo}#{number}: {e:#}");
                // GitHub did not answer: the last pass still settles what waited an hour.
                confirm_with(&app, &repo, number, None).await;
            }
        }
    });
}

async fn waiting(app: &App, repo: &str, number: u64) -> Result<bool> {
    let (repo, number) = (repo.to_lowercase(), number as i64);
    app.state_db.call(move |c| Ok(c.query_row("SELECT count(*) FROM suggestion_ledger WHERE pending = 1 AND repo = ?1 AND number = ?2", rusqlite::params![repo, number], |r| r.get::<_, i64>(0))? > 0)).await
}

/// Called after every refresh of an item's comments: claims of that item find their comment (the owner's, created
/// after the insert, close to the claimed text) or, an hour after the claim, become unconfirmed. Also marks the line
/// comments of reviews written for an older head as outdated.
pub async fn confirm(app: &App, repo: &str, number: u64, item: &Value) {
    confirm_with(app, repo, number, Some(item)).await;
    if let Some(head) = item["headRefOid"].as_str()
        && let Ok(list) = suggestions::list(app).await
    {
        for s in list {
            let same = matches!(&s.target, SuggestionTarget::GitHub { repo: r, number: n, .. } if r.eq_ignore_ascii_case(repo) && *n == number);
            if same && s.status == "draft" && !s.comments.is_empty() && s.head.as_deref().is_some_and(|h| h != head) {
                outdated(app, &s).await;
            }
        }
    }
}

async fn confirm_with(app: &App, repo: &str, number: u64, item: Option<&Value>) {
    if !matches!(waiting(app, repo, number).await, Ok(true)) {
        return;
    }
    let login = match crate::items::cached(app, "/user".into()).await.ok().flatten().and_then(|v| v["login"].as_str().map(str::to_string)) {
        Some(l) => Some(l),
        None => app.github.get("/user").await.ok().and_then(|v| v["login"].as_str().map(str::to_string)),
    };
    let comments = item.map(suggestions::item_comments).unwrap_or_default();
    let (repo_l, n) = (repo.to_lowercase(), number as i64);
    let _w = WRITE.lock().await;
    let rows = app
        .state_db
        .call(move |c| Ok(c.prepare("SELECT json FROM suggestion_ledger WHERE pending = 1 AND repo = ?1 AND number = ?2")?.query_map(rusqlite::params![repo_l, n], |r| r.get::<_, String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?))
        .await;
    let Ok(rows) = rows else { return };
    let mut changed = vec![];
    let mut taken: Vec<String> = vec![];
    for row in rows {
        let Ok(mut r) = serde_json::from_str::<LedgerRecord>(&row) else { continue };
        let (Some(claimed), Some(text)) = (r.last("posted_by_hand"), r.final_text.clone()) else { continue };
        let since = r.last("inserted").unwrap_or(claimed).min(claimed) - 120_000;
        let wanted = attribution::unsigned(&text).trim().to_string();
        let found = login.as_deref().and_then(|login| {
            comments
                .iter()
                .filter(|c| c.author.as_deref().is_some_and(|a| a.eq_ignore_ascii_case(login)) && launchpad_core::today::parse_time(&c.created_at) >= since && !taken.contains(&c.url))
                // A length that differs by more than the allowed change cannot match: skip the edit distance.
                .filter(|c| close_lengths(wanted.chars().count(), attribution::unsigned(&c.body).trim().chars().count()))
                .map(|c| (c, edit_size(&wanted, attribution::unsigned(&c.body).trim()).ratio_kept))
                .filter(|(_, ratio)| *ratio >= MATCH_RATIO)
                .max_by(|a, b| a.1.total_cmp(&b.1))
                .map(|(c, _)| c)
        });
        if let Some(c) = found {
            taken.push(c.url.clone());
            r.posted_url = Some(c.url.clone());
            // GitHub holds the text as posted: the record keeps that one.
            r.finish(&c.body);
            r.push("confirmed");
            changed.push(r);
        } else if now_ms() - claimed > CONFIRM_WITHIN_MS {
            r.push("unconfirmed");
            changed.push(r);
        }
    }
    logged("confirm", store(app, changed).await);
}

/// Whether two texts of these lengths (characters) can keep `MATCH_RATIO` of each other.
fn close_lengths(a: usize, b: usize) -> bool {
    (a.abs_diff(b) as f64) <= (1.0 - MATCH_RATIO) * a.max(b) as f64
}

/// Edit size between the agent's text and the posted one: Levenshtein on characters and on words, after the common
/// start and end (long middles are counted as fully changed rather than compared, to bound the work).
pub fn edit_size(original: &str, posted: &str) -> EditSize {
    let a: Vec<char> = original.chars().collect();
    let b: Vec<char> = posted.chars().collect();
    let chars = distance(&a, &b);
    let wa: Vec<&str> = original.split_whitespace().collect();
    let wb: Vec<&str> = posted.split_whitespace().collect();
    let words = distance(&wa, &wb);
    let longest = a.len().max(b.len());
    let ratio = if longest == 0 { 1.0 } else { 1.0 - chars as f64 / longest as f64 };
    EditSize { chars_changed: chars as u32, words_changed: words as u32, ratio_kept: (ratio * 1000.0).round() / 1000.0 }
}

fn distance<T: PartialEq>(a: &[T], b: &[T]) -> usize {
    let start = a.iter().zip(b).take_while(|(x, y)| x == y).count();
    let (a, b) = (&a[start..], &b[start..]);
    let end = a.iter().rev().zip(b.iter().rev()).take_while(|(x, y)| x == y).count();
    let (a, b) = (&a[..a.len() - end], &b[..b.len() - end]);
    if a.is_empty() || b.is_empty() {
        return a.len().max(b.len());
    }
    if a.len().saturating_mul(b.len()) > 25_000_000 {
        return a.len().max(b.len());
    }
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0; b.len() + 1];
    for (i, x) in a.iter().enumerate() {
        cur[0] = i + 1;
        for (j, y) in b.iter().enumerate() {
            cur[j + 1] = if x == y { prev[j] } else { 1 + prev[j].min(prev[j + 1]).min(cur[j]) };
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}
