#!/usr/bin/env node
// Counts off-scale UI classes in web/src, so drift shows in review (CLAUDE.md, UI rules: Sizes and Dates).
//
//   node web/scripts/ui-check.mjs [--summary]        (or: cd web && pnpm ui-check)
//
// Prints each finding as file:line, rule and match, then a count per rule. Exits 1 when it finds anything that the
// allowlist below does not excuse. No dependencies. Generated types (src/gen) are skipped.
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const SRC = path.join(path.dirname(fileURLToPath(import.meta.url)), "..", "src");
const TEXT_SCALE = new Set(["11", "12", "13", "15", "18"]);
const ICON_SCALE = new Set(["12", "14", "16"]);

/** Each rule returns the offending matches of one line. */
const RULES = {
  // Tailwind's own text sizes carry their own line heights and break the 11/12/13/15/18 scale (the theme removes them).
  "text-default": (line) => [...line.matchAll(/(?<![\w-])(?:[\w-]+:)*text-(?:xs|sm|base|lg|[2-9]?xl)(?![\w-])/g)].map((m) => m[0]),
  "text-size": (line) => [...line.matchAll(/text-\[(\d+(?:\.\d+)?)px\]/g)].filter((m) => !TEXT_SCALE.has(m[1])).map((m) => m[0]),
  // Corners: rounded-control (6 px) and rounded-card (10 px); rounded, -sm, -xs, -full and -none for marks and pills.
  radius: (line) => [...line.matchAll(/(?<![\w-])rounded(?:-[trblse]{1,2})?-(?:\[[^\]]+\]|md|lg|[2-9]?xl)(?![\w-])/g)].map((m) => m[0]),
  // Page bodies use max-w-page (920 px), reading columns max-w-text (760 px).
  width: (line) => [...line.matchAll(/(?<![\w-])max-w-\[(\d+)px\]/g)].filter((m) => Number(m[1]) >= 700).map((m) => m[0]),
  gap: (line) => [...line.matchAll(/(?<![\w-])gap(?:-[xy])?-\[[^\]]+\]/g)].map((m) => m[0]),
  // Colors come from the theme tokens (styles.css), which have a dark value.
  hex: (line) => [...line.matchAll(/(?<=["'`[_(,])#(?:[0-9a-fA-F]{3,4}|[0-9a-fA-F]{6}|[0-9a-fA-F]{8})(?![\w])/g)].map((m) => m[0]),
  // Icon buttons have one size (IconButton, .iconbtn); a forced size-…! makes another one.
  "size-override": (line) => [...line.matchAll(/(?<![\w-])!?size-[\w.[\]]+!?/g)].filter((m) => m[0].startsWith("!") || m[0].endsWith("!")).map((m) => m[0]),
  // Icons: 12 inline, 14 in icon buttons, 16 in navigation.
  "icon-size": (line) => [...line.matchAll(/<Icon\b[^>]*?\bsize=\{(\d+)\}/g)].filter((m) => !ICON_SCALE.has(m[1])).map((m) => `size={${m[1]}}`),
  // Dates: ago(), age(), when() and describeCron() from lib/format.ts.
  date: (line) => [
    ...[...line.matchAll(/toLocale(?:Date|Time)String\(/g)].map((m) => m[0]),
    ...(/(?:Date|date|_at\b|time)\S*\.toLocaleString\(/.test(line) ? ["toLocaleString( on a date"] : []),
  ],
};

/** Justified exceptions: file (relative to src), rule, optional substring of the match or line, and why. Keep it short. */
const ALLOW = [
  { file: "lib/format.ts", rule: "date", why: "the date formatters themselves" },
  { file: "lib/serverTime.ts", rule: "date", why: "a clock in the server's time zone, which when() does not take" },
  { file: "features/terminals/TerminalView.tsx", rule: "hex", why: "xterm themes need literal colors" },
  { file: "components/Icon.tsx", rule: "hex", why: "sprite symbols: white marks and the Hugging Face logo colors" },
  { file: "lib/attachments.ts", rule: "hex", why: "canvas fill when flattening images" },
  { file: "stores/theme.ts", rule: "hex", why: "the browser's theme-color meta tag" },
  { file: "features/settings/AgentActivity.tsx", rule: "hex", match: "#4b5563", why: "fallback for a harness color (data from /api/harnesses)" },
  { file: "demo/", rule: "hex", why: "fixture data: harness, profile and label colors" },
  { file: "mobile/", rule: "icon-size", why: "phone tab bar and ‹ back icons are 18–20 px for touch" },
  { file: "features/settings/PhoneSettings.tsx", rule: "icon-size", why: "phone Settings list icons, 18 px like the More page" },
];

const allowed = (file, rule, match, line) => ALLOW.some((a) => a.rule === rule && (a.file.endsWith("/") ? file.startsWith(a.file) : file === a.file) && (!a.match || match.includes(a.match) || line.includes(a.match)));

function* files(dir) {
  for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
    const full = path.join(dir, entry.name);
    if (entry.isDirectory()) { if (entry.name !== "gen") yield* files(full); }
    else if (/\.(tsx?|mts)$/.test(entry.name)) yield full;
  }
}

const findings = [];
for (const full of files(SRC)) {
  const file = path.relative(SRC, full).split(path.sep).join("/");
  fs.readFileSync(full, "utf8").split("\n").forEach((line, i) => {
    for (const [rule, check] of Object.entries(RULES)) {
      for (const match of check(line)) if (!allowed(file, rule, match, line)) findings.push({ file, line: i + 1, rule, match });
    }
  });
}

const summary = process.argv.includes("--summary");
if (!summary) for (const f of findings) console.log(`src/${f.file}:${f.line}  ${f.rule}  ${f.match}`);
const counts = {};
for (const f of findings) counts[f.rule] = (counts[f.rule] ?? 0) + 1;
if (findings.length) {
  console.log(`\n${findings.length} finding${findings.length === 1 ? "" : "s"}: ${Object.entries(counts).map(([r, n]) => `${r} ${n}`).join(", ")}`);
  console.log("Scale: text 11/12/13/15/18 px, rounded-control/-card, max-w-page/-text, gap-* steps, theme colors, IconButton sizes, icons 12/14/16, ago/age/when.");
  process.exit(1);
}
console.log("ui-check: no off-scale classes.");
