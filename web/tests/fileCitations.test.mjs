import assert from "node:assert/strict";
import { test } from "node:test";
import MarkdownIt from "markdown-it";
import { citationPath, installFileCitations } from "../src/lib/fileCitations.ts";

const md = new MarkdownIt({ html: false, linkify: true });
installFileCitations(md);
const codeLinks = { citation: (raw) => {
  const path = citationPath(raw, "/repo");
  return path ? { path, line: null, href: `/viewer?path=${encodeURIComponent(path)}`, downloadHref: `/api/code/download?root=%2Frepo&path=${encodeURIComponent(path)}` } : null;
} };
const directive = ':codex-file-citation{path="/repo/output/My book.pdf" purpose="output"}';

test("renders native output citations inline and standalone, including ignored files and spaces", () => {
  const html = md.render(`Created ${directive}.\n\n${directive}`, { codeLinks });
  assert.equal((html.match(/data-path=/g) ?? []).length, 2);
  assert.equal((html.match(/ download /g) ?? []).length, 2);
  assert.ok(html.includes('data-path="output/My book.pdf"'));
  assert.ok(html.includes('>My book.pdf</a>'));
  assert.ok(html.includes('path=output%2FMy%20book.pdf'));
  assert.ok(!html.includes("codex-file-citation"));
});

test("source citations offer the code viewer without a download and support reordered attributes", () => {
  const html = md.render(':codex-file-citation{purpose="source" path="/repo/src/main.rs"}', { codeLinks });
  assert.ok(html.includes('data-path="src/main.rs"'));
  assert.ok(!html.includes("Download"));
});

test("outside paths and missing session context remain readable and inert", () => {
  for (const env of [{}, { codeLinks }]) {
    const html = md.render(':codex-file-citation{path="/other/secret.txt" purpose="output"}', env);
    assert.ok(html.includes("/other/secret.txt"));
    assert.ok(!html.includes("<a"));
    assert.ok(!html.includes("codex-file-citation"));
  }
});

test("does not interpret citations inside inline code, fences, or existing links", () => {
  for (const text of [`\`${directive}\``, `\`\`\`\n${directive}\n\`\`\``, `[${directive}](https://example.org)`]) {
    const html = md.render(text, { codeLinks });
    assert.ok(html.includes("codex-file-citation"));
    assert.ok(!html.includes("data-path"));
  }
});

test("malformed and incomplete streaming directives are preserved, escaping HTML and attribute injection", () => {
  for (const text of [':codex-file-citation{path="/repo/x.pdf"', ':codex-file-citation{purpose="output"}', ':codex-file-citation{path=""}']) {
    assert.ok(md.render(text, { codeLinks }).includes("codex-file-citation"));
  }
  const html = md.render(':codex-file-citation{path="/repo/<img onerror=alert(1)>.pdf" purpose="output"}', { codeLinks });
  assert.ok(!html.includes("<img"));
  assert.ok(html.includes("&lt;img"));
});

test("explicit paths handle local, Windows and SSH roots without escaping the session folder", () => {
  assert.equal(citationPath("/repo/output/a.pdf", "/repo"), "output/a.pdf");
  assert.equal(citationPath("./output/a.pdf", "/repo"), "output/a.pdf");
  assert.equal(citationPath("C:\\repo\\output\\a.pdf", "C:\\repo"), "output/a.pdf");
  assert.equal(citationPath("/repo/output/a.pdf", "ssh-host:/repo"), "output/a.pdf");
  assert.equal(citationPath("ssh-host:/repo/output/a.pdf", "ssh-host:/repo"), "output/a.pdf");
  assert.equal(citationPath("another-host:/repo/output/a.pdf", "ssh-host:/repo"), null);
  for (const raw of ["/repo-other/a.pdf", "../a.pdf", "/repo/../a.pdf", "a/../../a.pdf", "https://example.org", "C:\\other\\a.pdf", "a\0.pdf", "//other/file"]) {
    assert.equal(citationPath(raw, "/repo"), null, raw);
  }
});
