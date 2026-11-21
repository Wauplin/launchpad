//! Idle process release and confirmed shutdown ownership.
use super::*;

impl Sessions {
    /// Stops an idle process before its folder moves or disappears; sending a message resumes it.
    /// The caller holds the session's action lock.
    pub(crate) async fn release(&self, app: &App, id: &str) -> Result<()> {
        self.ensure_shutdown_finished(id)?;
        if self.live(id).is_none() { return Ok(()); }
        self.detach(app, id).await?;
        self.update(app, id, |s| s.live = false).await?;
        Ok(())
    }

    pub(super) fn ensure_shutdown_finished(&self, id: &str) -> Result<()> {
        anyhow::ensure!(!self.inner.shutdowns.lock().unwrap().contains_key(id), "The previous process is still stopping. Wait for its exit.");
        Ok(())
    }

    /// Keep ownership in a background task when a process cannot acknowledge shutdown promptly.
    pub(crate) async fn wait_for_shutdown<F>(&self, id: &str, wait: F) -> Result<()>
    where F: std::future::Future<Output = Result<()>> + Send + 'static {
        self.wait_for_shutdown_timeout(id, wait, std::time::Duration::from_secs(5)).await
    }

    pub(super) async fn wait_for_shutdown_timeout<F>(&self, id: &str, wait: F, timeout: std::time::Duration) -> Result<()>
    where F: std::future::Future<Output = Result<()>> + Send + 'static {
        *self.inner.shutdowns.lock().unwrap().entry(id.into()).or_default() += 1;
        let (this, id) = (self.clone(), id.to_string());
        let task = tokio::spawn(async move {
            let result = wait.await;
            let mut shutdowns = this.inner.shutdowns.lock().unwrap();
            if let Some(count) = shutdowns.get_mut(&id) {
                *count -= 1;
                if *count == 0 { shutdowns.remove(&id); }
            }
            result
        });
        tokio::time::timeout(timeout, task).await
            .context("The process is still stopping in the background; its folder remains busy until exit is confirmed")??
    }
}

impl Sessions {
    /// At launch, worktree folders are not known yet: the launch appends their exact instructions.
    pub(super) async fn resolve(&self, app: &App, spec: &LaunchSpec, for_launch: bool) -> Result<Preview> {

        crate::attachments::validate(&spec.attachments)?;
        let (cwd, project) = match &spec.target {
            Target::Project { project_id } | Target::Predraft { project_id } => {
                let p = crate::projects::resolve(app, project_id).await?;
                (Some(p.path.clone()), Some(p))
            }
            Target::ExistingPredraft { predraft_id } => {
                let p = crate::predrafts::get(app, predraft_id).await?;
                anyhow::ensure!(p.agents_can_work(), "this Local PR is archived or merged");
                let project = crate::projects::resolve(app, &p.project_id).await?;
                (Some(project.path.clone()), Some(project))
            }
            Target::SessionFolder { session_id } => {
                let source = self.get(session_id).context("source session no longer exists")?;
                let project = match &source.project_id {
                    Some(id) => Some(crate::projects::get(app, id).await?.context("the project no longer exists")?),
                    None => None,
                };
                (Some(source.cwd), project)
            }
            Target::Temp => (None, None),
            Target::None => (None, None),
        };
        let cli = app.paths.cli_command();
        let config = app.config.get();
        let mut context = Vec::new();
        let mut add_dirs = Vec::new();
        let project_target = matches!(spec.target, Target::Project { .. });
        // Git folders of Home always run in fresh worktrees: there is no checkout to share.
        let home_git = project.as_ref().is_some_and(|p| p.home_git);
        let in_checkout = project_target && spec.worktree.is_none() && !home_git;
        let worktree_launch = project_target && (spec.worktree.is_some() || home_git);
        for cp in &spec.context_projects {
            let p = crate::projects::resolve(app, &cp.project_id).await.context("Context folder unavailable; remove it and select it again")?;
            add_dirs.push(p.path.clone());
            context.push(ContextInfo { path: p.path, description: p.description, branch: cp.branch.clone(), editable: cp.editable, own_worktree: cp.editable && !in_checkout });
        }
        crate::prompt_context::validate_mentions(&spec.text, &spec.mentions)?;
        // Where each labelled part of the prompt starts ("What it sees at start").
        let mut marks: Vec<(&str, &str, usize)> = vec![("message", "Your message, its context and presets", 0)];
        let mut prompt = render_prompt(spec, &context, &launch_presets(app, spec, &config).await?);
        marks.push(("skills", "Skills", prompt.len()));
        if !spec.skills.is_empty() {
            let harness = spec.harness.context("Pick a harness for selected skills")?;
            prompt.push_str(&crate::prompt_context::render_skills(app, harness, spec, cwd.as_deref()).await?);
        }
        let traces = self.context_traces(app, &spec.agent_traces).await?;
        marks.push(("traces", "Agent traces", prompt.len()));
        if !traces.is_empty() {
            if !prompt.is_empty() {
                prompt.push_str("\n\n");
            }
            prompt.push_str(&crate::compose::trace_references(&traces, &cli));
        }
        prompt.push_str(&crate::compose::topic_notes(&spec.topics));
        prompt.push_str(&crate::folio_ref::notes(&spec.folios, &cli));
        prompt.push_str(&crate::refs::notes(&spec.refs, &cli));
        marks.push(("folder", "Folder and worktrees", prompt.len()));
        if !for_launch && !prompt.trim().is_empty() && matches!(spec.target, Target::Predraft { .. } | Target::ExistingPredraft { .. }) {
            prompt.push_str("\n\nWork on the Local PR's branch in a fresh worktree. Commit your changes, update the Local PR title and description with launchpad pr update. Do not open a pull request on GitHub.");
        }
        if !for_launch && !prompt.trim().is_empty() && (worktree_launch || context.iter().any(|c| c.own_worktree)) {
            let base = match spec.worktree.as_ref() {
                Some(WorktreeOptions { pull_request: Some(pr), .. }) => format!("the head of pull request {}#{}", pr.repo, pr.number),
                w => {
                    let from = w.and_then(|w| w.base.clone()).map(|b| format!("`{b}`")).unwrap_or_else(|| "the default branch".into());
                    match w.and_then(|w| w.into.clone()).filter(|i| w.and_then(|w| w.base.as_ref()) != Some(i)) {
                        Some(into) => format!("{from}, for `{into}`"),
                        None => from,
                    }
                }
            };
            prompt.push_str(&format!("\n\nLaunchpad worktree: Launchpad creates dedicated Git worktrees for this task at launch{}, and lists their exact folders and branches here.", if worktree_launch { format!(", starting from {base}") } else { String::new() }));
        }
        marks.push(("routine", "Routine", prompt.len()));
        if let Some(r) = &spec.routine {
            prompt.push_str("\n\n");
            prompt.push_str(&crate::routines::instruction(r));
        }
        marks.push(("bot", "The bot and its BOT.md", prompt.len()));
        if let Some(bot) = &spec.bot {
            prompt.push_str("\n\n");
            prompt.push_str(&crate::bots::instruction(bot, &cli));
        }
        marks.push(("final", "Final message", prompt.len()));
        crate::compose::append_initial_summary_instruction(&mut prompt);
        let parts = crate::start::slices(&prompt, &marks);
        Ok(Preview {
            title: title_from(&spec.text, project.as_ref().map(|p| p.name.as_str()).unwrap_or("New session")),
            prompt,
            parts,
            cwd,
            add_dirs,
        })
    }

