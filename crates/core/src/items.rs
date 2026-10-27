//! Cached GitHub details, explicit owner mutations and PR diffs.
use crate::{App, util::now_ms};
use anyhow::{Context, Result, ensure};
use reqwest::Method;
use serde_json::{Value, json};

pub fn validate(repo: &str) -> Result<()> {
    let parts: Vec<_> = repo.split('/').collect();
    ensure!(
        parts.len() == 2
            && parts.iter().all(|s| !s.is_empty()
                && *s != "."
                && *s != ".."
                && s.chars()
                    .all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c))),
        "invalid repository"
    );
    Ok(())
}
pub async fn cached(app: &App, key: String) -> Result<Option<Value>> {
    app.cache_db
        .call(move |c| {
            use rusqlite::OptionalExtension;
            let s: Option<String> = c
                .query_row("SELECT json FROM gh_response WHERE url=?1", [key], |r| {
                    r.get(0)
                })
                .optional()?;
            Ok(s.map(|s| serde_json::from_str(&s)).transpose()?)
        })
        .await
}
async fn store(app: &App, key: String, value: &Value) -> Result<()> {
    let s = serde_json::to_string(value)?;
    app.cache_db
        .call(move |c| {
            c.execute(
                "INSERT OR REPLACE INTO gh_response(url,json) VALUES(?1,?2)",
                rusqlite::params![key, s],
            )?;
            Ok(())
        })
        .await
}
/// Labels and reviewers seen in this repository's cached items, for the pickers. Never calls GitHub.
pub async fn repo_meta(app: &App, repo: &str) -> Result<Value> {
    validate(repo)?;
    let (items, list) = (format!("item:{repo}:"), format!("project-items:{repo}"));
    let rows: Vec<String> = app
        .cache_db
        .call(move |c| {
            let mut st = c.prepare(
                "SELECT json FROM gh_response WHERE substr(url,1,length(?1))=?1 OR url=?2",
            )?;
            let rows = st
                .query_map([items, list], |r| r.get(0))?
                .collect::<rusqlite::Result<Vec<String>>>()?;
            Ok(rows)
        })
        .await?;
    let mut labels = std::collections::BTreeMap::new();
    let mut reviewers = std::collections::BTreeSet::new();
    let mut add_labels = |v: &Value| {
        for l in v["labels"]["nodes"].as_array().into_iter().flatten() {
            if let Some(name) = l["name"].as_str() {
                labels
                    .entry(name.to_string())
                    .or_insert_with(|| l["color"].clone());
            }
        }
    };
    for v in rows
        .iter()
        .filter_map(|s| serde_json::from_str::<Value>(s).ok())
    {
        add_labels(&v);
        for i in v["prs"]
            .as_array()
            .into_iter()
            .chain(v["issues"].as_array())
            .flatten()
        {
            add_labels(i);
        }
        let mut add = |u: &Value| {
            if let Some(login) = u["login"].as_str() {
                reviewers.insert(login.to_string());
            } else if let Some(slug) = u["slug"].as_str() {
                reviewers.insert(format!("team:{slug}"));
            }
        };
        for r in v["reviews"]["nodes"].as_array().into_iter().flatten() {
            add(&r["author"]);
        }
        for r in v["reviewRequests"]["nodes"]
            .as_array()
            .into_iter()
            .flatten()
        {
            add(&r["requestedReviewer"]);
        }
        for e in v["timelineItems"]["nodes"].as_array().into_iter().flatten() {
            add(&e["requestedReviewer"]);
        }
    }
    let labels: Vec<_> = labels
        .into_iter()
        .map(|(name, color)| json!({"name":name,"color":color}))
        .collect();
    Ok(json!({"labels":labels,"reviewers":reviewers}))
}
pub async fn detail(app: &App, repo: &str, n: u64, force: bool) -> Result<Value> {
    validate(repo)?;
    let common = r#"id number title url body bodyHTML state createdAt updatedAt author { login avatarUrl } participants(first:8) { nodes { login avatarUrl } } assignees(first:20) { nodes { login } } labels(first:50) { nodes { name color } } milestone { title }"#;
    let events = r#"__typename ... on IssueComment { id body bodyHTML url createdAt author { login avatarUrl } } ... on ClosedEvent { createdAt actor { login } } ... on ReopenedEvent { createdAt actor { login } } ... on LabeledEvent { createdAt actor { login } label { name } } ... on UnlabeledEvent { createdAt actor { login } label { name } } ... on AssignedEvent { createdAt actor { login } assignee { ... on User { login } } } ... on UnassignedEvent { createdAt actor { login } assignee { ... on User { login } } } ... on RenamedTitleEvent { createdAt actor { login } previousTitle currentTitle } ... on CrossReferencedEvent { createdAt actor { login } source { ... on Issue { title url number } ... on PullRequest { title url number } } }"#;
    let pr_events = r#"... on PullRequestReview { id body bodyHTML url submittedAt state author { login avatarUrl } } ... on PullRequestCommit { commit { oid messageHeadline committedDate url author { name } } } ... on MergedEvent { createdAt actor { login } } ... on ReadyForReviewEvent { createdAt actor { login } } ... on ConvertToDraftEvent { createdAt actor { login } } ... on ReviewRequestedEvent { createdAt actor { login } requestedReviewer { ... on User { login } ... on Team { name } } }"#;
    let pr = r#"isDraft merged mergeable mergeStateStatus reviewDecision headRefName baseRefName headRefOid baseRefOid viewerCanUpdate viewerCanMergeAsAdmin repository { mergeCommitAllowed squashMergeAllowed rebaseMergeAllowed } reviewRequests(first:30) { nodes { requestedReviewer { ... on User { login } ... on Team { name slug } } } } reviews(last:30) { nodes { state author { login } } } closingIssuesReferences(first:30) { nodes { number title url } } commits(last:1) { nodes { commit { statusCheckRollup { state contexts(first:100) { nodes { __typename ... on CheckRun { name conclusion status detailsUrl } ... on StatusContext { context state targetUrl } } } } } } }"#;
    let key = format!("item:{repo}:{n}");
    if !force && let Some(mut v) = cached(app, key.clone()).await? {
        if now_ms() - v["fetched_at"].as_i64().unwrap_or(0) < 60_000 {
            return Ok(v);
        }
        let probe=app.github.graphql(format!("query {{ node(id:{}) {{ ... on Issue {{ {common} }} ... on PullRequest {{ {common} {pr} }} }} }}",json!(v["id"]))).await?;
        if probe["node"]["updatedAt"] == v["updatedAt"] {
            for (k, field) in probe["node"].as_object().context("GitHub header")? {
                v[k] = field.clone();
            }
            proxy_images(app, &mut v).await;
            v["fetched_at"] = json!(now_ms());
            store(app, key, &v).await?;
            return Ok(v);
        }
    }
    let (owner, name) = repo.split_once('/').context("repository")?;
    let mut cursor: Option<String> = None;
    let mut all = vec![];
    let mut item;
    loop {
        let after = cursor
            .as_ref()
            .map(|s| format!(",after:{}", json!(s)))
            .unwrap_or_default();
        let query = format!(
            "query {{ repository(owner:{},name:{}) {{ issueOrPullRequest(number:{n}) {{ __typename ... on Issue {{ {common} timelineItems(first:100{after}) {{ nodes {{ {events} }} pageInfo {{ hasNextPage endCursor }} }} }} ... on PullRequest {{ {common} {pr} timelineItems(first:100{after}) {{ nodes {{ {events} {pr_events} }} pageInfo {{ hasNextPage endCursor }} }} }} }} }} }}",
            json!(owner),
            json!(name)
        );
        let data = app.github.graphql(query).await?;
        item = data["repository"]["issueOrPullRequest"].clone();
        ensure!(!item.is_null(), "GitHub item not found");
        all.extend(
            item["timelineItems"]["nodes"]
                .as_array()
                .cloned()
                .unwrap_or_default(),
        );
        if item["timelineItems"]["pageInfo"]["hasNextPage"] != true {
            break;
        }
        cursor = item["timelineItems"]["pageInfo"]["endCursor"]
            .as_str()
            .map(str::to_string);
        ensure!(cursor.is_some(), "missing timeline cursor");
    }
    item["timelineItems"]["nodes"] = json!(all);
    if item["__typename"] == "PullRequest" {
        let mut threads = vec![];
        let mut cursor: Option<String> = None;
        loop {
            let after = cursor
                .as_ref()
                .map(|s| format!(",after:{}", json!(s)))
                .unwrap_or_default();
            let data = app.github.graphql(format!("query {{ node(id:{}) {{ ... on PullRequest {{ reviewThreads(first:100{after}) {{ nodes {{ id path line isResolved isOutdated comments(first:100) {{ nodes {{ id body bodyHTML url createdAt author {{ login avatarUrl }} }} pageInfo {{ hasNextPage endCursor }} }} }} pageInfo {{ hasNextPage endCursor }} }} }} }} }}",json!(item["id"]))).await?;
            let conn = &data["node"]["reviewThreads"];
            for mut thread in conn["nodes"].as_array().cloned().unwrap_or_default() {
                while thread["comments"]["pageInfo"]["hasNextPage"] == true {
                    let after = thread["comments"]["pageInfo"]["endCursor"].clone();
                    let more = app.github.graphql(format!("query {{ node(id:{}) {{ ... on PullRequestReviewThread {{ comments(first:100,after:{}) {{ nodes {{ id body bodyHTML url createdAt author {{ login avatarUrl }} }} pageInfo {{ hasNextPage endCursor }} }} }} }} }}",json!(thread["id"]),json!(after))).await?;
                    let extra = &more["node"]["comments"];
                    ensure!(extra.is_object(), "review comments unavailable");
                    thread["comments"]["nodes"]
                        .as_array_mut()
                        .context("review comments")?
                        .extend(extra["nodes"].as_array().cloned().unwrap_or_default());
                    thread["comments"]["pageInfo"] = extra["pageInfo"].clone();
                }
                threads.push(thread);
            }
            if conn["pageInfo"]["hasNextPage"] != true {
                break;
            }
            cursor = conn["pageInfo"]["endCursor"].as_str().map(str::to_string);
            ensure!(cursor.is_some(), "missing thread cursor");
        }
        item["reviewThreads"] = json!({"nodes":threads});
    }
    item["fetched_at"] = json!(now_ms());
    item["repo"] = json!(repo);
    proxy_images(app, &mut item).await;
    store(app, key, &item).await?;
    Ok(item)
}
pub async fn project_items(app: &App, repo: &str, force: bool) -> Result<Value> {
    validate(repo)?;
    let key = format!("project-items:{repo}");
    if !force
        && let Some(v) = cached(app, key.clone()).await?
        && v["prs"].is_array()
        && v["issues"].is_array()
        && now_ms() - v["fetched_at"].as_i64().unwrap_or(0) < 300_000
    {
        return Ok(v);
    }
    let (owner, name) = repo.split_once('/').context("repository")?;
    let fields = "number title url updatedAt state author { login } labels(first:50) { nodes { name color } }";
    let query = format!(
        "query {{ repository(owner:{},name:{}) {{ pullRequests(first:30,states:OPEN,orderBy:{{field:UPDATED_AT,direction:DESC}}) {{ nodes {{ {fields} isDraft }} }} issues(first:30,states:OPEN,orderBy:{{field:UPDATED_AT,direction:DESC}}) {{ nodes {{ {fields} }} }} }} }}",
        json!(owner),
        json!(name)
    );
    let data = app.github.graphql(query).await?;
    ensure!(
        data["repository"].is_object(),
        "GitHub repository {repo} is unavailable"
    );
    ensure!(
        data["repository"]["pullRequests"]["nodes"].is_array()
            && data["repository"]["issues"]["nodes"].is_array(),
        "GitHub did not return issue and pull request lists for {repo}"
    );
    let v = json!({"fetched_at":now_ms(),"prs":data["repository"]["pullRequests"]["nodes"],"issues":data["repository"]["issues"]["nodes"]});
    store(app, key, &v).await?;
    Ok(v)
}
pub async fn mutate(app: &App, repo: &str, n: u64, action: &str, body: Value) -> Result<Value> {
    validate(repo)?;
    let issue = format!("/repos/{repo}/issues/{n}");
    let pull = format!("/repos/{repo}/pulls/{n}");
    let (method, path, payload) = match action {
        "comment" => {
            ensure!(
                body["body"].as_str().is_some_and(|s| !s.trim().is_empty()),
                "empty comment"
            );
            (
                Method::POST,
                format!("{issue}/comments"),
                json!({"body":body["body"]}),
            )
        }
        "review" => {
            ensure!(
                ["COMMENT", "APPROVE", "REQUEST_CHANGES"]
                    .contains(&body["event"].as_str().unwrap_or("")),
                "invalid review"
            );
            ensure!(
                body["commit_id"].as_str().is_some(),
                "missing reviewed commit"
            );
            // Optional line comments from the Changes view: path, line, side (LEFT/RIGHT) and body each.
            let mut review = json!({"event":body["event"],"body":body["body"],"commit_id":body["commit_id"]});
            if let Some(comments) = body["comments"].as_array().filter(|c| !c.is_empty()) {
                ensure!(comments.len() <= 100, "too many line comments");
                let mut out = Vec::with_capacity(comments.len());
                for c in comments {
                    let (Some(path), Some(line), Some(text)) = (c["path"].as_str(), c["line"].as_u64(), c["body"].as_str()) else { anyhow::bail!("invalid line comment") };
                    let side = c["side"].as_str().unwrap_or("RIGHT");
                    ensure!(side == "LEFT" || side == "RIGHT", "invalid line comment side");
                    ensure!(!text.trim().is_empty(), "empty line comment");
                    out.push(json!({"path":path,"line":line,"side":side,"body":text}));
                }
                review["comments"] = Value::Array(out);
            }
            (Method::POST, format!("{pull}/reviews"), review)
        }
        "reviewers" => (
            Method::POST,
            format!("{pull}/requested_reviewers"),
            json!({"reviewers":body["reviewers"],"team_reviewers":body["team_reviewers"]}),
        ),
        "labels" => (
            Method::PUT,
            format!("{issue}/labels"),
            json!({"labels":body["labels"]}),
        ),
        "state" => {
            ensure!(
                ["open", "closed"].contains(&body["state"].as_str().unwrap_or("")),
                "invalid state"
            );
            (
                Method::PATCH,
                if body["kind"] == "pr" { pull } else { issue },
                json!({"state":body["state"],"state_reason":body["state_reason"]}),
            )
        }
        "merge" => {
            ensure!(
                ["merge", "squash", "rebase"]
                    .contains(&body["merge_method"].as_str().unwrap_or("")),
                "invalid merge method"
            );
            ensure!(body["sha"].as_str().is_some(), "missing reviewed head");
            (
                Method::PUT,
                format!("{pull}/merge"),
                json!({"sha":body["sha"],"merge_method":body["merge_method"]}),
            )
        }
        "draft" | "ready" => {
            let d = detail(app, repo, n, true).await?;
            let op = if action == "draft" {
                "convertPullRequestToDraft"
            } else {
                "markPullRequestReadyForReview"
            };
            let v = app
                .github
                .graphql(format!(
                    "mutation {{ {op}(input:{{pullRequestId:{}}}) {{ pullRequest {{ id }} }} }}",
                    json!(d["id"])
                ))
                .await?;
            ensure!(!v[op].is_null(), "GitHub rejected draft state change");
            return Ok(v);
        }
        _ => anyhow::bail!("unknown action"),
    };
    let v = app
        .github
        .request(method, &path, Some(payload), false)
        .await?
        .value;
    if action == "merge" {
        ensure!(v["merged"] == true, "{}", v["message"]);
    }
    Ok(v)
}

