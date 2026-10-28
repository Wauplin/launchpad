// Demo routes: integrated terminals. Nothing runs: a shell replays a recording (git status, then the folder's tests)
// byte by byte, a harness TUI shows one recorded frame, and typed input only answers that commands do not run.
// Sockets follow the real protocol (`crates/server/src/terminals.rs`): a `reset` control with the size and owner,
// the retained output as binary frames, `ready`, then live output; `resize`, `claim` and `ack` come back.

import type { HarnessKind } from "@/gen/HarnessKind";
import type { TaskWorktree } from "@/gen/TaskWorktree";
import type { TerminalFolders } from "@/gen/TerminalFolders";
import type { TerminalInfo } from "@/gen/TerminalInfo";
import type { TerminalKind } from "@/gen/TerminalKind";
import { clock, DemoError, FakeSocket, onTerminalSocket, publish, route } from "../daemon";
import { world } from "../world";
import { ROOT } from "../world/ids";
import { HOME } from "../world/settings";

const MAX_OPEN = 12;
const encoder = new TextEncoder();

// ANSI helpers.
const ESC = "\x1b[";
const c = (code: string, text: string) => `${ESC}${code}m${text}${ESC}0m`;
const green = (t: string) => c("32", t);
const red = (t: string) => c("31", t);
const yellow = (t: string) => c("33", t);
const dim = (t: string) => c("2", t);
const bold = (t: string) => c("1", t);
const NOT_RUN = "Demo terminal: commands do not run";

/** One step of a recording: text typed at the prompt (one byte at a time) or output after a pause. */
type Step = { type: string } | { out: string; wait: number };

/** Live state of a demo terminal (the daemon's PTY and its 1 MB buffer). */
type Live = {
  /** The world record it belongs to (a world reset makes new records with the same IDs). */
  info: TerminalInfo;
  output: Uint8Array[];
  viewers: Set<FakeSocket>;
  rows: number;
  cols: number;
  /** Recording steps not played yet; playback starts when the terminal is first shown. */
  script: Step[];
  playing: boolean;
  timers: number[];
  /** The line typed since the last prompt. */
  line: string;
  prompt: string;
  tui: boolean;
};
const live = new Map<string, Live>();

// ---------------------------------------------------------------- recordings

const tilde = (path: string) => (path.startsWith(HOME) ? `~${path.slice(HOME.length)}` : path);
const shellPrompt = (t: TerminalInfo) => `${bold(green("demo@this-computer"))}:${bold(c("34", tilde(t.cwd.replace(/^[^/]*:/, ""))))}${t.branch ? c("36", ` (${t.branch})`) : ""}$ `;
const lines = (text: string) => text.split("\n").join("\r\n") + "\r\n";

const gitStatusDirty = (branch: string) => lines(`On branch ${branch}
Your branch is up to date with 'origin/${branch}'.

Changes not staged for commit:
  (use "git add <file>..." to update what will be committed)
  (use "git restore <file>..." to discard changes in working directory)
	${red("modified:   src/checkout/cart.ts")}
	${red("modified:   src/checkout/totals.ts")}

Untracked files:
  (use "git add <file>..." to include in what will be committed)
	${red("src/checkout/cart.test.ts")}

no changes added to commit (use "git add" and/or "git commit -a")`);

const gitStatusClean = (branch: string, remote: boolean) => lines(`On branch ${branch}${remote ? `\nYour branch is up to date with 'origin/${branch}'.` : ""}

nothing to commit, working tree clean`);

