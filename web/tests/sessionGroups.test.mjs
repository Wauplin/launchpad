import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { byPickerOrder, sessionGroup } from "../src/lib/sessionStatus.ts";

// The same cases as Rust's `Session::group` (`cargo test shared_group_cases`).
const cases = JSON.parse(readFileSync(new URL("./session-groups/cases.json", import.meta.url), "utf8"));
const session = (c) => ({ status: c.status, open_question: c.open_question, status_since: 10, seen_at: c.unseen ? 5 : 20, sleep: c.sleep ? { until: 1, waits: "", note: "", schedule_id: "x" } : undefined, archived_done: false });

test("the web's grouping matches Home's on the shared cases", () => {
  assert.ok(cases.length >= 10);
  for (const c of cases) assert.equal(sessionGroup(session(c)), c.group, JSON.stringify(c));
});

test("Home's group wins over the local rule, and picker order follows the groups", () => {
  const done = session({ status: "waiting", open_question: false, unseen: false, sleep: false });
  assert.equal(sessionGroup({ ...done, group: "needs_you" }), "needs_you");
  const rows = [{ ...session({ status: "closed", unseen: false }), id: "a" }, { ...session({ status: "working", unseen: false }), id: "b" }, { ...session({ status: "error", unseen: true }), id: "c" }];
  assert.deepEqual(rows.sort(byPickerOrder).map((s) => s.id), ["c", "b", "a"]);
});
