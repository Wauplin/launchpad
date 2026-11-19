import { test } from "node:test";
import assert from "node:assert/strict";
import { editRequest, edited, lineKeys, startEdit } from "../src/content/github/suggestionAction.ts";
import { checkedEdit } from "../src/background/validation.ts";

const block = "Use the helper:\n\n```suggestion\nlet x = parse(raw)?;\n```\n";
const line = (path: string, n: number, body: string, o: { start_line?: number | null; side?: string } = {}) => ({ part: `line:${path}:${n}:${o.side ?? "RIGHT"}`, path, line: n, side: o.side ?? "RIGHT", start_line: o.start_line ?? null, body });
const review = {
  id: "s1", kind: "review", body: "**Looks good**, two nits.", event: "APPROVE", merge: "squash", run_ci: false,
  lines: [line("src/a.rs", 12, block, { start_line: 10 }), line("src/a.rs", 12, "Second note on the same lines.", { start_line: 10 }), line("src/b.rs", 4, "Removed on purpose?", { side: "LEFT" })],
};

test("an untouched edit sends the review back unchanged: raw Markdown, places, event and follow-up", () => {
  const d = startEdit(review);
  assert.equal(edited(review, d), false);
  assert.deepEqual(editRequest(review, d), {
    id: "s1", body: review.body, event: "APPROVE", merge: "squash", run_ci: false,
    comments: [
      { path: "src/a.rs", line: 12, side: "RIGHT", start_line: 10, body: block },
      { path: "src/a.rs", line: 12, side: "RIGHT", start_line: 10, body: "Second note on the same lines." },
      { path: "src/b.rs", line: 4, side: "LEFT", start_line: null, body: "Removed on purpose?" },
    ],
  });
});

test("the summary and each line comment take the text typed; a left-out comment is removed", () => {
  const keys = lineKeys(review.lines);
  assert.deepEqual(keys, ["line:src/a.rs:12:RIGHT", "line:src/a.rs:12:RIGHT#2", "line:src/b.rs:4:LEFT"]);
  const d = startEdit(review);
  d.body = "Approving.";
  d.lines[keys[0]!] = block.replace("parse(raw)", "parse(&raw)");
  d.lines[keys[1]!] = null;
  assert.equal(edited(review, d), true);
  const r = editRequest(review, d);
  assert.equal(r.body, "Approving.");
  assert.deepEqual(r.comments?.map((c) => [c.path, c.line, c.start_line, c.side, c.body]), [
    ["src/a.rs", 12, 10, "RIGHT", "Use the helper:\n\n```suggestion\nlet x = parse(&raw)?;\n```\n"],
    ["src/b.rs", 4, null, "LEFT", "Removed on purpose?"],
  ]);
});

test("a draft follows its comment by key when another one leaves the list", () => {
  const d = startEdit(review);
  d.lines["line:src/b.rs:4:LEFT"] = "Edited.";
  const fewer = { ...review, lines: [review.lines[0]!, review.lines[2]!] };
  assert.deepEqual(editRequest(fewer, d).comments?.map((c) => c.body), [block, "Edited."]);
});

test("a comment edit sends no line comments, so Home keeps the rest as is", () => {
  const comment = { id: "c1", kind: "comment", body: "Thanks!", event: "COMMENT", merge: null, run_ci: false, lines: [] };
  const d = startEdit(comment);
  d.body = "Thanks, merging.";
  assert.deepEqual(editRequest(comment, d), { id: "c1", body: "Thanks, merging.", event: "COMMENT", merge: null, run_ci: false });
});

test("the worker rebuilds an edit field by field and refuses bad ones", () => {
  const sent = { ...editRequest(review, startEdit(review)), extra: 1 };
  const out = checkedEdit(sent);
  assert.equal("extra" in out, false);
  assert.equal(out.comments?.[0]?.body, block);
  assert.deepEqual(checkedEdit({ body: "x" }), { body: "x", event: "COMMENT", merge: null, run_ci: false });
  for (const bad of [
    { body: "  " }, { body: "x", event: "MERGE" }, { body: "x", merge: "ff" }, { body: "x", comments: "no" },
    { body: "x", comments: [{ path: "a", line: 0, side: "RIGHT", body: "y" }] },
    { body: "x", comments: [{ path: "a", line: 1, side: "UP", body: "y" }] },
    { body: "x", comments: [{ path: "a", line: 1, side: "RIGHT", start_line: -2, body: "y" }] },
    { body: "x", comments: [{ path: "a", line: 1, side: "RIGHT", body: "\n" }] },
  ]) assert.throws(() => checkedEdit(bad), JSON.stringify(bad));
});
