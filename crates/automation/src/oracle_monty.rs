//! Monty on Home: the pool of `monty` worker processes (monty-pool) that runs every oracle, one fresh process per
//! evaluation with an empty environment, and the answers to its imports. Oracle code reaches nothing but Monty's
//! bundled standard library, `launchpad_oracle` (embedded here, served read-only) and `launchpad_host`, whose two
//! functions (`call`, `fail`) come back to Home as suspensions (`oracle_eval`).

use std::time::Duration;

use anyhow::{Context, Result};
use launchpad_core::App;
use monty_pool::{Checkout, Pool, PoolConfig, ReplConfig, ResumeValue, TurnEvent};
use monty_types::{DateTimeSource, ModuleFile, MontyObject, MontyUuid, NamedValues, OsPolicy, RandomSeed, RandomStart, ResourceLimits, SleepMode, TypeCheckingConfig, TypeCheckingFormat};
use tokio::sync::OnceCell;

use crate::oracles::MAX_TIMEOUT_MS;

/// The guest module and its interface (served to the type checker).
pub const GUEST: &str = include_str!("../oracle/launchpad_oracle/__init__.py");
pub const STUB: &str = include_str!("../oracle/launchpad_oracle/__init__.pyi");
/// Workers at once.
pub const WORKERS: usize = 16;
/// Memory of one evaluation.
pub const MAX_MEMORY: usize = 128 << 20;
/// Suspensions of one evaluation (tool calls and imports; Home also counts tool calls itself).
pub const MAX_SUSPENSIONS: usize = 250;
/// The module of the host functions.
pub const HOST_MODULE: &str = "launchpad_host";
const HOST_ID: u128 = 0x6c61_756e_6368_7061_645f_686f_7374_0001;

static POOL: OnceCell<std::result::Result<Pool, String>> = OnceCell::const_new();

/// The `monty` worker binary: `$LAUNCHPAD_MONTY`, else `monty` on PATH (Home's image installs it).
fn worker_binary(app: &App) -> Result<std::path::PathBuf> {
    crate::oracle_check::binary(app, "LAUNCHPAD_MONTY", "monty").context("the monty worker is not installed on Home: oracles run in it (Home's image has it; on a dev Home put monty on PATH or set LAUNCHPAD_MONTY)")
}

/// Home's pool of workers, started at the first evaluation.
pub async fn pool(app: &App) -> Result<&'static Pool> {
    let bin = worker_binary(app)?;
    let r = POOL
        .get_or_init(|| async move {
            let mut cfg = PoolConfig::subprocess(bin);
            cfg.min_processes = 1;
            cfg.max_processes = WORKERS;
            // A fresh process per evaluation: nothing survives from one to the next.
            cfg.max_checkouts_per_worker = Some(1);
            // A worker that stops answering is killed (the evaluation's own deadline is tighter).
            cfg.request_timeout = Some(Duration::from_millis(MAX_TIMEOUT_MS as u64 + 2_000));
            Box::pin(Pool::new(cfg)).await.map_err(|e| format!("could not start the monty workers: {e}"))
        })
        .await;
    r.as_ref().map_err(|e| anyhow::anyhow!("{e}"))
}

/// The session of one evaluation: its limits, a clock frozen at `now_ms` (UTC), `random` seeded from `seed`, and no
/// sleeping.
pub fn session(timeout: Duration, now_ms: i64, seed: &str, type_check: bool) -> ReplConfig {
    let limits = ResourceLimits { max_feed_duration: Some(timeout), max_memory: Some(MAX_MEMORY), max_suspensions: MAX_SUSPENSIONS, max_total_sleep: Some(Duration::ZERO), ..ResourceLimits::default() };
    let os_policy = OsPolicy {
        datetime: DateTimeSource::Fixed { unix_seconds: now_ms.div_euclid(1000), microsecond: (now_ms.rem_euclid(1000) * 1000) as u32 },
        // Sleeps are charged against a zero budget (`max_total_sleep`): any sleep ends the evaluation.
        sleep: SleepMode::default(),
        random_start: RandomStart::Seed(RandomSeed::Str(seed.to_string())),
        ..OsPolicy::default()
    };
    ReplConfig {
        script_name: "oracle.py".into(),
        limits: Some(limits),
        type_check,
        type_check_config: TypeCheckingConfig { format: TypeCheckingFormat::Concise, color: false },
        print_flush_interval: Some(Duration::ZERO),
        os_policy,
        ..ReplConfig::default()
    }
}