fn image_url(url: &str) -> Result<reqwest::Url> {
    let u = reqwest::Url::parse(url)?;
    ensure!(
        u.scheme() == "https"
            && u.port_or_known_default() == Some(443)
            && u.username().is_empty()
            && u.password().is_none()
            && u.host_str().is_some_and(|h| [
                "avatars.githubusercontent.com",
                "camo.githubusercontent.com",
                "user-images.githubusercontent.com",
                "private-user-images.githubusercontent.com",
                "github.com",
                "github-production-user-asset-6210df.s3.amazonaws.com"
            ]
            .contains(&h)),
        "unsupported image host"
    );
    if u.host_str() == Some("github.com") {
        ensure!(
            u.path().starts_with("/user-attachments/"),
            "unsupported image path"
        );
    }
    Ok(u)
}
// Only public GitHub image hosts; no credentials, local addresses or redirects to arbitrary hosts.
pub async fn image(app: &App, url: &str) -> Result<(Vec<u8>, String)> {
    use sha2::{Digest, Sha256};
    let mut u = image_url(url)?;
    let dir = app.paths.data_dir.join("images");
    tokio::fs::create_dir_all(&dir).await?;
    let path = dir.join(hex::encode(Sha256::digest(url.as_bytes())));
    if let (Ok(bytes), Ok(mime)) = (
        tokio::fs::read(&path).await,
        tokio::fs::read_to_string(path.with_extension("mime")).await,
    ) {
        return Ok((bytes, mime));
    }
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_secs(15))
        .build()?;
    let mut redirects = 0;
    let res = loop {
        let res = client.get(u.clone()).send().await?;
        if res.status().is_redirection() {
            ensure!(redirects < 3, "too many image redirects");
            redirects += 1;
            let target = u.join(
                res.headers()
                    .get("location")
                    .context("image redirect without location")?
                    .to_str()?,
            )?;
            u = image_url(target.as_str())?;
        } else {
            break res.error_for_status()?;
        }
    };
    let mime = res
        .headers()
        .get("content-type")
        .and_then(|s| s.to_str().ok())
        .unwrap_or("")
        .split(';')
        .next()
        .unwrap_or("")
        .to_string();
    ensure!(
        ["image/png", "image/jpeg", "image/gif", "image/webp"].contains(&mime.as_str()),
        "unsupported image type"
    );
    ensure!(
        res.content_length().unwrap_or(0) <= 5_000_000,
        "image too large"
    );
    let mut res = res;
    let mut bytes = vec![];
    while let Some(chunk) = res.chunk().await? {
        ensure!(bytes.len() + chunk.len() <= 5_000_000, "image too large");
        bytes.extend_from_slice(&chunk);
    }
    tokio::fs::write(&path, &bytes).await?;
    tokio::fs::write(path.with_extension("mime"), &mime).await?;
    Ok((bytes, mime))
}
fn rewrite_image_src(html: &str, url: &str, local: &str) -> String {
    html.replace(&format!("src=\"{}\"", url.replace('&', "&amp;")), &format!("src=\"{local}\""))
        .replace(&format!("src=\"{url}\""), &format!("src=\"{local}\""))
}