const pnpmTest = (): Step[] => [
  { out: lines(`\n> storefront@1.4.0 test ${ROOT}/storefront\n> vitest run\n`), wait: 500 },
  { out: lines(` ${c("46;30", " RUN ")} ${dim(`${ROOT}/storefront`)}\n`), wait: 700 },
  ...[
    ["src/lib/money.test.ts", 12, 9],
    ["src/catalog/search.test.ts", 8, 31],
    ["src/checkout/totals.test.ts", 9, 14],
    ["src/checkout/cart.test.ts", 7, 22],
    ["src/account/session.test.ts", 6, 18],
  ].map(([file, n, ms]) => ({ out: lines(` ${green("✓")} ${file} ${dim(`(${n} tests) ${ms}ms`)}`), wait: 260 + Number(ms) * 8 })),
  { out: lines(`\n ${dim("Test Files")}  ${bold(green("5 passed"))} ${dim("(5)")}\n      ${dim("Tests")}  ${bold(green("42 passed"))} ${dim("(42)")}\n   ${dim("Start at")}  ${new Date(clock.now()).toTimeString().slice(0, 8)}\n   ${dim("Duration")}  1.84s ${dim("(transform 412ms, setup 0ms, collect 1.21s, tests 94ms)")}\n`), wait: 600 },
];

const cargoTest: Step[] = [
  { out: lines(`   ${bold(green("Compiling"))} billing-api v0.9.2 (${ROOT}/billing-api)`), wait: 900 },
  { out: lines(`    ${bold(green("Finished"))} \`test\` profile [unoptimized + debuginfo] target(s) in 6.42s\n     ${bold(green("Running"))} unittests src/main.rs (target/debug/deps/billing_api-3f9c1e2a)\n\nrunning 18 tests`), wait: 1200 },
  ...["invoices::tests::creates_invoice", "invoices::tests::rejects_negative_amount", "limit::tests::allows_burst", "limit::tests::returns_429_after_limit", "tax::tests::rounds_half_even"].map((name) => ({ out: lines(`test ${name} ... ${green("ok")}`), wait: 140 })),
  { out: lines(`\ntest result: ${green("ok")}. 18 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.31s\n`), wait: 500 },
];

/** What a new shell in this folder replays: `git status`, then the folder's tests when it has some. */
function recording(t: TerminalInfo): Step[] {
  if (!t.branch) return [];
  const project = world.projects.find((p) => p.id === t.project_id);
  const steps: Step[] = [{ type: "git status" }];
  if (project?.name === "storefront") return [...steps, { out: gitStatusDirty(t.branch), wait: 250 }, { type: "pnpm test" }, ...pnpmTest()];
  steps.push({ out: gitStatusClean(t.branch, !!project?.gh_repo), wait: 250 });
  if (project?.name === "billing-api") steps.push({ type: "cargo test" }, ...cargoTest);
  return steps;
}

