//! Exercise the real HTTP guard, cookies and both socket routes with isolated state and no monitors.
use super::*;
use futures_util::StreamExt;
use launchpad_core as core;
use tokio_tungstenite::{connect_async, tungstenite::client::IntoClientRequest};

#[tokio::test]
async fn front_door_protocol() {
    let fixture = tests::Fixture::new();
    let bus = core::events::Bus::new();
    let config = core::config::ConfigHandle::load(&fixture.0, bus.clone()).unwrap();
    let mut settings = config.get();
    settings.server.public_origin = Some("https://launchpad.example".into());
    let mut variant = core::config::HarnessVariant {
        id: "test".into(),
        name: "Test".into(),
        harness: core::model::HarnessKind::new("codex"),
        env: Default::default(),
        model: None,
        effort: None,
        billing: Default::default(),
        budget_usd: None,
    };
    variant
        .env
        .insert("EXAMPLE_KEY".into(), "private-value".into());
    settings.variants.push(variant);
    config.save(settings).unwrap();
    let state_db = core::db::open_state(&fixture.0.state_db()).unwrap();
    let cache_db = core::db::open_cache(&fixture.0.cache_db()).unwrap();
    let env = core::shellenv::UserEnv::resolve().await;
    let app = core::App {
        paths: Arc::new(fixture.0.clone()),
        config: config.clone(),
        state_db: state_db.clone(),
        cache_db: cache_db.clone(),
        status: core::status::StatusHandle::new(bus.clone()),
        harnesses: core::harness::Registry::new(&env),
        github: core::github::GitHub::new(env.clone(), cache_db),
        env,
        bus,
        sessions: core::sessions::Sessions::new(),
        inbox: core::inbox::Inbox::new(),
        models: Default::default(),
        predraft_lock: Arc::new(tokio::sync::Mutex::new(())),
        worktree_lock: Arc::new(tokio::sync::Mutex::new(())),
        terminals: Default::default(),
    };
    let auth = Arc::new(
        Auth::new(&fixture.0, &app.config.get().server, state_db)
            .await
            .unwrap(),
    );
    let server = Server {
        app,
        auth: auth.clone(),
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        axum::serve(listener, crate::router(server)).await.unwrap();
    });
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let base = format!("http://{address}");
    let get = |path: &str| {
        client
            .get(format!("{base}{path}"))
            .header("Host", "launchpad.example")
    };
    let post = |path: &str| {
        client
            .post(format!("{base}{path}"))
            .header("Host", "launchpad.example")
            .header("Origin", "https://launchpad.example")
    };
    assert_eq!(
        get("/api/devices").send().await.unwrap().status(),
        StatusCode::UNAUTHORIZED
    );
    let page = get("/auth").send().await.unwrap();
    assert_eq!(page.headers()[header::REFERRER_POLICY], "no-referrer");
    assert!(!page.text().await.unwrap().contains("/auth?t="));
    let pair: Value = post("/api/devices/pair")
        .bearer_auth(&auth.token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let code = tests::code(pair);
    let response = post("/auth")
        .json(&json!({"code":code,"name":"Protocol browser"}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let cookie = response.headers()[header::SET_COOKIE]
        .to_str()
        .unwrap()
        .to_string();
    assert!(cookie.contains("; Secure"));
    assert!(cookie.contains("HttpOnly; SameSite=Strict"));
    assert!(!cookie.contains(&auth.token));
    let cookie = cookie.split(';').next().unwrap();
    assert_eq!(
        post("/auth")
            .json(&json!({"code":code,"name":"Replay"}))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
    let devices: Value = get("/api/devices")
        .header("Cookie", cookie)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(devices[0]["current"], true);
    let id = devices[0]["id"].as_str().unwrap();
    let response: Value = get("/api/config")
        .header("Cookie", cookie)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(response["config"]["variants"][0]["env"], json!({}));
    let host_config = config;
    let mut config = response["config"].clone();
    config["zoom"] = json!(3);
    let put = |body: &Value| {
        client
            .put(format!("{base}/api/config"))
            .header("Host", "launchpad.example")
            .header("Origin", "https://launchpad.example")
            .header("Cookie", cookie)
            .json(body)
    };
    assert_eq!(put(&config).send().await.unwrap().status(), StatusCode::OK);
    assert_eq!(
        host_config.get().variants[0].env["EXAMPLE_KEY"],
        "private-value"
    );
    config["commands"]["browser"] = json!("changed-command");
    assert_eq!(
        put(&config).send().await.unwrap().status(),
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        post("/api/devices/pair")
            .header("Cookie", cookie)
            .header("Origin", "https://foreign.example")
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::FORBIDDEN
    );

    let local = client
        .post(format!("{base}/api/devices/pair"))
        .header("Host", "127.0.0.1:7318")
        .header("X-Launchpad-Local-Pair", "1")
        .bearer_auth(&auth.token)
        .send()
        .await
        .unwrap();
    assert!(local.headers().contains_key(header::CONTENT_LENGTH)); // desktop's plain HTTP bootstrap
    assert_eq!(local.headers()[header::CACHE_CONTROL], "no-store");
    let local: Value = local.json().await.unwrap();
    assert!(
        local["url"]
            .as_str()
            .unwrap()
            .starts_with("http://127.0.0.1:7318/auth#")
    );
    let local = client
        .post(format!("{base}/auth"))
        .header("Host", "127.0.0.1:7318")
        .json(&json!({"code": tests::code(local), "name":"Desktop"}))
        .send()
        .await
        .unwrap();
    assert_eq!(local.status(), StatusCode::OK);
    assert!(
        !local.headers()[header::SET_COOKIE]
            .to_str()
            .unwrap()
            .contains("Secure")
    );

    // The combined stack must authenticate before replaying even a completed operation.
    // Seed a response so this check cannot launch a harness or send real input.
    let operation = uuid::Uuid::new_v4().to_string();
    assert!(matches!(
        core::operations::claim(&auth.db, &operation, "/api/sessions", b"{}")
            .await
            .unwrap(),
        core::operations::Claim::New
    ));
    core::operations::complete(&auth.db, &operation, 200, "{\"replayed\":true}".into())
        .await
        .unwrap();
    let replay = || {
        post("/api/sessions")
            .header("Idempotency-Key", &operation)
            .json(&json!({}))
    };
    assert_eq!(
        replay().send().await.unwrap().status(),
        StatusCode::UNAUTHORIZED
    );
    let response = replay().header("Cookie", cookie).send().await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["Idempotency-Status"], "completed");
    assert_eq!(response.text().await.unwrap(), "{\"replayed\":true}");
    assert_eq!(
        replay()
            .header("Cookie", cookie)
            .header("Origin", "https://foreign.example")
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::FORBIDDEN
    );

    let socket_request = |path: &str| {
        let mut req = format!("ws://{address}{path}")
            .into_client_request()
            .unwrap();
        req.headers_mut()
            .insert(header::HOST, "launchpad.example".parse().unwrap());
        req.headers_mut()
            .insert(header::ORIGIN, "https://launchpad.example".parse().unwrap());
        req.headers_mut()
            .insert(header::COOKIE, cookie.parse().unwrap());
        req
    };
    let (mut socket, _) = connect_async(socket_request("/ws")).await.unwrap();
    assert!(
        socket
            .next()
            .await
            .unwrap()
            .unwrap()
            .into_text()
            .unwrap()
            .contains("hello")
    );
    // A first connection has nothing to resume from.
    assert!(
        socket
            .next()
            .await
            .unwrap()
            .unwrap()
            .into_text()
            .unwrap()
            .contains("resync")
    );
    let mut foreign = socket_request("/ws");
    foreign
        .headers_mut()
        .insert(header::ORIGIN, "https://foreign.example".parse().unwrap());
    let error = connect_async(foreign).await.unwrap_err();
    assert!(
        matches!(error, tokio_tungstenite::tungstenite::Error::Http(r) if r.status() == StatusCode::FORBIDDEN)
    );
    assert_eq!(
        client
            .delete(format!("{base}/api/devices/{id}"))
            .header("Host", "launchpad.example")
            .header("Origin", "https://launchpad.example")
            .header("Cookie", cookie)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    assert_eq!(
        replay()
            .header("Cookie", cookie)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
    let closed = tokio::time::timeout(std::time::Duration::from_secs(2), socket.next())
        .await
        .unwrap();
    assert!(closed.is_none() || closed.unwrap().is_err());
    for path in ["/ws", "/api/terminals/missing/ws"] {
        let error = connect_async(socket_request(path)).await.unwrap_err();
        assert!(
            matches!(error, tokio_tungstenite::tungstenite::Error::Http(r) if r.status() == StatusCode::UNAUTHORIZED)
        );
    }
    task.abort();
    let _ = task.await;
}
