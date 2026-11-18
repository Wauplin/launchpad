//! Record access and title updates.
use super::*;

impl Sessions {
    pub fn list(&self) -> Vec<Session> {
        let mut v: Vec<Session> = self.inner.records.lock().unwrap().values().cloned().collect();
        v.sort_by_key(|a| std::cmp::Reverse(a.started_at));
        v
    }

    /// Profiles whose ledger cannot represent all of their usage reliably.
    pub(crate) fn profiles_without_cost(&self) -> std::collections::HashSet<String> {
        self.inner.records.lock().unwrap().values().filter(|s| s.usage.cost_unavailable)
            .filter_map(|s| s.launch_spec.profile.clone()).collect()
    }

    pub fn get(&self, id: &str) -> Option<Session> {
        self.inner.records.lock().unwrap().get(id).cloned()
    }

    pub(super) fn live(&self, id: &str) -> Option<Arc<dyn LiveSession>> {
        self.inner.live.lock().unwrap().get(id).map(|(_, l)| l.clone())
    }

    pub fn find_by_harness_id(&self, harness_id: &str) -> Option<Session> {
        self.inner.records.lock().unwrap().values().find(|s| s.harness_session_id.as_deref() == Some(harness_id)).cloned()
    }

    pub async fn rename(&self, app: &App, id: &str, title: &str) -> Result<Session> {
        self.ensure_local(id)?;
        let title = title.trim();
        anyhow::ensure!(!title.is_empty() && title.chars().count() <= 100 && !title.contains(['\n', '\r']), "Use a title of 1 to 100 characters on one line.");
        self.update(app, id, |s| { s.title = title.to_string(); s.title_pending = false; }).await
    }

    pub(crate) fn title_request_lock(&self, id: &str) -> Arc<tokio::sync::Mutex<()>> {
        self.inner.title_requests.get(id)
    }
}

impl Sessions {
    pub(super) fn persist(db: &crate::db::Db, s: &Session) -> impl std::future::Future<Output = Result<()>> + use<> {
        let s = s.clone();
        db.enqueue(move |c| Self::store(c, &s))
    }

    pub(crate) fn store(c: &rusqlite::Connection, s: &Session) -> Result<()> {
        c.execute(
            "INSERT INTO session (id, harness, harness_session_id, project_id, cwd, title, status, status_since,
                open_question, model, effort, launch_spec_json, prompt, cost_usd, usage_json, started_at, ended_at, last_seq, quota_pause_json, title_pending, seen_at, last_message, archived_done, capacity_retry_count, capacity_retry_at, runner_id, generation, unsaved_json, title_refreshed, sleep_json)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26, ?27, ?28, ?29, ?30)
             ON CONFLICT (id) DO UPDATE SET harness_session_id = ?3, project_id = ?4, cwd = ?5, title = ?6, status = ?7,
                status_since = ?8, open_question = ?9, model = ?10, effort = ?11, cost_usd = ?14, usage_json = ?15,
                ended_at = ?17, launch_spec_json = ?12, quota_pause_json = ?19, title_pending = ?20,
                seen_at = ?21, archived_done = ?23, capacity_retry_count = ?24, capacity_retry_at = ?25,
                runner_id = ?26, generation = ?27, unsaved_json = ?28, title_refreshed = ?29, sleep_json = ?30",
            params![
                s.id,
                s.harness.as_str(),
                s.harness_session_id,
                s.project_id,
                s.cwd,
                s.title,
                s.status.as_str(),
                s.status_since,
                s.open_question as i64,
                s.model,
                s.effort,
                serde_json::to_string(&s.launch_spec)?,
                s.prompt,
                s.usage.cost_usd,
                serde_json::to_string(&s.usage)?,
                s.started_at,
                s.ended_at,
                s.last_seq,
                s.quota_pause.as_ref().map(serde_json::to_string).transpose()?,
                s.title_pending,
                s.seen_at,
                s.last_message,
                s.archived_done,
                s.capacity_retry_count,
                s.capacity_retry_at,
                s.runner_id,
                s.generation,
                s.unsaved.as_ref().map(serde_json::to_string).transpose()?,
                s.title_refreshed,
                s.sleep.as_ref().map(serde_json::to_string).transpose()?,
            ],
        )?;
        crate::replica::queue(c, &crate::replica::Item::Session { session: Box::new(s.without_bytes()) })?;
        Ok(())
    }
}
