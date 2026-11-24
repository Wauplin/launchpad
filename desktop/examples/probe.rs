//! A development check without a desktop or keyring: links to a scratch dev Home through reverse pairing (approving
//! the code itself with the scratch Home's owner token), checks `/api/version`, then reads the event stream for a few
//! seconds and prints the menu's live groups. Never point it at a real Home.
//!
//! `LAUNCHPAD_PROBE_ORIGIN=http://127.0.0.1:7391 LAUNCHPAD_PROBE_OWNER_TOKEN=$(cat <scratch>/token) cargo run --example probe`

use std::time::{Duration, Instant};

use futures_util::StreamExt;
use launchpad_desktop::home::{self, Home};
use launchpad_desktop::links::Links;
use launchpad_desktop::model::{self, BusEvent, Model, Reload};
use launchpad_desktop::settings::{HomeKind, HomeLink};
use launchpad_desktop::{conn, sse};
use serde_json::json;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let origin = home::address_origin(&std::env::var("LAUNCHPAD_PROBE_ORIGIN")?).map_err(anyhow::Error::msg)?;
    let owner = std::env::var("LAUNCHPAD_PROBE_OWNER_TOKEN")?;
    let http = home::http();
    let start = home::post_unlinked(&http, &format!("{origin}/auth/reverse/start"), &json!({"name": "Desktop · probe", "kind": "desktop"}), None).await.map_err(|e| anyhow::anyhow!("{e:?}"))?;
    let (code, secret) = (start["code"].as_str().unwrap().to_string(), start["secret"].as_str().unwrap().to_string());
    println!("pairing code {code}");
    let approved = http.post(format!("{origin}/api/devices/reverse/approve")).bearer_auth(owner).json(&json!({"code": code})).send().await?;
    println!("approve: {} {}", approved.status(), approved.text().await?);
    let device = loop {
        tokio::time::sleep(Duration::from_millis(2500)).await;
        let r = home::post_unlinked(&http, &format!("{origin}/auth/reverse/poll"), &json!({"code": code, "secret": secret}), None).await.map_err(|e| anyhow::anyhow!("{e:?}"))?;
        if let Some(d) = r["device"].as_str() {
            break d.to_string();
        }
    };
    println!("device secret received ({} characters)", device.len());
    let link = HomeLink { kind: HomeKind::Address, origin: origin.clone(), space: None, device_name: "Desktop · probe".into(), hf: None };
    let home = Home::new(http, link, device, None);
    let version: serde_json::Value = home.get("/api/version").await?;
    let caps: Vec<String> = version["capabilities"].as_array().map(|a| a.iter().filter_map(|c| c.as_str().map(String::from)).collect()).unwrap_or_default();
    println!("version {version} → compatible: {:?}", conn::compatibility(version["api"].as_u64().map(|a| a as u32), version["min_client"].as_u64().unwrap_or(0) as u32, &caps).is_none());
    let mut model = Model::default();
    let response = home.events(None).await?;
    let mut body = response.bytes_stream();
    let mut parser = sse::Parser::default();
    let until = Instant::now() + Duration::from_secs(4);
    while let Ok(Some(Ok(chunk))) = tokio::time::timeout(until.saturating_duration_since(Instant::now()), body.next()).await {
        for frame in parser.push(&String::from_utf8_lossy(&chunk)) {
            let ev: BusEvent = serde_json::from_str(&frame.data)?;
            println!("event {} {}/{}", ev.bus, ev.topic, ev.kind);
            if matches!(ev.kind.as_str(), "replayed" | "resync") {
                model.workspaces = home.get("/api/workspaces").await?;
                model.folders = home.get("/api/projects").await?;
                model.bots = home.get("/api/bots").await?;
                let sessions: Vec<model::Session> = home.get("/api/sessions").await?;
                model.sessions = sessions.into_iter().map(|s| (s.id.clone(), s)).collect();
                model.today = home.get::<model::Today>("/api/today").await?.items;
                model.loaded = true;
            } else if let Some(Reload::Today) = model.apply(&ev) {
                model.today = home.get::<model::Today>("/api/today").await?.items;
            }
        }
    }
    let ws = model.workspace(None);
    println!("workspaces {:?} → following {ws:?}; {} sessions, {} bots, {} Today items", model.workspaces.iter().map(|w| &w.name).collect::<Vec<_>>(), model.sessions.len(), model.bots.len(), model.today.len());
    if let Some(ws) = ws {
        println!("{:#?}", model::status(&model, &ws, &Links::new(&origin, &ws)));
    }
    // The spotlight's requests (step 3): a link resolved, an ask sent, the launch spec checked, a composer hand-off.
    if let (Some(ws), Some(bot)) = (model.workspace(None), model.bots.first().cloned()) {
        let links = Links::new(&origin, &ws);
        println!("resolve lp:bot/{} → {:?}", bot.id, launchpad_desktop::actions::resolve(&home, &format!("lp:bot/{}", bot.id)).await.map_err(|e| e.message));
        println!("resolve lp:session/nope → {:?}", launchpad_desktop::actions::resolve(&home, "lp:session/nope").await.map_err(|e| e.message));
        let key = launchpad_desktop::spot::new_key();
        println!("ask {} → thread {:?}", bot.name(), launchpad_desktop::actions::ask(&home, &bot.id, "probe: hello", None, &key, &[]).await.map_err(|e| e.message));
        println!("same key again (idempotent) → thread {:?}", launchpad_desktop::actions::ask(&home, &bot.id, "probe: hello", None, &key, &[]).await.map_err(|e| e.message));
        // A pasted image (a 1×1 PNG) with the next ask: uploaded first, then a new thread of its own.
        let png = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 0x0d, b'I', b'H', b'D', b'R', 0, 0, 0, 1, 0, 0, 0, 1, 8, 6, 0, 0, 0, 0x1f, 0x15, 0xc4, 0x89, 0, 0, 0, 0x0d, b'I', b'D', b'A', b'T', 0x78, 0x9c, 0x63, 0, 1, 0, 0, 5, 0, 1, 0x0d, 0x0a, 0x2d, 0xb4, 0, 0, 0, 0, b'I', b'E', b'N', b'D', 0xae, 0x42, 0x60, 0x82];
        let image = launchpad_desktop::spot::Pasted { name: "Pasted image.png".into(), mime: "image/png".into(), bytes: png };
        println!("ask with an image → thread {:?} (a new one)", launchpad_desktop::actions::ask(&home, &bot.id, "probe: image", None, &launchpad_desktop::spot::new_key(), &[image]).await.map_err(|e| e.message));
        let sends: serde_json::Value = home.get(&format!("/api/bots/{}/sends", bot.id)).await?;
        println!("Home's receipts for the bot: {}", sends.as_array().map_or(0, |a| a.iter().filter(|s| s["id"] == key.as_str()).count()));
        let profile = launchpad_desktop::model::Profile { id: "probe".into(), name: "Probe".into(), harness: "claude".into() };
        let spec = launchpad_desktop::actions::launch_spec("probe: list the files", &profile, None, "keep", &ws);
        let preview = home.write(reqwest::Method::POST, "/api/launch/preview", &spec, None).await;
        println!("launch spec preview → {:?}", preview.map(|_| "accepted").map_err(|e| e.message));
        println!("handoff → {:?}", launchpad_desktop::actions::handoff(&home, &links, &ws, "probe: draft text").await.map_err(|e| e.message));
        let draft: serde_json::Value = home.get(&format!("/api/launch/draft?workspace={ws}")).await?;
        println!("draft now: text {:?}, open {}, revision {}", draft["spec"]["text"], draft["open"], draft["revision"]);
        println!("handoff again (appends, next revision) → {:?}", launchpad_desktop::actions::handoff(&home, &links, &ws, "second").await.map_err(|e| e.message));
        let draft: serde_json::Value = home.get(&format!("/api/launch/draft?workspace={ws}")).await?;
        println!("draft now: text {:?}, revision {}", draft["spec"]["text"], draft["revision"]);
    }
    home.delete("/api/devices/self").await?;
    println!("device deleted; next call: {:?}", home.get::<serde_json::Value>("/api/version").await.err().map(|e| e.failure));
    Ok(())
}