/// Bound disk retention for public image bytes and MIME sidecars, including orphan files.
pub async fn collect_images(dir: &std::path::Path) -> Result<()> {
    let mut entries = match tokio::fs::read_dir(dir).await {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e.into()),
    };
    while let Some(entry) = entries.next_entry().await? {
        let meta = entry.metadata().await?;
        if meta.is_file() && meta.modified()?.elapsed().unwrap_or_default().as_secs() > 21 * 86400 {
            tokio::fs::remove_file(entry.path()).await?;
        }
    }
    Ok(())
}

async fn proxy_images(app: &App, value: &mut Value) {
    // HTML attributes are replaced before browser sanitization. Failed images keep their URL.
    fn urls(v: &Value, out: &mut Vec<String>) {
        match v {
            Value::Object(m) => {
                for (k, v) in m {
                    if k == "avatarUrl" {
                        if let Some(s) = v.as_str() {
                            out.push(s.into());
                        }
                    } else if k == "bodyHTML" {
                        if let Some(s) = v.as_str() {
                            for rest in s.split("src=\"").skip(1) {
                                if let Some(url) = rest.split('"').next() {
                                    out.push(url.replace("&amp;", "&"));
                                }
                            }
                        }
                    } else {
                        urls(v, out);
                    }
                }
            }
            Value::Array(a) => {
                for v in a {
                    urls(v, out);
                }
            }
            _ => {}
        }
    }
    let mut sources = vec![];
    urls(value, &mut sources);
    sources.sort();
    sources.dedup();
    let mut replacements = vec![];
    for url in sources {
        if image(app, &url).await.is_ok() {
            let encoded: String =
                reqwest::Url::parse_with_params("http://localhost/img", [("url", url.as_str())])
                    .unwrap()
                    .query()
                    .unwrap()
                    .into();
            replacements.push((url, format!("/img?{encoded}")));
        }
    }
    fn rewrite(v: &mut Value, replacements: &[(String, String)]) {
        match v {
            Value::Object(m) => {
                for (k, v) in m {
                    if k == "bodyHTML" || k == "avatarUrl" {
                        if let Some(s) = v.as_str() {
                            let mut s = s.to_string();
                            for (url, local) in replacements {
                                if k == "avatarUrl" {
                                    if &s == url { s = local.clone(); }
                                } else {
                                    s = rewrite_image_src(&s, url, local);
                                }
                            }
                            *v = json!(s);
                        }
                    } else {
                        rewrite(v, replacements);
                    }
                }
            }
            Value::Array(a) => {
                for v in a {
                    rewrite(v, replacements);
                }
            }
            _ => {}
        }
    }
    rewrite(value, &replacements);
}

