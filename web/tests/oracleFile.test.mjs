import assert from "node:assert/strict";
import { test } from "node:test";
import { paramValue, signatureOf, toolsOf, withTools } from "../src/lib/oracleFile.ts";

const HEAD = `# /// oracle
# title = "CI red"
# description = "Is CI red?"
# min_interval = "2m"
# ///
from launchpad_oracle import tools, verdict
`;

test("the Tools switches edit the header's tools line", () => {
  assert.deepEqual(toolsOf(HEAD), []);
  const one = withTools(HEAD, ["github"]);
  assert.match(one, /# description = "Is CI red\?"\n# tools = \["github"\]\n# min_interval/);
  assert.deepEqual(toolsOf(one), ["github"]);
  assert.deepEqual(toolsOf(withTools(one, ["github", "http"])), ["http", "github"]);
  assert.equal(withTools(one, []), HEAD);
  assert.equal(toolsOf("# /// script\n# ///\n"), null);
  assert.equal(withTools("print(1)\n", ["http"]), null);
});

test("the signature of check gives Test its fields", () => {
  const sig = signatureOf(`def check(cursor: str | None, repo: str, branch: str = "main, dev", limit: int = 20,\n          hours: float = 2, drafts: bool = False, label: str | None = None, now: datetime = None):\n    pass`);
  assert.equal(sig.cursor, true);
  assert.equal(sig.now, true);
  assert.deepEqual(sig.params.map((p) => [p.name, p.kind, p.optional, p.required, p.default]), [
    ["repo", "str", false, true, null], ["branch", "str", false, false, "main, dev"], ["limit", "int", false, false, 20],
    ["hours", "float", false, false, 2], ["drafts", "bool", false, false, false], ["label", "str", true, false, null],
  ]);
  assert.equal(signatureOf("def other():\n  pass"), null);
  const [repo, , limit] = sig.params;
  assert.equal(paramValue(limit, ""), undefined);
  assert.equal(paramValue(limit, "50"), 50);
  assert.throws(() => paramValue(limit, "abc"));
  assert.equal(paramValue(repo, "acme/app"), "acme/app");
});
