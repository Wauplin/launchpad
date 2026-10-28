// Chapter 3: rate-limit /invoices (billing-api, Codex). A question with two answers, and an approval card for
// `cargo test` in Ask for approval mode. Variants: answer A or B; permission YOLO or Ask for approval; When done.

import { approve, ask, at, commit, edit, final, merged, say, scenario, think, tool, when, type Item } from "../player";

export const failingTest = `running 3 tests
test rate_limit::allows_ten_requests_per_minute ... ok
test rate_limit::rejects_the_eleventh_request ... FAILED
test rate_limit::resets_after_a_minute ... FAILED

failures:
---- rate_limit::rejects_the_eleventh_request stdout ----
thread 'rate_limit::rejects_the_eleventh_request' panicked at tests/rate_limit.rs:31:5:
assertion \`left == right\` failed
  left: 200
 right: 429

test result: FAILED. 1 passed; 2 failed; 0 ignored; finished in 0.41s`;

export const passingTest = `running 3 tests
test rate_limit::allows_ten_requests_per_minute ... ok
test rate_limit::rejects_the_eleventh_request ... ok
test rate_limit::resets_after_a_minute ... ok

test result: ok. 3 passed; 0 failed; 0 ignored; finished in 0.38s`;

const memoryDiff = `@@ -0,0 +1,28 @@
+use std::collections::HashMap;
+use std::sync::Mutex;
+use std::time::{Duration, Instant};
+
+/// Fixed-window counter per API key, kept in this instance's memory.
+pub struct RateLimiter {
+    limit: u32,
+    window: Duration,
+    counts: Mutex<HashMap<String, (Instant, u32)>>,
+}
+
+impl RateLimiter {
+    pub fn new(limit: u32, window: Duration) -> Self {
+        Self { limit, window, counts: Mutex::new(HashMap::new()) }
+    }
+
+    /// True when the request is allowed.
+    pub fn check(&self, key: &str) -> bool {
+        let mut counts = self.counts.lock().unwrap();
+        let now = Instant::now();
+        let entry = counts.entry(key.to_string()).or_insert((now, 0));
+        if now.duration_since(entry.0) >= self.window {
+            *entry = (now, 0);
+        }
+        entry.1 += 1;
+        entry.1 <= self.limit
+    }
+}
`;

const redisDiff = `@@ -0,0 +1,24 @@
+use redis::AsyncCommands;
+
+/// Fixed-window counter per API key, shared by every instance through Redis.
+pub struct RateLimiter {
+    client: redis::Client,
+    limit: u32,
+    window_secs: u64,
+}
+
+impl RateLimiter {
+    pub fn new(client: redis::Client, limit: u32, window_secs: u64) -> Self {
+        Self { client, limit, window_secs }
+    }
+
+    /// True when the request is allowed.
+    pub async fn check(&self, key: &str) -> redis::RedisResult<bool> {
+        let mut conn = self.client.get_multiplexed_async_connection().await?;
+        let bucket = format!("rate:invoices:{key}");
+        let count: u32 = conn.incr(&bucket, 1).await?;
+        if count == 1 {
+            let _: () = conn.expire(&bucket, self.window_secs as i64).await?;
+        }
+        Ok(count <= self.limit)
+    }
+}
`;

const routeDiff = `@@ -12,6 +12,12 @@ pub async fn list_invoices(
     State(app): State<App>,
     key: ApiKey,
 ) -> Result<Json<Vec<Invoice>>, ApiError> {
+    if !app.invoice_limiter.check(&key.0) {
+        return Err(ApiError::TooManyRequests {
+            retry_after_secs: 60,
+        });
+    }
+
     let invoices = app.db.invoices_for(&key.0).await?;
     Ok(Json(invoices))
 }
`;

const cargoDiff = `@@ -14,6 +14,7 @@ axum = "0.8"
 serde = { version = "1", features = ["derive"] }
 tokio = { version = "1", features = ["full"] }
+redis = { version = "0.27", features = ["tokio-comp"] }
 tracing = "0.1"
`;

const summary = (choice: "memory" | "redis") => `**\`GET /invoices\` is rate-limited to 10 requests per minute per API key.** The eleventh request in a minute gets \`429 Too Many Requests\` with \`Retry-After: 60\`.

### Changes
- \`src/limit.rs\`: a fixed-window \`RateLimiter\` ${choice === "memory" ? "kept in memory, one counter per instance" : "backed by Redis (`INCR` + `EXPIRE`), shared by every instance"}.
- \`src/routes/invoices.rs\`: checks the limiter before reading invoices.${choice === "redis" ? "\n- `Cargo.toml`: adds the `redis` crate." : ""}

### Checks
- \`cargo test rate_limit\`: **3 passed** (was 1 passed, 2 failed).

### Limitations
${choice === "memory" ? "- With several instances behind the load balancer, each one counts separately: the effective limit is 10 × the number of instances. Choose Redis later if that matters." : "- Needs `REDIS_URL` in every environment; the service refuses to start without it."}`;

const tail = (choice: "memory" | "redis"): Item[] => [
  when("permission:ask", approve("cargo test rate_limit")),
  at(1.0, tool("bash", "cargo test rate_limit", { exit: 0, seconds: 2.6, output: passingTest })),
  at(0.6, commit("Rate-limit GET /invoices to 10 requests per minute")),
  when("done:merge", merged("lp/rate-limit-invoices")),
  final(summary(choice)),
];

export default scenario({
  id: "rate-limit",
  folder: "billing-api",
  harness: "codex",
  title: "Rate-limit /invoices",
  timeline: [
    at(0.4, think("The tests in tests/rate_limit.rs expect a 429 after 10 requests per minute. Nothing limits the invoices route yet.")),
    at(0.8, tool("grep", "rg -n \"invoices\" src/routes", { seconds: 0.5, output: "src/routes/mod.rs:8:    .route(\"/invoices\", get(invoices::list_invoices))\nsrc/routes/invoices.rs:11:pub async fn list_invoices(" })),
    at(0.8, tool("bash", "cargo test rate_limit", { exit: 101, seconds: 2.6, output: failingTest })),
    at(0.6, say("Two tests fail as expected. Before I write the limiter, one design choice changes the code and the deployment.")),
    ask("Where should the request counter live?", {
      "In memory, per instance": [at(0.8, think("A Mutex<HashMap> per instance: no new dependency.")), at(0.6, edit("src/limit.rs", memoryDiff, "add")), at(0.6, edit("src/routes/invoices.rs", routeDiff)), ...tail("memory")],
      "Redis (shared)": [at(0.8, think("Redis INCR with an expiry gives one counter for every instance.")), at(0.6, edit("src/limit.rs", redisDiff, "add")), at(0.6, edit("src/routes/invoices.rs", routeDiff)), at(0.5, edit("Cargo.toml", cargoDiff)), ...tail("redis")],
    }, {
      header: "Counter",
      descriptions: {
        "In memory, per instance": "Simple, no new service. Each instance counts on its own.",
        "Redis (shared)": "Exact limit across instances. Needs REDIS_URL everywhere.",
      },
    }),
  ],
});