pub async fn diff(app: &App, repo: &str, n: u64) -> Result<Value> {
    let d = detail(app, repo, n, false).await?;
    ensure!(d["__typename"] == "PullRequest", "not a pull request");
    let registered = crate::projects::for_repo(app, repo).await?;
    let key = format!(
        "diff:{repo}:{}:{}:{}",
        d["baseRefOid"],
        d["headRefOid"],
        registered.as_ref().map(|p| p.id.as_str()).unwrap_or("api")
    );
    if let Some(v) = cached(app, key.clone()).await? {
        return Ok(v);
    }
    let mut files = vec![];
    let mut project = None;
    let mut local_error = None;
    if let Some(p) = registered {
        let root = std::path::Path::new(&p.path);
        let result = async {
            let remote = crate::git::remotes(root)
                .await
                .into_iter()
                .find(|(_, url)| crate::git::github_repo(url).is_some_and(|r| r.eq_ignore_ascii_case(repo)))
                .map(|(name, _)| name)
                .context("missing mapped remote")?;
            let head = format!("+refs/pull/{n}/head:refs/launchpad/pr/{n}");
            let base = d["baseRefOid"].as_str().context("missing base")?;
            let sha = d["headRefOid"].as_str().context("missing head")?;
            crate::procs::output(
                &app.env,
                "git",
                &["fetch", &remote, &head, base],
                Some(root),
            )
            .await?;
            crate::procs::output(
                &app.env,
                "git",
                &[
                    "-c",
                    "core.quotepath=false",
                    "diff",
                    "--find-renames",
                    "--no-ext-diff",
                    "--no-color",
                    &format!("{base}...{sha}"),
                ],
                Some(root),
            )
            .await
        }
        .await;
        match result {
            Ok(patch) => {
                files = parse_diff(&patch);
                project = Some(p.id);
            }
            Err(e) => local_error = Some(e.to_string()),
        }
    }
    if project.is_none() {
        let mut page = 1;
        loop {
            let res = app
                .github
                .request(
                    Method::GET,
                    &format!("/repos/{repo}/pulls/{n}/files?per_page=100&page={page}"),
                    None,
                    true,
                )
                .await?;
            let batch = res.value.as_array().context("GitHub files response")?;
            let count = batch.len();
            files.extend(batch.iter().cloned());
            // A conditional 304 can omit Link, so the cached array length controls pagination.
            if count < 100 {
                break;
            }
            page += 1;
        }
    }
    if project.is_none() {
        let current = app
            .github
            .request(
                Method::GET,
                &format!("/repos/{repo}/pulls/{n}"),
                None,
                false,
            )
            .await?
            .value;
        ensure!(
            current["head"]["sha"] == d["headRefOid"] && current["base"]["sha"] == d["baseRefOid"],
            "PR changed while loading patches; refresh the item and retry"
        );
    }
    let v = json!({"files":files,"project_id":project,"head":d["headRefOid"],"base":d["baseRefOid"],"fallback":project.is_none(),"local_error":local_error});
    store(app, key, &v).await?;
    // File requests identify API fallback results by this project key.
    if project.is_none() {
        store(app, format!("diff:{repo}:{}:{}:api", d["baseRefOid"], d["headRefOid"]), &v).await?;
    }
    Ok(v)
}
/// The file list of a PR diff with line stats, without patches (`diff_file` returns one patch).
pub async fn diff_summary(app: &App, repo: &str, n: u64) -> Result<Value> {
    let mut v = diff(app, repo, n).await?;
    if let Some(files) = v["files"].as_array_mut() {
        for f in files {
            let patch = f.as_object_mut().and_then(|o| o.remove("patch"));
            let text = patch.as_ref().and_then(Value::as_str).unwrap_or("");
            if f.get("additions").is_none() {
                let (add, del) = line_stats(text);
                f["additions"] = json!(add);
                f["deletions"] = json!(del);
            }
            f["has_patch"] = json!(!text.is_empty());
            if let Some(o) = f.as_object_mut() {
                // GitHub API fields the UI does not use.
                for k in ["blob_url", "raw_url", "contents_url", "sha"] {
                    o.remove(k);
                }
            }
        }
    }
    Ok(v)
}

