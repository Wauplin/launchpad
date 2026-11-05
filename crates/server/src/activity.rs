//! Records the actions sessions with admin Launchpad CLI access run directly in the activity list
//! (`launchpad_automation::activity`, held by Home): the values before the change (the action's `before` route,
//! read as the owner before it runs) and the action's answer. The bot chat shows them as Changed lines with Undo.
use axum::body::{Body, to_bytes};
use axum::extract::Request;
use axum::http::header;
use axum::response::Response;
use launchpad_core::App;
use launchpad_core::actions::{self, Action, ActionAccess, ActionEffect};
use serde_json::{Value, json};
use tower::ServiceExt;

pub struct Pending {
    session_id: String,
    action: &'static Action,
    args: Vec<String>,
    body: Option<Value>,
    before: Option<Value>,
}

fn decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() + 1 && i + 3 <= b.len() {
            if let Some(v) = s.get(i + 1..i + 3).and_then(|h| u8::from_str_radix(h, 16).ok()) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// An admin action of a session token that changes something (not checks, not seen markers): its route
/// parameters, body and the values before it.
pub async fn prepare(app: &App, session_id: &str, req: Request) -> (Request, Option<Pending>) {
    let path = req.uri().path().to_string();
    let Some(a) = actions::find(req.method().as_str(), &path).filter(|a| a.access == ActionAccess::Admin && a.effect != ActionEffect::Checks && !matches!(a.name, "session.seen" | "session.unread")) else {
        return (req, None);
    };
    let args: Vec<String> = a.route.split('/').zip(path.split('/')).filter(|(r, _)| r.starts_with('{')).map(|(_, p)| decode(p)).collect();
    let (parts, body) = req.into_parts();
    let bytes = to_bytes(body, 1 << 20).await.unwrap_or_default();
    let value = serde_json::from_slice::<Value>(&bytes).ok();
    let req = Request::from_parts(parts, Body::from(bytes));
    let before = match a.before {
        Some(route) => match actions::path(&Action { route, ..*a }, &args.iter().map(String::as_str).collect::<Vec<_>>()) {
            Ok(p) => owner(app, "GET", &p, None).await,
            Err(_) => None,
        },
        None => None,
    };
    (req, Some(Pending { session_id: session_id.into(), action: a, args, body: value, before }))
}

/// After the action: a success goes to the activity list (on Home).
pub async fn finish(app: &App, p: Pending, res: Response) -> Response {
    if !res.status().is_success() {
        return res;
    }
    let (parts, body) = res.into_parts();
    let Ok(bytes) = to_bytes(body, 64 << 20).await else {
        return Response::from_parts(parts, Body::empty());
    };
    let result = serde_json::from_slice::<Value>(&bytes).ok();
    let record = json!({"session_id": p.session_id, "action": p.action.name, "args": p.args, "body": p.body, "before": p.before, "result": result});
    if owner(app, "POST", "/api/activity", Some(record)).await.is_none() {
        tracing::warn!("activity: could not record {} by session {}", p.action.name, p.session_id);
    }
    Response::from_parts(parts, Body::from(bytes))
}

/// A request through this daemon's router as the owner (a runner forwards Home's records to Home).
async fn owner(app: &App, method: &str, path: &str, body: Option<Value>) -> Option<Value> {
    let router = crate::ROUTER.get()?;
    let mut req = Request::builder()
        .method(method)
        .uri(path)
        .header(header::HOST, format!("127.0.0.1:{}", app.paths.port))
        .header(header::AUTHORIZATION, format!("Bearer {}", app.paths.token().ok()?))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.map(|b| b.to_string()).unwrap_or_default()))
        .ok()?;
    req.extensions_mut().insert(axum::extract::ConnectInfo(std::net::SocketAddr::from(([127, 0, 0, 1], 0))));
    let res = router.clone().oneshot(req).await.ok()?;
    if !res.status().is_success() {
        return None;
    }
    let bytes = to_bytes(res.into_body(), 64 << 20).await.ok()?;
    serde_json::from_slice(&bytes).ok()
}
