import assert from "node:assert/strict";
import { test } from "node:test";
import MarkdownIt from "markdown-it";
import { commitCandidates, installCommitLinks } from "../src/lib/commitLinks.ts";

const md = new MarkdownIt({ html: false, linkify: true });
installCommitLinks(md);
const sha = "b3c3d2fa4b73".padEnd(40, "0");
const commits = { b3c3d2f: { sha, href: `/projects/repo/code?rev=${sha}&session=run` } };

test("finds short/full mixed-case hashes in prose and inline code, excluding identifiers and code blocks", () => {
  assert.deepEqual(commitCandidates(md.parse(`Committed as \`b3c3d2f\`. Also B3C3D2F and ${sha}.
foo_b3c3d2f b3c3d2f.txt /b3c3d2f abcdef012345678901234567890123456789012345678
\`\`\`sh
abcdef0
\`\`\`

    1234567
`, {})), ["B3C3D2F", "b3c3d2f", sha].sort());
});

test("only confirmed hashes become accessible links, preserving inline code and escaping text", () => {
  const html = md.render("Committed as `b3c3d2f`; unknown `abcdef0`. <script> b3c3d2f", { commits });
  assert.equal((html.match(/data-commit=/g) ?? []).length, 2);
  assert.ok(html.includes('<code><a class="lp-path"'));
  assert.ok(html.includes(`<code>abcdef0</code>`));
  assert.ok(html.includes("&lt;script&gt;"));
  assert.ok(html.includes(`href="/projects/repo/code?rev=${sha}&amp;session=run"`));
  assert.ok(!md.render("b3c3d2f", {}).includes("data-commit"));
});

test("existing links, images and code blocks are never relinked", () => {
  const text = "[b3c3d2f](https://example.org/b3c3d2f) ![b3c3d2f](b3c3d2f.png) [\`b3c3d2f\`](https://example.org)\n\n```\nb3c3d2f\n```";
  assert.deepEqual(commitCandidates(md.parse(text, {})), []);
  assert.ok(!md.render(text, { commits }).includes("data-commit"));
});

test("preserves the existing inline file renderer when no verified commit matches", () => {
  const files = new MarkdownIt();
  const original = files.renderer.rules.code_inline;
  files.renderer.rules.code_inline = (tokens, idx, options, env, self) => tokens[idx].content === "src/main.rs:12"
    ? '<code data-path="src/main.rs" data-line="12">src/main.rs:12</code>'
    : original(tokens, idx, options, env, self);
  installCommitLinks(files);
  const html = files.render("See `src/main.rs:12`; committed as `b3c3d2f`.", { commits });
  assert.ok(html.includes('data-path="src/main.rs" data-line="12"'));
  assert.ok(html.includes('data-commit='));
});