/// One file of a PR diff, read from the cached diff of the given base and head.
pub async fn diff_file(app: &App, repo: &str, n: u64, base: &str, head: &str, project: Option<&str>, path: &str) -> Result<Value> {
    validate(repo)?;
    let key = format!("diff:{repo}:{}:{}:{}", json!(base), json!(head), project.unwrap_or("api"));
    let v = match cached(app, key).await? {
        Some(v) => v,
        None => {
            let v = diff(app, repo, n).await?;
            ensure!(v["base"] == base && v["head"] == head, "the PR changed; reload its files");
            v
        }
    };
    v["files"]
        .as_array()
        .and_then(|files| files.iter().find(|f| f["filename"] == path))
        .cloned()
        .context("no such file in the diff")
}

fn line_stats(patch: &str) -> (usize, usize) {
    let mut out = (0, 0);
    for line in patch.lines() {
        if line.starts_with('+') {
            out.0 += 1;
        } else if line.starts_with('-') {
            out.1 += 1;
        }
    }
    out
}

pub(crate) fn parse_diff(patch: &str) -> Vec<Value> {
    let mut out = vec![];
    let mut file: Option<Value> = None;
    let mut in_hunk = false;
    for line in patch.lines() {
        if line.starts_with("diff --git ") {
            in_hunk = false;
            if let Some(f) = file.take() {
                out.push(f);
            }
            let path = line
                .rsplit_once(" \"b/")
                .map(|(_, p)| unquote(&format!("\"{p}")))
                .or_else(|| line.rsplit_once(" b/").map(|(_, p)| p.to_string()))
                .unwrap_or_default();
            file = Some(json!({"filename":path,"status":"modified","patch":""}));
        } else if let Some(f) = &mut file {
            in_hunk |= line.starts_with("@@");
            if in_hunk {
                if let Value::String(patch) = &mut f["patch"] {
                    patch.push_str(line);
                    patch.push('\n');
                }
                continue;
            }
            if let Some(path) = line.strip_prefix("+++ ") {
                // Git terminates unquoted headers containing spaces with a tab separator.
                let path = unquote(path.strip_suffix('\t').unwrap_or(path));
                if let Some(path) = path.strip_prefix("b/") {
                    f["filename"] = json!(path);
                } else if path == "/dev/null" {
                    f["status"] = json!("removed");
                }
            } else if let Some(path) = line.strip_prefix("--- ") {
                let path = unquote(path.strip_suffix('\t').unwrap_or(path));
                if f["filename"] == ""
                    && let Some(path) = path.strip_prefix("a/")
                {
                    f["filename"] = json!(path);
                }
            } else if let Some(path) = line.strip_prefix("rename from ") {
                f["previous_filename"] = json!(unquote(path));
            } else if let Some(path) = line.strip_prefix("rename to ") {
                f["filename"] = json!(unquote(path));
                f["status"] = json!("renamed");
            } else if line.starts_with("new file mode") {
                f["status"] = json!("added");
            } else if line.starts_with("deleted file mode") {
                f["status"] = json!("removed");
            }
            if let Some(path) = line
                .strip_prefix("Binary files a/")
                .and_then(|s| s.split_once(" and ").map(|(a, _)| a))
                && f["filename"] == ""
            {
                f["filename"] = json!(path);
            }
        }
    }
    if let Some(f) = file {
        out.push(f);
    }
    out
}
fn unquote(s: &str) -> String {
    serde_json::from_str::<String>(s).unwrap_or_else(|_| s.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn source_lines_cannot_spoof_diff_headers() {
        let files = parse_diff("diff --git a/real b/real\n--- a/real\n+++ b/real\n@@ -1,2 +1,4 @@\n--- a/fake\n+++ b/fake\n+++ /dev/null\n+added\n@@ -9 +11 @@\n+++ b/another\n");
        assert_eq!(files.len(), 1);
        assert_eq!(files[0]["filename"], "real");
        assert_eq!(files[0]["status"], "modified");
        assert!(files[0]["patch"].as_str().unwrap().contains("+++ /dev/null"));
        assert_eq!(line_stats(files[0]["patch"].as_str().unwrap()), (4, 1));
    }

    #[test]
    fn image_links_stay_external() {
        let html = "<a href=\"https://example/image?a=1&amp;b=2\"><img src=\"https://example/image?a=1&amp;b=2\"></a>";
        assert_eq!(rewrite_image_src(html, "https://example/image?a=1&b=2", "/img?url=test"),
            "<a href=\"https://example/image?a=1&amp;b=2\"><img src=\"/img?url=test\"></a>");
    }

    #[tokio::test]
    async fn image_retention_removes_old_bytes_and_sidecars_only() {
        let dir = std::env::temp_dir().join(format!("lp-images-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        for name in ["old", "old.mime", "fresh", "orphan.mime"] {
            let file = std::fs::File::create(dir.join(name)).unwrap();
            if name != "fresh" {
                file.set_times(std::fs::FileTimes::new().set_modified(std::time::SystemTime::now() - std::time::Duration::from_secs(22 * 86400))).unwrap();
            }
        }
        collect_images(&dir).await.unwrap();
        assert!(dir.join("fresh").exists());
        assert!(!dir.join("old").exists());
        assert!(!dir.join("old.mime").exists());
        assert!(!dir.join("orphan.mime").exists());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn diff_paths_and_empty_changes() {
        let files = parse_diff(
            "diff --git a/a b/a\n--- a/a\n+++ b/a\n@@ -1 +1 @@\n-old\n+new\ndiff --git a/old b/new\nsimilarity index 100%\nrename from old\nrename to new\ndiff --git a/gone b/gone\ndeleted file mode 100644\n--- a/gone\n+++ /dev/null\n@@ -1 +0,0 @@\n-gone\n",
        );
        assert_eq!(files.len(), 3);
        assert_eq!(files[0]["filename"], "a");
        assert!(files[0]["patch"].as_str().unwrap().contains("+new"));
        assert_eq!(files[1]["filename"], "new");
        assert_eq!(files[1]["previous_filename"], "old");
        assert_eq!(files[1]["patch"], "");
        assert_eq!(files[2]["status"], "removed");
        let binary = parse_diff(
            "diff --git a/new image.png b/new image.png\nnew file mode 100644\nBinary files /dev/null and b/new image.png differ\n",
        );
        assert_eq!(binary[0]["filename"], "new image.png");
        assert_eq!(binary[0]["status"], "added");
    }
}