    pub async fn launch(&self, app: &App, spec: LaunchSpec) -> Result<Session> {
        self.launch_with_id(app, spec, uuid::Uuid::new_v4().to_string()).await
    }

    pub async fn launch_with_id(&self, app: &App, spec: LaunchSpec, id: String) -> Result<Session> {
        self.launch_inner(app, spec, id, false).await
    }

    pub(super) async fn launch_inner(&self, app: &App, spec: LaunchSpec, id: String, automatic: bool) -> Result<Session> {
        crate::role::ensure_runs_agents(app.paths.role)?;
        // Home sends references: their bytes come from Home before the checks read them.
        crate::attachments::fetch_missing(app, &id, &spec.attachments).await?;
        anyhow::ensure!(app.paths.role != crate::role::Role::Runner || !crate::home_link::guard(&app.paths).paused, "This runner takes no new sessions: switch Takes new sessions back on in its own window");
        // This session's lock (a second launch with the same ID waits, then finds the record), and the lock of the
        // session whose folder it continues in, so that one cannot start a turn meanwhile.
        let mut locked = vec![id.clone()];
        if let Target::SessionFolder { session_id } = &spec.target { locked.push(session_id.clone()); }
        let _action = self.action_locks(&locked).await;
        anyhow::ensure!(self.get(&id).is_none(), "a session with this id already exists");
        let mut spec = spec;
        // A pull request on GitHub needs the owner's own launch: others open a Local PR the owner publishes.
        let when_done = spec.when_done();
        if let Some(w) = spec.worktree.as_mut() { w.when_done = when_done; }
        if let Target::SessionFolder { session_id } = &spec.target {
            let source = self.get(session_id).context("source session no longer exists")?;
            anyhow::ensure!(!self.is_busy(&source) && source.status != SessionStatus::InTerminal, "stop the source agent or re-attach its terminal before continuing with another agent");
        }
        let harness = spec.harness.context("pick a harness")?;
        if spec.target == Target::None {
            bail!("pick a project or a temporary folder");
        }
        crate::attachments::validate(&spec.attachments)?;
        anyhow::ensure!(spec.attachments.iter().all(|f| !crate::attachments::is_image(&f.data_url)) || crate::harness::registration(harness).descriptor.capabilities.images, "This harness does not support images.");
        let profile = crate::profiles::resolve(app, harness, &spec)?;
        if crate::harness::registration(harness).descriptor.capabilities.cost && !crate::profiles::supports_cost(app, harness, &spec, spec.model.as_deref()) {
            spec.budget_usd = None;
        }
        check_budget(harness, spec.budget_usd)?;
        crate::spend::ensure(app, &spec)?;
        let (model, effort) = (
            spec.model.clone().or_else(|| profile.as_ref().and_then(|v| v.model.clone())),
            spec.effort.clone().or_else(|| profile.as_ref().and_then(|v| v.effort.clone())),
        );
        let mut preview = self.resolve(app, &spec, true).await?;
        let env = crate::profiles::env(app, harness, &spec, model.as_deref())?;
        if preview.prompt.trim().is_empty() {
            bail!("the prompt is empty");
        }
        let mut worktrees: Vec<(crate::worktrees::TaskWorktree, String)> = vec![];
        let mut setups: Vec<(Project, String)> = vec![];
        // Worktrees of this launch: cloned from Home in the background, before the agent starts.
        let mut created: Vec<String> = vec![];
        // The Local PR this session works on (an existing one), or opens once its branch exists.
        let mut local_pr: Option<(Option<crate::predrafts::Predraft>, String)> = None;
        // Read-only context folders of Home: their clones are refreshed (plain ones brought when missing) before the agent starts.
        let mut views: Vec<Project> = vec![];
        // A runner starts only launches Home placed on it (`runners::record_placement`, before Home relayed the
        // launch): Home gives leases only to sessions it placed (`vault::check_owner`). A failed launch below
        // sends `Forget`, which removes Home's record.
        let prepared = async {
            let (mut cwd, project_id) = match &spec.target {
                Target::Temp => {
                    let workspace = spec.workspace_id.as_deref().context("a temporary folder needs a workspace: launch it from a workspace")?;
                    let p = crate::projects::create_temp(app, &spec.text, workspace).await?;
                    preview.prompt.push_str(&format!("\n\n{}", temp_instruction(&app.paths.cli_command(), &id, workspace, &p.path)));
                    (p.path, Some(p.id))
                }
                Target::Project { project_id } => (preview.cwd.clone().unwrap_or_default(), Some(project_id.clone())),
                Target::Predraft { project_id } => (preview.cwd.clone().unwrap_or_default(), Some(project_id.clone())),
                Target::ExistingPredraft { predraft_id } => {
                    let p = crate::predrafts::get(app, predraft_id).await?;
                    anyhow::ensure!(p.agents_can_work(), "this Local PR is archived or merged");
                    anyhow::ensure!(
                        !p.session_id.as_ref().and_then(|s| self.get(s)).is_some_and(|s| self.is_busy(&s) || s.status == SessionStatus::InTerminal),
                        "the Local PR already has an active agent"
                    );
                    let project = crate::projects::get(app, &p.project_id).await?.context("folder not found")?;
                    let opts = WorktreeOptions { base: Some(p.base_branch.clone()), ..Default::default() };
                    let (w, folder) = crate::worktrees::plan(app, &id, &project, &opts, true, &preview.title, Some(&p.branch)).await?;
                    created.push(w.id.clone());
                    preview.prompt.push_str(&format!("\n\n{}", crate::predrafts::instruction(app, &p)));
                    worktrees.push((w, folder.clone()));
                    setups.push((project, folder.clone()));
                    let project_id = p.project_id.clone();
                    local_pr = Some((Some(p), String::new()));
                    (folder, Some(project_id))
                }
                Target::SessionFolder { session_id } => {
                    let source = self.get(session_id).context("source session no longer exists")?;
                    let into = spec.worktree.as_ref().and_then(|w| w.into.clone()).filter(|b| !b.is_empty());
                    let joined = crate::worktrees::join(app, session_id, &id, into.as_deref()).await?;
                    // Removed folders are cloned again in the background.
                    created.extend(joined.iter().filter(|w| w.state != crate::worktrees::WorktreeState::Active).map(|w| w.id.clone()));
                    worktrees.extend(joined.into_iter().filter(|w| w.state != crate::worktrees::WorktreeState::Discarded).map(|w| {
                        let path = crate::worktrees::folder(&w);
                        (w, path)
                    }));
                    (preview.cwd.clone().context("source session folder missing")?, source.project_id)
                }
                Target::None => unreachable!(),
            };
            // Task worktrees for the project and the editable context projects, before any agent starts.
            let project_target = match &spec.target {
                Target::Project { project_id } | Target::Predraft { project_id } => Some(project_id.as_str()),
                _ => None,
            };
            let target_project = match project_target { Some(id) => crate::projects::get(app, id).await?, None => None };
            // Git folders of Home always run in a fresh worktree.
            let opts = match (&target_project, &spec.worktree) {
                (Some(p), None) if p.home_git => Some(WorktreeOptions::default()),
                (_, w) => w.clone(),
            }.map(|mut o| {
                if matches!(spec.target, Target::Predraft { .. }) { o.when_done = crate::model::WhenDone::LocalPr; }
                o
            });
            let in_checkout = project_target.is_some() && opts.is_none();
            if let (Some(project), Some(opts)) = (target_project, &opts) {
                let (w, folder) = crate::worktrees::plan(app, &id, &project, opts, true, &preview.title, None).await?;
                created.push(w.id.clone());
                cwd = folder.clone();
                worktrees.push((w, folder.clone()));
                setups.push((project, folder));
                if matches!(spec.target, Target::Predraft { .. }) {
                    local_pr = Some((None, preview.title.clone()));
                }
            }
            for cp in &spec.context_projects {
                if cp.editable && !in_checkout { continue }
                if let Some(p) = crate::projects::get(app, &cp.project_id).await?.filter(|p| p.home_git || p.plain) { views.push(p); }
            }
            for cp in spec.context_projects.iter().filter(|c| c.editable && !in_checkout) {
                let Some(project) = crate::projects::get(app, &cp.project_id).await? else { continue };
                let opts = WorktreeOptions { base: cp.branch.clone(), ..Default::default() };
                let (w, folder) = crate::worktrees::plan(app, &id, &project, &opts, false, &preview.title, None).await?;
                if !created.contains(&w.id) { created.push(w.id.clone()); }
                if let Some(d) = preview.add_dirs.iter_mut().find(|d| **d == project.path) { *d = folder.clone(); }
                if !worktrees.iter().any(|(x, _)| x.id == w.id) { setups.push((project, folder.clone())); }
                worktrees.push((w, folder));
            }
            if !worktrees.is_empty() {
                let projects = crate::projects::list(app).await?;
                let when_done = if in_checkout || local_pr.as_ref().is_some_and(|(p, _)| p.is_some()) { crate::model::WhenDone::Keep } else { opts.clone().unwrap_or_default().when_done };
                preview.prompt.push_str(&format!("\n\n{}", crate::worktrees::instruction(app, &id, &worktrees, &projects, when_done)));
            }
            let (images, attachment_dir, instruction) = crate::attachments::prepare(app, &id, &spec.attachments).await?;
            if let Some(dir) = attachment_dir { preview.add_dirs.push(dir.to_string_lossy().into_owned()); }
            preview.prompt.push_str(&instruction);
            if let Some(text) = crate::ext::extensions().suggestion_instruction(app, &id) { preview.prompt.push_str(&format!("\n\n{text}")); }
            let now = now_ms();
            let session = Session {
                runner_id: None,
                generation: 1,
                id: id.clone(),
                harness,
                harness_session_id: None,
                project_id,
                cwd: cwd.clone(),
                title: preview.title.clone(),
                title_pending: true,
                title_refreshed: false,
                status: SessionStatus::Starting,
                archived_done: false,
                status_since: now,
                live: false,
                open_question: false,
                model: model.clone(),
                effort: effort.clone(),
                prompt: preview.prompt.clone(),
                usage: Usage::default(),
                started_at: now,
                ended_at: None,
                last_seq: 0,
                launch_spec: spec.clone(),
                quota_pause: None,
                capacity_retry_count: 0,
                capacity_retry_at: None,
                seen_at: 0,
                last_message: None,
                unsaved: None,
                sleep: None,
            };
            Self::persist(&app.state_db, &session).await?;
            let add_dirs = preview.add_dirs;
            let start = StartSpec {
                permission_mode: spec.permission_mode,
                session_id: id.clone(),
                harness_session_id: None,
                resume: false,
                cwd: PathBuf::from(&cwd),
                title: preview.title,
                model,
                effort,
                add_dirs,
                env,
                jail: None,
            };
            app.bus.publish("sessions", "updated", serde_json::to_value(session.light())?);
            self.inner.records.lock().unwrap().insert(id.clone(), session);
            self.publish_counts(app);
            Ok::<_, anyhow::Error>((start, images, preview.prompt))
        }.await;
        let (start, images, prompt) = match prepared {
            Ok(result) => result,
            Err(error) => {
                self.inner.records.lock().unwrap().remove(&id);
                let failed_id = id.clone();
                let _ = app.state_db.call(move |c| {
                    c.execute("DELETE FROM session WHERE id = ?1", [&failed_id])?;
                    crate::replica::queue(c, &crate::replica::Item::Forget { id: failed_id })
                }).await;
                let mut created = Vec::new();
                for (w, _) in worktrees { if w.sessions == [id.clone()] && !created.iter().any(|c: &crate::worktrees::TaskWorktree| c.id == w.id) { created.push(w); } }
                crate::worktrees::abandon(app, &created).await;
                if let Err(e) = crate::worktrees::leave(app, &id).await {
                    tracing::warn!("could not unlink failed launch worktrees: {e:#}");
                }
                return Err(error);
            }
        };
        setups.retain(|(p, _)| !p.worktree_setup.trim().is_empty());
        // The remote sync, worktree setup and harness start can take minutes (network, slow file
        // systems, setup commands): they run in the background so the launch request returns now.
        let (this, app2, id2) = (self.clone(), app.clone(), id.clone());
        let (ready, wait) = tokio::sync::oneshot::channel::<()>();
        let (cancel, mut cancelled) = tokio::sync::watch::channel(false);
        let (stopped, stop_wait) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            let _ = wait.await;
            let sink = this.sink(&app2, &id2);
            let mut prompt = prompt;
            let mut prepared = true;
            for wid in &created {
                if *cancelled.borrow() { break; }
                let result = match crate::worktrees::get(&app2, wid).await {
                    Ok(w) if w.state == crate::worktrees::WorktreeState::Removed => crate::worktrees::restore(&app2, wid).await,
                    _ => crate::worktrees::prepare(&app2, wid).await,
                };
                if let Err(e) = result {
                    sink.emit(EventBody::Notice { text: format!("Could not prepare the worktree from Home Git: {e:#}") }).await;
                    prepared = false;
                    break;
                }
            }
            // Plain folders and asset folders come down from Home; the session takes their locks.
            if prepared && !*cancelled.borrow() && let Err(e) = crate::plain::prepare(&app2, &id2).await {
                sink.emit(EventBody::Notice { text: format!("Could not prepare the folder from Home: {e:#}") }).await;
                prepared = false;
            }
            for p in &views {
                if *cancelled.borrow() || !prepared { break; }
                let synced = if p.plain { crate::plain::reference(&app2, &p.id, std::path::Path::new(&p.path)).await } else { crate::repo_cache::sync(&app2, p).await.map(drop) };
                if let Err(e) = synced {
                    sink.emit(EventBody::Notice { text: format!("Could not refresh the context folder {}: {e:#}", p.name) }).await;
                }
            }
            if prepared && let Some((existing, title)) = &local_pr && let Some(w) = created.first() && let Ok(w) = crate::worktrees::get(&app2, w).await {
                let opened = match existing {
                    Some(p) => crate::predrafts::open_for_branch(&app2, &p.project_id, &p.branch, Some(&p.base_branch), Some(&id2), None).await,
                    None => crate::predrafts::open_for_branch(&app2, &w.project_id, &w.branch, w.base_branch.as_deref(), Some(&id2), Some(title.clone())).await,
                };
                match opened {
                    Ok(p) => { let _ = crate::worktrees::mark_predraft(&app2, &w.id, &p.id).await; }
                    Err(e) => sink.emit(EventBody::Notice { text: format!("Could not open the Local PR on Home: {e:#}") }).await,
                }
            }
            if !prepared {
                if let Some(s) = this.get(&id2).filter(|s| s.status == SessionStatus::Starting) {
                    // The first message stays in the conversation, before its failure (its ID is the session's).
                    sink.emit(EventBody::UserMessage { text: prompt.clone(), client_id: id2.clone(), queued: false, images: images.clone(), from: s.launch_spec.from.clone() }).await;
                    sink.set_status(SessionStatus::Error).await;
                }
                let _ = stopped.send(());
                this.inner.setups.lock().unwrap().remove(&id2);
                return;
            }
            for (project, folder) in &setups {
                if *cancelled.borrow() { break; }
                if let Err(e) = crate::worktrees::run_setup(&app2, project, folder, &sink, &mut cancelled).await {
                    if *cancelled.borrow() { break; }
                    sink.emit(EventBody::Notice { text: format!("Worktree setup failed: {e:#}") }).await;
                    if this.ensure_shutdown_finished(&id2).is_err() { break; }
                    prompt.push_str(&format!("\n\nNote: the worktree setup command of {} (`{}`) failed in `{folder}`:\n{e:#}\nFix the environment yourself if the task needs it.", project.name, project.worktree_setup.trim()));
                }
            }
            // The contained start (lease from Home, tool installs, knowledge base sync) can take minutes: it runs
            // before the session's action lock, while Stop can still cancel the launch.
            let contained = if !*cancelled.borrow() && this.get(&id2).is_some_and(|s| s.status == SessionStatus::Starting) {
                Some(crate::contain::apply(&app2, start).await)
            } else {
                None
            };
            if *cancelled.borrow() && this.get(&id2).is_some_and(|s| s.status == SessionStatus::Starting) {
                sink.set_status(SessionStatus::Stopped).await;
            }
            // Acknowledge before taking actions: Stop/Archive may still be waiting there.
            let _ = stopped.send(());
            let _action = this.action_lock(&id2).await;
            this.inner.setups.lock().unwrap().remove(&id2);
            let start = match contained {
                Some(start) if !*cancelled.borrow() && this.get(&id2).is_some_and(|s| s.status == SessionStatus::Starting) => start,
                // Cancelled meanwhile: the lease this start took goes (no process holds it).
                Some(Ok(_)) if this.live(&id2).is_none() => return crate::contain::release(&app2, &id2).await,
                _ => return,
            };
            if let Err(e) = this.ensure_shutdown_finished(&id2) {
                if start.is_ok() && this.live(&id2).is_none() { crate::contain::release(&app2, &id2).await; }
                sink.emit(EventBody::Notice { text: e.to_string() }).await;
                sink.set_status(SessionStatus::Stopped).await;
                return;
            }
            this.start_new(&app2, &id2, start, (prompt, images), automatic).await;
        });
        self.inner.setups.lock().unwrap().insert(id.clone(), SetupTask { cancel, stopped: stop_wait });
        let _ = ready.send(());
        Ok(self.get(&id).unwrap())
    }

    /// Starts the harness of a new session and sends its first prompt, or holds it at exhausted quota. `start`: the
    /// contained start `contain::apply` prepared before the caller took the session's action lock.
    pub(super) async fn start_new(&self, app: &App, id: &str, start: Result<StartSpec>, input: (String, Vec<ImageAttachment>), automatic: bool) {
        let (prompt, images) = input;
        let Some(session) = self.get(id) else { return };
        let harness = session.harness;
        let sink = self.sink(app, id);
        let protected = crate::quota::protected(app, harness, &session.launch_spec);
        if protected && (!crate::quota::blockers(app, harness, &session.launch_spec).is_empty() || (automatic && !crate::quota::available(&crate::quota::reading_for(app, harness, &session.launch_spec), now_ms()))) {
            // No process holds the lease until the quota allows the start (`continue_quota` takes it again).
            if start.is_ok() && self.live(id).is_none() { crate::contain::release(app, id).await; }
            if let Err(e) = self.hold_input(app, id, &prompt, images, session.launch_spec.from.clone(), Some(id.to_string())).await {
                sink.fail(&format!("{e:#}")).await;
            }
            return;
        }
        if protected && !crate::quota::available(&crate::quota::reading_for(app, harness, &session.launch_spec), now_ms()) {
            sink.emit(EventBody::Notice { text: "Quota reading unavailable or stale. Starting with best-effort protection; additional spending cannot be ruled out until quota is known.".into() }).await;
        }
        let attached = match start {
            Ok(start) => self.attach_contained(app, harness, start, sink.clone()).await,
            Err(e) => Err(e),
        };
        match attached {
            Ok(live) => {
                // The first message's ID is the session's: the bot chat matches it to the message Home accepted.
                let client_id = id.to_string();
                sink.emit(EventBody::UserMessage { text: prompt.clone(), client_id: client_id.clone(), queued: false, images: images.clone(), from: session.launch_spec.from.clone() }).await;
                sink.set_status(SessionStatus::Working).await;
                let sent = match crate::attachments::for_harness(app, id, &images).await {
                    Ok(images) => live.send(&prompt, &client_id, &images).await,
                    Err(e) => Err(e),
                };
                if let Err(e) = sent {
                    sink.fail(&format!("{e:#}")).await;
                }
            }
            Err(e) => {
                // The first message stays in the conversation, before its failure.
                sink.emit(EventBody::UserMessage { text: prompt, client_id: id.to_string(), queued: false, images, from: session.launch_spec.from.clone() }).await;
                sink.fail(&format!("could not start {}: {e:#}", harness.display())).await
            }
        }
        if let Some(session) = self.get(id).filter(|s| s.status != SessionStatus::Error) {
            crate::titles::generate(app, &session);
            crate::titles::name_topic(app, &session);
        }
    }

    pub(super) async fn attach(&self, app: &App, harness: HarnessKind, start: StartSpec, sink: Sink) -> Result<Arc<dyn LiveSession>> {
        let start = crate::contain::apply(app, start).await?;
        self.attach_contained(app, harness, start, sink).await
    }

    /// Starts the harness with a start `contain::apply` prepared. The caller holds the session's action lock.
    pub(super) async fn attach_contained(&self, app: &App, harness: HarnessKind, start: StartSpec, mut sink: Sink) -> Result<Arc<dyn LiveSession>> {
        let id = start.session_id.clone();
        let instance = self.inner.next_instance.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        sink.instance = instance;
        // A runner replacing an agent CLI holds it for writing: no process starts from a half-installed CLI.
        let cli = crate::harness::CLI_INSTALL.read().await;
        let started = app.harnesses.get(harness).start(app, start, sink.clone()).await;
        drop(cli);
        let live = match started {
            Ok(live) => live,
            Err(e) => {
                // A harness that did not start holds no lease.
                if self.live(&id).is_none() {
                    crate::contain::release(app, &id).await;
                }
                return Err(e);
            }
        };
        self.inner
            .live
            .lock()
            .unwrap()
            .insert(id.clone(), (instance, live.clone()));
        self.inner.process_restarts.lock().unwrap().remove(&id);
        self.update(app, &id, |s| s.live = true).await?;
        Ok(live)
    }

    /// Starts a process for a session that has none (resume).
    /// An idle process waiting for model, effort or permission changes is replaced.
    pub(super) async fn reattach(&self, app: &App, id: &str) -> Result<Arc<dyn LiveSession>> {
        self.ensure_local(id)?;
        self.ensure_shutdown_finished(id)?;
        let s = self.get(id).context("no such session")?;
        if let Some(l) = self.live(id) {
            if !self.inner.process_restarts.lock().unwrap().contains(id) || s.status.is_running() || s.open_question || !l.ready_for_turn() {
                return Ok(l);
            }
            self.detach(app, id).await?;
            // A failed replacement (for example unsupported Auto review) must not leave a
            // session claiming to have a live attachment.
            self.update(app, id, |s| s.live = false).await?;
        }
        if s.launch_spec.harness.is_none() && crate::terminals::tui_running(app, &s) {
            bail!("This imported conversation is still running in another client. Finish it there first.");
        }
        // A temporary folder removed at Archive (or after a move) comes back with its files from Home; Home
        // deletes them 30 days after Archive, and the agent then continues in an empty folder.
        let had_folder = crate::files::is_dir(std::path::Path::new(&s.cwd)).await;
        let restored = match crate::projects::ensure_temp(app, &s).await? {
            Some(p) => {
                if p.path != s.cwd {
                    self.update(app, id, |s| s.cwd = p.path.clone()).await?;
                }
                !had_folder
            }
            None => {
                anyhow::ensure!(!matches!(s.launch_spec.target, Target::Temp) || had_folder, "Its temporary folder is gone: start a new agent");
                false
            }
        };
        // A clean worktree removed by cleanup comes back from its branch before the agent resumes.
        crate::worktrees::restore_for_session(app, id).await?;
        crate::plain::prepare(app, id).await?;
        if restored {
            let s = self.get(id).context("no such session")?;
            if crate::plain::walk(std::path::Path::new(&s.cwd)).is_empty() {
                self.sink(app, id).emit(EventBody::Notice { text: "Its temporary folder is empty: Home had no files of it (they are deleted 30 days after Archive).".into() }).await;
            }
        }
        let s = self.get(id).context("no such session")?;
        let worktrees = crate::worktrees::for_session(app, id).await?;
        let mut add_dirs = Vec::new();
        let attachment_dir = crate::attachments::dir(app, &s.id);
        if crate::files::is_dir(&attachment_dir).await { add_dirs.push(attachment_dir.to_string_lossy().into_owned()); }
        for cp in &s.launch_spec.context_projects {
            if let Ok(p) = crate::projects::resolve(app, &cp.project_id).await {
                // Editable context projects resolve to their recorded worktree, never to today's checkout.
                let in_checkout = matches!(s.launch_spec.target, Target::Project { .. }) && s.launch_spec.worktree.is_none() && !worktrees.iter().any(|w| w.primary);
                if cp.editable && !in_checkout {
                    add_dirs.push(crate::worktrees::context_folder(app, &worktrees, &p).await?);
                } else {
                    // A session that moved here has no copy of its read-only context folders yet.
                    if p.plain {
                        crate::plain::reference(app, &p.id, std::path::Path::new(&p.path)).await?;
                    } else if p.home_git && !crate::files::is_dir(&p.path).await {
                        crate::repo_cache::sync(app, &p).await?;
                    }
                    add_dirs.push(p.path);
                }
            }
        }
        for w in worktrees.iter().filter(|w| !w.primary && w.state == crate::worktrees::WorktreeState::Active) {
            let dir = crate::worktrees::folder(w);
            if !add_dirs.contains(&dir) { add_dirs.push(dir); }
        }
        if !crate::files::is_dir(&s.cwd).await {
            bail!("the folder {} no longer exists", s.cwd);
        }
        let cwd = PathBuf::from(&s.cwd);
        let env = crate::profiles::env(app, s.harness, &s.launch_spec, s.model.as_deref())?;
        let start = StartSpec {
            permission_mode: s.launch_spec.permission_mode,
            session_id: s.id.clone(),
            harness_session_id: s.harness_session_id.clone(),
            resume: s.harness_session_id.is_some(),
            cwd,
            title: s.title.clone(),
            model: s.model.clone(),
            effort: s.effort.clone(),
            add_dirs,
            env,
            jail: None,
        };
        let live = self.attach(app, s.harness, start, self.sink(app, id)).await?;
        Ok(live)
    }

    /// Hold an input without sending it to a harness. This also covers launches at exhausted quota.
    pub(super) async fn hold_input(&self, app: &App, id: &str, text: &str, images: Vec<ImageAttachment>, from: Option<crate::model::MessageFrom>, client_id: Option<String>) -> Result<()> {
        self.inner.quota_stops.lock().unwrap().remove(id);
        let s = self.get(id).context("no such session")?;
        let client_id = client_id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        self.update(app, id, |s| s.quota_pause = Some(QuotaPause {
            phase: QuotaPausePhase::Paused, blockers: crate::quota::blockers(app, s.harness, &s.launch_spec),
            message: text.into(), images: images.clone(), client_id: Some(client_id.clone()), error: None, messages: vec![],
        })).await?;
        let sink = self.sink(app, id);
        sink.emit(EventBody::UserMessage { text: text.into(), client_id, queued: true, images, from }).await;
        sink.set_status(SessionStatus::QuotaPaused).await;
        sink.emit(EventBody::Notice { text: format!("{} input held until a fresh quota reading confirms included quota availability.", s.harness.display()) }).await;
        Ok(())
    }

    /// Durable claim before sending, serialized with owner actions. An uncertain send is never replayed.
    /// A session whose action lock is taken waits for the next check; the resume (which may download, install and
    /// start a process) runs in its own task with that session's lock, so the guard never waits for it.
    pub(crate) async fn check_capacity_retries(&self, app: &App) -> Result<()> {
        // A runner's session (on Home) is retried by its runner.
        for pending in self.list().into_iter().filter(|s| s.runner_id.is_none() && s.capacity_retry_at.is_some_and(|at| at <= now_ms())) {
            let Some(action) = self.try_action_lock(&pending.id) else { continue };
            let Some(s) = self.get(&pending.id) else { continue };
            if s.capacity_retry_at.is_none_or(|at| at > now_ms()) { continue; }
            if self.is_busy(&s) { continue; }
            let eligible = s.status == SessionStatus::Error && s.capacity_retry_count < MAX_CAPACITY_RETRIES;
            self.update(app, &s.id, |s| {
                s.capacity_retry_at = None;
                if eligible { s.capacity_retry_count += 1; }
            }).await?;
            if !eligible { continue; }
            let (this, app) = (self.clone(), app.clone());
            tokio::spawn(async move {
                let _action = action;
                if let Err(e) = this.capacity_retry(&app, &s).await {
                    tracing::warn!("capacity retry {}: {e:#}", s.id);
                }
            });
        }
        Ok(())
    }

    /// One claimed capacity retry. The caller holds the session's action lock.
    pub(super) async fn capacity_retry(&self, app: &App, s: &Session) -> Result<()> {
        let sink = self.sink(app, &s.id);
        sink.emit(EventBody::Notice { text: format!("Retrying automatically after model capacity failure (attempt {}/{}).", s.capacity_retry_count + 1, MAX_CAPACITY_RETRIES) }).await;
        if let Err(e) = self.resume_locked(app, &s.id, false, true).await {
            let message = format!("{e:#}");
            // Reattachment may fail before a turn starts and before the adapter can
            // report a new status. Only an explicit capacity error earns another retry.
            if is_capacity_error(&message) {
                self.update(app, &s.id, |s| {
                    if s.status == SessionStatus::Error && s.capacity_retry_at.is_none() && s.capacity_retry_count < MAX_CAPACITY_RETRIES {
                        s.capacity_retry_at = Some(now_ms() + CAPACITY_RETRY_DELAY_MS);
                    }
                }).await?;
            }
            sink.emit(EventBody::Notice { text: format!("Automatic capacity retry could not continue: {message}. If no retry is pending, inspect the trace and resume manually.") }).await;
        }
        Ok(())
    }

    /// Each session is checked under its action lock, so a user Stop and an automatic continuation never race. A
    /// session whose lock is taken (an action of its own runs) waits for the next check; an automatic continuation
    /// (which may download, install and start a process) runs in its own task with that lock, so one session never
    /// delays the checks of the others.
    pub(crate) async fn check_quotas(&self, app: &App) -> Result<()> {
        crate::spend::publish(app);
        for s in self.list() {
            // Home: a runner's session is protected by its runner, which receives Home's readings.
            if s.runner_id.is_some() { continue; }
            // Setup has not sent a task yet; start_new checks quota after it finishes.
            if self.setup_running(&s.id) { continue; }
            let Some(action) = self.try_action_lock(&s.id) else { continue };
            // The record as it is now that no action of this session runs.
            let Some(s) = self.get(&s.id) else { continue };
            if s.live && s.quota_pause.is_none() && (s.status.is_running() || s.status.is_done()) {
                crate::titles::generate(app, &s);
                crate::titles::name_topic(app, &s);
            }
            if let Some(live) = self.live(&s.id)
                && let Some(after) = live.idle_release()
                && idle_release_due(&s, now_ms(), after)
                && self.ensure_shutdown_finished(&s.id).is_ok()
                && live.ready_for_turn() && !live.turn_active() && !live.has_background_work() && !live.has_open_questions() {
                drop(live);
                // Its own task with the lock: stopping a process may take seconds, and the others are checked now.
                let (this, app, id) = (self.clone(), app.clone(), s.id.clone());
                tokio::spawn(async move {
                    let _action = action;
                    if let Err(e) = this.release(&app, &id).await { tracing::info!("release idle process {id}: {e:#}"); }
                });
                continue;
            }
            if (s.budget_reached() || !crate::spend::reached(app, &s.launch_spec).is_empty()) && s.status.is_running() && !s.open_question
                && let Some(live) = self.live(&s.id).filter(|l| l.turn_active())
                && self.inner.budget_interrupts.lock().unwrap().get(&s.id).is_none_or(|at| now_ms() - at >= 10_000) {
                // Ends as Stopped, with an automatic-stop notification; the trace also records the reached limit.
                self.inner.budget_interrupts.lock().unwrap().insert(s.id.clone(), now_ms());
                self.inner.stop_requests.lock().unwrap().insert(s.id.clone());
                if let Err(e) = live.interrupt().await {
                    tracing::warn!("cost limit interrupt {}: {e:#}", s.id);
                }
            }
            if !crate::quota::protected(app, s.harness, &s.launch_spec) && s.quota_pause.is_none() { continue; }
            let blockers = self.quota_blockers(app, &s);
            if s.quota_pause.is_none() && s.status.is_running() && !blockers.is_empty() {
                // A question is already idle: leave it answerable after the quota resets.
                if s.open_question { continue; }
                self.update(app, &s.id, |s| s.quota_pause = Some(QuotaPause {
                    phase: QuotaPausePhase::Pausing, blockers, message: "Continue the interrupted task from where you stopped; check the current state before repeating actions.".into(),
                    images: vec![], client_id: None, error: None, messages: vec![],
                })).await?;
                // The turn's real end (the harness reports it once interrupted) makes it QuotaPaused (`transition`).
                self.sink(app, &s.id).emit(EventBody::Notice { text: "Included quota exhausted. Pausing until quota is available; interruption is best effort and commands may still finish.".into() }).await;
                self.inner.quota_interrupts.lock().unwrap().insert(s.id.clone(), now_ms());
                if let Some(live) = self.live(&s.id)
                    && let Err(e) = live.interrupt().await {
                        // A turn may have ended naturally while we were deciding to interrupt.
                        if !live.ready_for_turn() {
                            self.update(app, &s.id, |s| { if let Some(p) = &mut s.quota_pause { p.phase = QuotaPausePhase::NeedsAttention; p.error = Some(format!("Could not interrupt: {e:#}. Stop or inspect the trace.")); } }).await?;
                        }
                }
            }
            let Some(current) = self.get(&s.id) else { continue };
            let Some(p) = current.quota_pause.clone() else { continue };
            if matches!(p.phase, QuotaPausePhase::Pausing | QuotaPausePhase::Paused) && !self.quota_blockers(app, &s).is_empty()
                && self.live(&s.id).is_some_and(|l| l.turn_active())
                && self.inner.quota_interrupts.lock().unwrap().get(&s.id).is_none_or(|at| now_ms() - at >= 10_000) {
                self.inner.quota_interrupts.lock().unwrap().insert(s.id.clone(), now_ms());
                self.update(app, &s.id, |s| { if let Some(p) = &mut s.quota_pause { p.phase = QuotaPausePhase::Pausing; } }).await?;
                if let Some(l) = self.live(&s.id) { let _ = l.interrupt().await; }
            }
            // A provider's spending/billing limit is independent of the included-quota
            // meters. Its explanation stays visible and only an owner retry can clear it.
            if matches!(p.phase, QuotaPausePhase::NeedsAttention | QuotaPausePhase::Resuming | QuotaPausePhase::ProviderLimited) { continue; }
            if self.live(&s.id).is_some_and(|l| !l.ready_for_turn()) { continue; }
            if p.phase == QuotaPausePhase::Pausing {
                self.update(app, &s.id, |s| { if let Some(p) = &mut s.quota_pause { p.phase = QuotaPausePhase::Paused; } }).await?;
            }
            // The turn is over (or its process is gone) without a final status from the harness: this is its end.
            if self.get(&s.id).is_some_and(|s| s.status != SessionStatus::QuotaPaused) {
                self.sink(app, &s.id).set_status(SessionStatus::QuotaPaused).await;
            }
            let reading = crate::quota::reading_for(app, s.harness, &s.launch_spec);
            let mode = if crate::quota::available(&reading, now_ms()) { Some(QuotaResume::Automatic) }
                else if crate::quota::reset_fallback(&reading, &p, now_ms()) { Some(QuotaResume::ResetFallback) }
                else { None };
            if !current.budget_reached() && crate::spend::reached(app, &current.launch_spec).is_empty() && let Some(mode) = mode {
                let (this, app, id) = (self.clone(), app.clone(), s.id.clone());
                tokio::spawn(async move {
                    let _action = action;
                    if let Err(e) = this.continue_quota(&app, &id, mode).await {
                        tracing::warn!("quota continuation {id}: {e:#}");
                    }
                });
            }
        }
        Ok(())
    }
}
