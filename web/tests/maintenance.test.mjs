import { test } from "node:test";
import assert from "node:assert/strict";
import { bannerText, refusalText, refusedFor } from "../src/lib/maintenance.ts";

const m = { phase: "replacing", reason: "Home updates to v2", target: "v2", since: 0, retry_after: 600 };

test("a write refused for maintenance is recognized and keeps the draft", () => {
  const refused = { status: 503, body: { error: "…", retryable: true, maintenance: m } };
  assert.deepEqual(refusedFor(refused), m);
  assert.match(refusalText(m), /Nothing was sent or changed, and your text is kept: try again in about 10 min/);
  // Other errors are not: a 503 without the marker, a 409, or a plain error.
  assert.equal(refusedFor({ status: 503, body: { error: "runner offline" } }), null);
  assert.equal(refusedFor({ status: 409, body: { retryable: true, maintenance: m } }), null);
  assert.equal(refusedFor(new Error("x")), null);
  assert.equal(refusedFor(null), null);
});

test("the banner follows the status across the restart's reconnect", () => {
  // Every tab gets the same pushed status: draining, then the checkpoint, then the build.
  assert.match(bannerText({ ...m, phase: "draining" }, true), /finishing the requests in progress/);
  assert.match(bannerText({ ...m, phase: "checkpointing" }, true), /saving its final backup/);
  assert.match(bannerText(m, true), /building the new version: changes are paused, and drafts are kept/);
  // The old Home stops: the connection drops and the last status stays, now as a restart.
  assert.match(bannerText(m, false), /restarting with the new version/);
  // The new Home's first status (refetched on reconnect) has no maintenance: the banner goes.
  assert.equal(bannerText(null, true), null);
  assert.equal(bannerText(undefined, false), null);
});