/** One recorded frame of the harness's own TUI ("Continue in TUI"), drawn on the alternate screen. */
function tuiFrame(t: TerminalInfo, harness: string, lastMessage: string | null, cols: number): string {
  const d = world.harnesses.find((h) => h.id === harness);
  const name = d?.name ?? harness;
  const hex = (d?.color ?? "#4b5563").replace("#", "");
  const accent = (text: string) => `${ESC}38;2;${parseInt(hex.slice(0, 2), 16)};${parseInt(hex.slice(2, 4), 16)};${parseInt(hex.slice(4, 6), 16)}m${text}${ESC}0m`;
  const w = Math.max(40, Math.min(cols - 2, 96));
  const box = (rows: string[]) => {
    const visible = (s: string) => s.replace(/\x1b\[[0-9;]*m/g, "").length;
    return [accent(`╭${"─".repeat(w - 2)}╮`), ...rows.map((r) => `${accent("│")} ${r}${" ".repeat(Math.max(0, w - 4 - visible(r)))} ${accent("│")}`), accent(`╰${"─".repeat(w - 2)}╯`)];
  };
  const wrap = (text: string, width: number) => {
    const out: string[] = [];
    for (const word of text.split(/\s+/)) {
      if (out.length && (out[out.length - 1]! + " " + word).length <= width) out[out.length - 1] += " " + word;
      else out.push(word);
    }
    return out;
  };
  const message = wrap(lastMessage ?? "Resumed the session. The last turn is finished; ask for a follow-up or exit to return to Launchpad.", w - 6);
  const screen = [
    ...box([`${accent("✻")} ${bold(name)} ${dim("· resumed session")}`, dim(`cwd: ${tilde(t.cwd.replace(/^[^/]*:/, ""))}`)]),
    "",
    ` ${dim(">")} ${dim(t.title)}`,
    "",
    ...message.map((l, i) => `${i ? "   " : ` ${accent("⏺")} `}${l}`),
    "",
    ...box([">"]),
    `  ${dim("? for shortcuts")}${" ".repeat(Math.max(1, w - 40))}${dim("Demo · recorded frame")}`,
  ];
  // Alternate screen, clear, frame, cursor in the input box.
  const inputRow = screen.length - 2;
  return `${ESC}?1049h${ESC}H${ESC}2J${screen.join("\r\n")}${ESC}${inputRow};5H`;
}

// ---------------------------------------------------------------- terminal state

function emit(id: string, text: string) {
  const l = live.get(id);
  if (!l) return;
  const bytes = encoder.encode(text);
  l.output.push(bytes);
  for (const s of l.viewers) s.deliver(bytes);
}

const info = (id: string) => world.terminals.find((t) => t.id === id);

function setBusy(id: string, busy: boolean) {
  const t = info(id);
  if (t && t.busy !== busy) { t.busy = busy; publish("terminals", "opened", t); }
}

/** Plays the remaining recording steps: typing one byte at a time, then each output chunk. */
function play(id: string) {
  const l = live.get(id);
  if (!l || l.playing || !l.script.length) return;
  l.playing = true;
  setBusy(id, true);
  let at = 600;
  for (const step of l.script) {
    if ("type" in step) {
      for (const ch of step.type) { at += 55 + Math.random() * 70; l.timers.push(window.setTimeout(() => emit(id, ch), at)); }
      at += 350;
      l.timers.push(window.setTimeout(() => emit(id, "\r\n"), at));
    } else {
      at += step.wait;
      const out = step.out;
      l.timers.push(window.setTimeout(() => emit(id, out), at));
      if (step === l.script.at(-1) || "type" in (l.script[l.script.indexOf(step) + 1] ?? { type: "" })) {
        at += 200;
        l.timers.push(window.setTimeout(() => emit(id, l.prompt), at));
      }
    }
  }
  l.timers.push(window.setTimeout(() => { l.script = []; l.playing = false; setBusy(id, false); }, at + 50));
}

/** Typed input: echoed and edited locally; Enter answers that commands do not run. */
function input(id: string, data: Uint8Array) {
  const l = live.get(id);
  const t = info(id);
  if (!l || !t?.alive || l.playing) return;
  const text = new TextDecoder().decode(data);
  if (text.startsWith("\x1b")) return; // Arrows and other escape sequences.
  for (const ch of text) {
    if (ch === "\r") {
      const typed = l.line.trim();
      l.line = "";
      if (l.tui) emit(id, `\r\n   ${typed ? yellow(NOT_RUN) : ""}${typed ? "\r\n" : ""}`);
      else emit(id, `\r\n${typed ? `${yellow(NOT_RUN)}\r\n` : ""}${l.prompt}`);
    } else if (ch === "\x7f" || ch === "\b") {
      if (l.line) { l.line = l.line.slice(0, -1); emit(id, "\b \b"); }
    } else if (ch === "\x03") {
      l.line = "";
      emit(id, l.tui ? "^C" : `^C\r\n${l.prompt}`);
    } else if (ch === "\x0c") {
      emit(id, l.tui ? "" : `${ESC}H${ESC}2J${l.prompt}${l.line}`);
    } else if (ch >= " ") {
      l.line += ch;
      emit(id, ch);
    }
  }
}

function serve(socket: FakeSocket, id: string) {
  const t = info(id);
  if (!t) { socket.close(); return; }
  const l = ensureLive(t);
  l.viewers.add(socket);
  // Every viewer owns the size: there is no shared PTY to letterbox.
  socket.deliver(JSON.stringify({ type: "reset", epoch: 1, offset: 0, rows: l.rows, cols: l.cols, owner: "self" }));
  for (const chunk of l.output) socket.deliver(chunk);
  socket.deliver(JSON.stringify({ type: "ready" }));
  if (!t.alive) socket.deliver(JSON.stringify({ type: "exit", code: t.exit_code }));
  socket.onSend = (data) => {
    if (typeof data === "string") {
      try {
        const msg = JSON.parse(data) as { type?: string; rows?: number; cols?: number };
        if (msg.type === "resize" && msg.rows && msg.cols && (msg.rows !== l.rows || msg.cols !== l.cols)) {
          l.rows = msg.rows; l.cols = msg.cols;
          for (const s of l.viewers) s.deliver(JSON.stringify({ type: "size", rows: l.rows, cols: l.cols, owner: "self" }));
        }
      } catch { /* Not a control message. */ }
      return;
    }
    const bytes = data instanceof Uint8Array ? data : ArrayBuffer.isView(data) ? new Uint8Array(data.buffer, data.byteOffset, data.byteLength) : data instanceof ArrayBuffer ? new Uint8Array(data) : null;
    if (bytes) input(id, bytes);
  };
  socket.onClosed = () => l.viewers.delete(socket);
  play(id);
}
onTerminalSocket(serve);

/** Ends the recording; the output stays for viewers. */
function halt(l: Live) {
  for (const timer of l.timers) clearTimeout(timer);
  l.timers = [];
  l.script = [];
  l.playing = false;
}

function stop(id: string) {
  const l = live.get(id);
  if (!l) return;
  halt(l);
  for (const s of l.viewers) s.close();
  live.delete(id);
}

function remove(id: string) {
  stop(id);
  world.terminals = world.terminals.filter((t) => t.id !== id);
  publish("terminals", "removed", { id });
}

/** Creates the live state of a terminal from the world (the initial terminals have none until first used). */
function ensureLive(t: TerminalInfo, opts: { harness?: string | null; lastMessage?: string | null; text?: string } = {}): Live {
  const existing = live.get(t.id);
  if (existing?.info === t) return existing;
  if (existing) stop(t.id);
  const l: Live = { info: t, output: [], viewers: new Set(), rows: 24, cols: 100, script: [], playing: false, timers: [], line: "", prompt: shellPrompt(t), tui: t.kind === "tui" };
  live.set(t.id, l);
  if (t.kind === "tui") emit(t.id, tuiFrame(t, opts.harness ?? world.sessions.find((s) => s.id === t.session_id)?.harness ?? "claude", opts.lastMessage ?? world.sessions.find((s) => s.id === t.session_id)?.last_message ?? null, l.cols));
  else if (opts.text !== undefined) emit(t.id, opts.text);
  else { emit(t.id, l.prompt); l.script = recording(t); }
  return l;
}

// ---------------------------------------------------------------- folders

type Place = { project_id: string | null; folder: string; place: string | null; branch: string | null };
const worktrees = () => ((world as unknown as { worktrees?: TaskWorktree[] }).worktrees ?? []).filter((w) => Array.isArray(w.sessions));
const lastName = (path: string) => path.replace(/[\\/]+$/, "").split(/[\\/:]/).pop() || path;

/** `describe` in the daemon: the folder name, the place chip (checkout or worktree) and the branch. */
function describe(cwd: string): Place {
  const project = world.projects.find((p) => p.path === cwd);
  if (project) return { project_id: project.id, folder: project.name, place: project.is_temp ? null : "checkout", branch: project.is_temp ? null : "main" };
  const record = worktrees().find((w) => w.root === cwd || cwd.startsWith(`${w.root}/`));
  const layout = cwd.match(/\/\.worktrees\/([^/]+)\/([^/]+)/);
  const owner = world.projects.find((p) => p.id === (record?.project_id ?? layout?.[1]));
  if (owner) return { project_id: owner.id, folder: owner.name, place: lastName(record?.root ?? cwd), branch: record?.branch ?? `lp/${layout?.[2] ?? lastName(cwd)}` };
  const inside = world.projects.filter((p) => cwd.startsWith(`${p.path}/`)).sort((a, b) => b.path.length - a.path.length)[0];
  if (inside) return { project_id: inside.id, folder: inside.name, place: null, branch: inside.is_temp ? null : "main" };
  return { project_id: null, folder: lastName(cwd), place: null, branch: null };
}

export type DemoTerminalOptions = {
  kind: TerminalKind;
  cwd: string;
  session_id?: string | null;
  /** Tab title; defaults to the folder and branch (shells) or the session title (TUIs). */
  title?: string;
  /** TUIs: the harness whose frame is drawn (defaults to the session's). */
  harness?: HarnessKind | string | null;
  /** TUIs: the message shown in the frame (defaults to the session's last message). */
  lastMessage?: string | null;
  /** Setup terminals (sign-in, install): no project, shown in every workspace. */
  setup?: boolean;
  /** Commands and setup terminals: the command line shown before the demo notice. */
  command?: string;
};

/** Opens a demo terminal, publishes `terminals/opened` and returns it like `POST /api/terminals` does. */
export function openDemoTerminal(o: DemoTerminalOptions): TerminalInfo {
  if (world.terminals.length >= MAX_OPEN) throw new DemoError(400, `At most ${MAX_OPEN} terminals can be open. Close one first.`);
  const where = o.setup ? { project_id: null, folder: lastName(o.cwd), place: null, branch: null } : describe(o.cwd);
  const session = o.session_id ? world.sessions.find((s) => s.id === o.session_id) : undefined;
  const title = o.title ?? (o.kind === "tui" ? session?.title || `${o.harness ?? session?.harness ?? "claude"} session` : where.branch ? `${where.folder} · ${where.branch}` : where.folder);
  const t: TerminalInfo = {
    id: `demo-term-${Math.random().toString(36).slice(2, 10)}`, kind: o.kind, title, cwd: o.cwd, project_id: where.project_id, session_id: o.session_id ?? null,
    alive: true, exit_code: null, created_at: clock.now(), folder: where.folder, place: where.place, branch: where.branch, setup: !!o.setup, busy: o.kind === "shell" ? false : null, notice: null,
  };
  world.terminals = [...world.terminals, t];
  const command = o.command ? `${dim("$")} ${o.command}\r\n${yellow(`${NOT_RUN}.`)}\r\n${dim("Close this tab when you are done.")}\r\n` : undefined;
  ensureLive(t, { harness: o.harness, lastMessage: o.lastMessage, text: command });
  publish("terminals", "opened", t);
  return t;
}

/** Removes a terminal (Re-attach of a TUI, Close, Remove). */
export function removeDemoTerminal(id: string) {
  if (info(id)) remove(id);
}

// ---------------------------------------------------------------- routes

route("GET", "/api/terminals", () => world.terminals.map((t) => ({ ...t, busy: t.alive ? (t.kind === "shell" ? !!live.get(t.id)?.playing : null) : null })));

route("POST", "/api/terminals", ({ body }) => {
  const b = (body ?? {}) as { cwd?: string; command?: string | null; language?: string | null };
  const cwd = b.cwd?.trim();
  if (!cwd) throw new DemoError(400, "Choose a folder");
  const known = cwd === HOME || world.projects.some((p) => cwd === p.path || cwd.startsWith(`${p.path}/`) || cwd.startsWith(`${ROOT}/.worktrees/${p.id}/`)) || world.sessions.some((s) => s.cwd === cwd);
  if (!known) throw new DemoError(400, "Shells open only in a registered folder, one of its worktrees or a session folder.");
  if (b.command) return openDemoTerminal({ kind: "command", cwd, command: b.command });
  // An idle shell for the same folder is reused, like the daemon does.
  const idle = world.terminals.find((t) => t.kind === "shell" && t.alive && t.cwd === cwd && !live.get(t.id)?.playing);
  if (idle) return { ...idle, notice: `Showing the open idle shell in ${idle.folder}.` };
  return openDemoTerminal({ kind: "shell", cwd });
});

route("POST", "/api/profiles/:id/setup", ({ params, body }) => {
  const p = world.config.profiles.find((x) => x.id === params.id);
  if (!p) throw new DemoError(400, `unknown profile ${params.id}`);
  const b = (body ?? {}) as { host?: string | null; install?: boolean };
  const commands: Record<string, [string, string]> = {
    claude: ["claude auth login", "curl -fsSL https://claude.ai/install.sh | bash"],
    codex: ["codex login --device-auth", "npm install -g @openai/codex"],
    opencode: ["opencode auth login", "curl -fsSL https://opencode.ai/install | bash"],
  };
  const [signIn, install] = commands[p.harness] ?? ["", ""];
  const vars = world.harnesses.find((h) => h.id === p.harness)?.account_env ?? [];
  const prefix = vars.flatMap((v) => (p.env[v] ? [`${v}=${p.env[v]}`] : [])).join(" ");
  const command = b.install ? install : `${prefix ? `${prefix} ` : ""}${signIn}`;
  return openDemoTerminal({ kind: "command", cwd: b.host ? `${b.host}:${HOME}` : HOME, setup: true, title: `${b.install ? "Install" : "Sign in"} · ${p.name}`, command });
});

route("POST", "/api/terminals/close-all", () => {
  for (const t of [...world.terminals]) remove(t.id);
  return { ok: true };
});

route("GET", "/api/terminals/folders", ({ query }): TerminalFolders => {
  const cwd = query.get("cwd") ?? "";
  const where = describe(cwd);
  const project = world.projects.find((p) => p.id === where.project_id);
  if (!project || project.is_temp) {
    return { project_id: project?.id ?? null, project_name: project?.name ?? null, default: cwd, current: { path: cwd, label: lastName(cwd), branch: where.branch, available: true, reason: null }, checkout: null, worktrees: [], notice: null };
  }
  const isCheckout = cwd === project.path;
  const records = worktrees().filter((w) => w.project_id === project.id);
  const record = records.find((w) => w.root === cwd || cwd.startsWith(`${w.root}/`));
  const merged = record?.merged ? `Merged into ${record.merged.into}` : null;
  const gone = record && record.state !== "active" ? (record.state === "removed" ? "Removed by cleanup; Resume the agent to restore it" : record.state === "discarded" ? "Discarded" : "Checked out in the checkout") : null;
  const current = isCheckout ? null : { path: cwd, label: lastName(record?.root ?? cwd), branch: where.branch, available: !gone, reason: gone ?? merged };
  const others = records.filter((w) => w !== record && w.state === "active").map((w) => ({ path: w.root, label: lastName(w.root), branch: w.branch, available: true, reason: null }));
  const usable = !isCheckout && !gone && !merged;
  return {
    project_id: project.id, project_name: project.name, current, checkout: { path: project.path, label: "Checkout", branch: "main", available: true, reason: null }, worktrees: others,
    default: usable ? cwd : project.path,
    notice: isCheckout || usable ? null : `This worktree is ${gone ? "gone" : `merged into ${record?.merged?.into ?? "main"}`}: Terminal opens the checkout.`,
  };
});

route("POST", "/api/terminals/:id/restart", ({ params }) => {
  const t = info(params.id);
  if (!t) throw new DemoError(400, "no such terminal");
  remove(t.id);
  return openDemoTerminal({ kind: t.kind, cwd: t.cwd, session_id: t.session_id, setup: t.setup, title: t.setup ? t.title : undefined });
});

route("DELETE", "/api/terminals/:id", ({ params }) => { removeDemoTerminal(params.id); return { ok: true }; });

route("POST", "/api/terminals/:id/close", ({ params }) => {
  const t = info(params.id);
  if (!t) throw new DemoError(400, "no such terminal");
  // Stopping an idle shell ends it successfully, which removes its tab; other kinds keep their final output.
  if (t.kind === "shell") { remove(t.id); return { ok: true }; }
  const l = live.get(t.id);
  t.alive = false;
  t.exit_code = 130;
  if (l) {
    halt(l);
    for (const s of l.viewers) s.deliver(JSON.stringify({ type: "exit", code: 130 }));
  }
  publish("terminals", "exited", t);
  return { ok: true };
});
