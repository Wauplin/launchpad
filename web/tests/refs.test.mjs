// The UI builds the same short IDs and addresses as Home (`launchpad_core::refs`, `cargo test shared_cases`).
import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { describeRef, looksLikeRef, refId, refLink, refPath, refToken } from "../src/lib/refs.ts";

const cases = JSON.parse(readFileSync(new URL("./refs/cases.json", import.meta.url), "utf8"));

test("short IDs and addresses match Home's", () => {
  assert.ok(cases.length >= 10);
  for (const c of cases) {
    assert.equal(refId(c.ref), c.id);
    assert.equal(refPath(c.ref), c.path, c.id);
    assert.ok(looksLikeRef(c.id), c.id);
    assert.ok(looksLikeRef(`https://home.example.ts.net${c.path}`, "https://home.example.ts.net"), c.path);
  }
});

test("pasted text is a reference only when it is one link or ID of a Launchpad page", () => {
  const origin = "https://me.hf.space";
  assert.ok(looksLikeRef("https://other.ts.net/bots/b1?thread=t2&msg=s.4"));
  assert.ok(looksLikeRef(`${origin}/topics/t1`, origin));
  assert.ok(looksLikeRef("https://x.ts.net/?card=session:abc&ws=Work"));
  for (const t of ["hello lp:chat/b/t", "https://github.com/o/r/pull/3", "https://x.ts.net/settings/profiles", "https://x.ts.net/?filter=send", "lp:", "ftp://x/agents?session=a", "https://x.ts.net/projects"]) assert.ok(!looksLikeRef(t, origin), t);
});

test("links carry the workspace, tokens and words name the object", () => {
  const r = { kind: "chat", id: "b1", thread: "t9", session: "s", seq: 4, title: "Fix login (v2)" };
  assert.equal(refLink(r, "https://h", "ws 1"), "https://h/bots/b1?thread=t9&msg=s.4&ws=ws%201");
  assert.equal(refLink({ kind: "topic", id: "t" }, "https://h"), "https://h/topics/t");
  assert.equal(refToken(r), "@chat:Fix-login-v2#msg");
  assert.equal(describeRef(r), "Chat “Fix login (v2)”, one message");
  assert.equal(describeRef({ kind: "folio", id: "x", section: "W1" }), "Folio, section W1");
});