/// The answer to an OS call of the guest: the two modules Launchpad serves, the type checker's stubs, and nothing
/// else (files, the environment and the rest fail inside the sandbox).
pub fn os_answer(function_name: &str, module: Option<&str>) -> ResumeValue {
    match (function_name, module) {
        ("import", Some("launchpad_oracle")) => ResumeValue::ModuleFile(ModuleFile { path: "/launchpad/launchpad_oracle/__init__.py".into(), source: GUEST.into() }),
        ("import", Some(HOST_MODULE)) => ResumeValue::Return(MontyObject::module(
            HOST_MODULE,
            MontyUuid::from_u128(HOST_ID),
            [(MontyObject::string("call"), MontyObject::function("call", None)), (MontyObject::string("fail"), MontyObject::function("fail", None))],
        )),
        ("import", _) => ResumeValue::NotFound,
        _ => ResumeValue::NotHandled,
    }
}

/// The answer to the type checker's request for stubs: `launchpad_oracle`'s interface, None for any other module.
pub fn stubs_answer(args: &monty_types::CallArgs) -> ResumeValue {
    let asked: Vec<String> = args.arg(0).and_then(|a| a.items()).map(|items| items.iter().filter_map(|i| i.as_str().map(str::to_string)).collect()).unwrap_or_default();
    ResumeValue::Return(MontyObject::dict(asked.into_iter().map(|m| {
        let stub = if m == "launchpad_oracle" { MontyObject::string(STUB) } else { MontyObject::none() };
        (MontyObject::string(m), stub)
    })))
}

/// The module name of an import call.
pub fn module_of(args: &monty_types::CallArgs) -> Option<String> {
    args.arg(0).and_then(|a| a.as_str().map(str::to_string))
}

/// Type-checks a file against the interface of `launchpad_oracle` in a worker, without running it: the
/// diagnostics, or None when there are none. The worker is discarded at once either way.
pub async fn typecheck(app: &App, src: &str) -> Result<Option<String>> {
    let (app, src) = (app.clone(), src.to_string());
    tokio::spawn(async move { typecheck_task(&app, &src).await }).await.map_err(|_| anyhow::anyhow!("the type check stopped"))?
}

async fn typecheck_task(app: &App, src: &str) -> Result<Option<String>> {
    let pool = pool(app).await?;
    let _slot = crate::oracle_eval::slot().await;
    let mut co: Checkout = Box::pin(pool.checkout(&session(Duration::from_secs(20), launchpad_core::util::now_ms(), "typecheck", true))).await.map_err(|e| anyhow::anyhow!("{e}"))?;
    let mut on_print = monty_pool::on_print_sync(|_, _| {});
    let mut ev = Box::pin(co.feed(src.to_string(), NamedValues::new(), vec![], false, &mut on_print)).await;
    let r = loop {
        match ev {
            Err(monty_pool::PoolError::Typing(text)) => break Ok(Some(text)),
            // The file type-checked and started to run: stop it there.
            Ok(TurnEvent::OsCall { ref function_name, ref args, .. }) if function_name == "import_stubs" => {
                let answer = stubs_answer(args);
                ev = Box::pin(co.resume(answer, &mut on_print)).await;
            }
            Ok(_) | Err(monty_pool::PoolError::Runtime(_)) => break Ok(None),
            Err(e) => break Err(anyhow::anyhow!("{e}")),
        }
    };
    drop(co);
    r
}
