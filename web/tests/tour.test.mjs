// Guardrails that keep the tour and the demo daemon in step with the app (static reads, no browser, no server):
// step anchors exist as data-tour attributes, step routes exist in the router, referenced scenarios exist, and
// every GET path the UI calls has a demo handler. Failures name the step or path.

import assert from "node:assert/strict";
import { readdirSync, readFileSync, statSync } from "node:fs";
import { join } from "node:path";
import { test } from "node:test";

const SRC = new URL("../src/", import.meta.url).pathname;

function files(dir, out = []) {
  for (const name of readdirSync(dir)) {
    const path = join(dir, name);
    if (statSync(path).isDirectory()) files(path, out);
    else if (/\.(ts|tsx)$/.test(name)) out.push(path);
  }
  return out;
}

const all = files(SRC);
const read = (p) => readFileSync(p, "utf8");
const app = all.filter((p) => !p.includes("/demo/") && !p.includes("/onboarding/chapters/") && !p.includes("/gen/"));
const chapters = all.filter((p) => p.includes("/onboarding/chapters/"));
const appText = app.map(read).join("\n");

/** Every `{ id: "...", ... }` step of the chapter files with its anchor and route (one step object per match). */
function steps() {
  const out = [];
  for (const file of chapters) {
    const text = read(file);
    for (const m of text.matchAll(/id: "([a-z]+\.[\w-]+)"([\s\S]*?)(?=\n\s*\{\s*\n\s*id: "|\n\s*\],?\s*\n\};|$)/g)) {
      out.push({ id: m[1], anchor: m[2].match(/anchor: "([^"]+)"/)?.[1], route: m[2].match(/route: "([^"]+)"/)?.[1] });
    }
  }
  return out;
}

test("every step anchor exists as a data-tour attribute", () => {
  const anchors = new Set([...appText.matchAll(/data-tour="([^"]+)"/g)].map((m) => m[1]));
  for (const m of appText.matchAll(/tour="([^"]+)"/g)) anchors.add(m[1]);
  for (const m of appText.matchAll(/data-tour=\{"([^"]+)"\}/g)) anchors.add(m[1]);
  const missing = steps().filter((s) => s.anchor && !anchors.has(s.anchor)).map((s) => `${s.id} → ${s.anchor}`);
  assert.deepEqual(missing, [], `steps whose anchor has no data-tour attribute:\n${missing.join("\n")}`);
});

test("every step route exists in the router", () => {
  const router = read(join(SRC, "app/router.tsx"));
  const paths = [...router.matchAll(/path: "([^"]+)"/g)].map((m) => new RegExp(`^${m[1].replace(/\$\w+/g, "[^/]+")}$`));
  const missing = steps().filter((s) => s.route && !paths.some((p) => p.test(s.route.split("?")[0]))).map((s) => `${s.id} → ${s.route}`);
  assert.deepEqual(missing, [], `steps whose route is not in app/router.tsx:\n${missing.join("\n")}`);
});

test("every scenario a chapter plays exists", () => {
  const scenarioIds = new Set(all.filter((p) => p.includes("/demo/scenarios/")).flatMap((p) => [...read(p).matchAll(/\bid: "([\w-]+)"/g)].map((m) => m[1])));
  const used = chapters.flatMap((p) => [...read(p).matchAll(/playScenario\([^,]+,\s*"([\w-]+)"/g)].map((m) => [p.split("/").pop(), m[1]]));
  const missing = used.filter(([, id]) => !scenarioIds.has(id)).map(([f, id]) => `${f} → ${id}`);
  assert.deepEqual(missing, [], `chapters playing unknown scenarios:\n${missing.join("\n")}`);
});

test("every GET path the UI calls has a demo handler", () => {
  const handlers = all.filter((p) => p.includes("/demo/routes/")).flatMap((p) => [...read(p).matchAll(/route\("GET", "([^"]+)"/g)].map((m) => m[1]));
  const patterns = handlers.map((h) => new RegExp(`^${h.replace(/[.+?^${}()|[\]\\]/g, "\\$&").replace(/:\w+/g, "[^/]+").replace(/\*\w+/g, ".+")}$`));
  const calls = new Set();
  for (const m of appText.matchAll(/api\.get(?:<[^(]*>)?\(\s*[`"](\/api\/[^`"?]*)/g)) calls.add(m[1]);
  // Template parts become one sample segment; repositories (owner/repo) become two.
  const sample = (path) => path.replace(/\$\{[^}]*repo[^}]*\}/gi, "owner/repo").replace(/\$\{[^}]*\}/g, "x");
  // The real daemon answers the onboarding record in demo mode too.
  const missing = [...new Set([...calls].map(sample))].filter((p) => p !== "/api/onboarding" && !patterns.some((r) => r.test(p.split("$")[0]))).sort();
  assert.deepEqual(missing, [], `UI GET paths without a demo handler (add one in src/demo/routes/, or notInDemo):\n${missing.join("\n")}`);
});
