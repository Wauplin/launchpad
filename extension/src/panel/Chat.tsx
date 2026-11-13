// P1 Page chat: a chat with a bot of the workspace (its Ask target by default) about the current tab. The tab is a
// chip; on Send the panel reads the page once (readable text, URL, selection) and attaches it as `page.md`, so the agent
// never touches the browser. Suggestions only fill the box. The conversation is one of the bot's threads (the same as
// its chat in Launchpad with `?thread=`), chosen in the picker (main by default), shown live from the event stream;
// New conversation starts a new thread at the next Send, next to the bot's others.

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { BotThread } from "@/gen/BotThread";
import type { GitHubRef } from "@/gen/GitHubRef";
import type { Session } from "@/gen/Session";
import type { SessionEvent } from "@/gen/SessionEvent";
import { isRunning } from "@/lib/sessionStatus";
import { pageOf } from "../content/github/page";
import { Favicon, Icon } from "../icons";
import { QUOTE_TOPIC, request, subscribe, type BotBrief, type ConnectionState, type Quote } from "../shared/messages";
import { askSite, isWebPage, selectionOf, snapshot, type ActiveTab } from "./tab";
import { chatRows, duration, type ChatRow } from "./thread";

const message = (e: unknown) => (e instanceof Error ? e.message : String(e));
const hostOf = (url: string) => { try { return new URL(url).hostname.replace(/^www\./, ""); } catch { return url; } };
const BOT_KEY = (ws: string) => `launchpad-extension-chat-bot:${ws}`;

/** The bots of the workspace and the one the chat talks to (the last one picked here, else the Ask target). */
export function useBots(state: ConnectionState) {
  const [list, setList] = useState<{ bots: BotBrief[]; ask: string | null } | null>(null);
  const [error, setError] = useState<string | null>(null);
  const ws = state.workspace;
  useEffect(() => {
    if (!ws || state.phase !== "online") return;
    let alive = true;
    const load = () => void request("bots").then((l) => { if (alive) { setList(l); setError(null); } }).catch((e) => alive && setError(message(e)));
    load();
    const off = subscribe(["bots"], { event: load, resync: load });
    return () => { alive = false; off(); };
  }, [ws, state.phase]);
  return { list, error };
}

const threadOf = (s: Session) => s.launch_spec.bot?.thread || "main";

/** The threads of a bot (main first, then the newest), kept live. */
function useThreads(bot: string | null) {
  const [threads, setThreads] = useState<BotThread[]>([]);
  useEffect(() => {
    setThreads([]);
    if (!bot) return;
    let alive = true;
    const load = () => void request("bot.threads", { bot }).then((t) => alive && setThreads(t)).catch(() => undefined);
    load();
    const off = subscribe(["bots"], { event: load, resync: load });
    return () => { alive = false; off(); };
  }, [bot]);
  return threads;
}

/** One thread of a bot: its sessions and the events of the current (last) one, kept live. `all`: every open
 * session of the bot (the picker's states). */
function useThread(bot: string | null, threadId: string) {
  const [all, setSessions] = useState<Session[]>([]);
  const sessions = useMemo(() => all.filter((s) => threadOf(s) === threadId), [all, threadId]);
  const [events, setEvents] = useState<SessionEvent[]>([]);
  const [deltas, setDeltas] = useState<Record<string, string>>({});
  const [error, setError] = useState<string | null>(null);
  const current = sessions.at(-1) ?? null;
  const currentId = current?.id ?? null;

  const loadSessions = useCallback(async () => {
    if (!bot) return;
    try { setSessions(await request("sessions", { bot })); setError(null); } catch (e) { setError(message(e)); }
  }, [bot]);
  useEffect(() => { setSessions([]); setEvents([]); void loadSessions(); }, [loadSessions]);

  // Session records: reload the thread when one of the bot's sessions changed (status, a new session).
  useEffect(() => {
    if (!bot) return;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const later = () => { clearTimeout(timer); timer = setTimeout(() => void loadSessions(), 300); };
    const off = subscribe(["sessions"], {
      event: (e) => {
        const s = e.payload as Partial<Session> | null;
        if (e.type === "changed" || s?.launch_spec?.bot?.id === bot) later();
      },
      resync: later,
    });
    return () => { clearTimeout(timer); off(); };
  }, [bot, loadSessions]);

  // The current session's events: the newest page, then each new event from the stream.
  useEffect(() => {
    if (!currentId) { setEvents([]); return; }
    let alive = true;
    let last = 0;
    const load = () => void request("session.events", { id: currentId, tail: 200 }).then((p) => {
      if (!alive) return;
      setEvents(p.events);
      last = p.events.at(-1)?.seq ?? 0;
      setDeltas({});
    }).catch((e) => alive && setError(message(e)));
    load();
    const off = subscribe([`session/${currentId}`], {
      event: (e) => {
        const ev = e.payload as SessionEvent;
        if (ev.seq === 0) {
          const b = ev.body;
          if (b.kind === "assistant_delta") setDeltas((d) => ({ ...d, [b.item_id]: (d[b.item_id] ?? "") + b.text }));
          return;
        }
        if (ev.seq <= last) return;
        if (ev.seq !== last + 1) { load(); return; }
        last = ev.seq;
        setEvents((list) => [...list, ev]);
        if (ev.body.kind === "assistant_message" || ev.body.kind === "turn_end") setDeltas({});
      },
      resync: load,
    });
    return () => { alive = false; off(); };
  }, [currentId]);

  return { all, sessions, current, events, deltas, error, reload: loadSessions };
}

const SUGGESTIONS: { label: string; icon: "sum" | "q" | "eye"; text: string; selection?: boolean }[] = [
  { label: "Summarize this page", icon: "sum", text: "Summarize this page." },
  { label: "Explain my selection", icon: "q", text: "Explain this:", selection: true },
  { label: "Find…", icon: "eye", text: "Find on this page: " },
];

export function Chat({ state, tab, botId, setBotId, threadId: picked, setThreadId }: { state: ConnectionState; tab: ActiveTab | null; botId: string | null; setBotId: (id: string) => void; threadId: string | null; setThreadId: (id: string | null) => void }) {
  const { list, error: botsError } = useBots(state);
  const ws = state.workspace ?? "";
  const bots = list?.bots ?? [];
  const saved = typeof localStorage !== "undefined" ? localStorage.getItem(BOT_KEY(ws)) : null;
  const bot = bots.find((b) => b.id === botId) ?? bots.find((b) => b.id === saved) ?? bots.find((b) => b.id === list?.ask) ?? bots[0] ?? null;
  const threads = useThreads(bot?.id ?? null);
  const threadId = picked && (threads.some((t) => t.id === picked) || !threads.length) ? picked : "main";
  const thread = useThread(bot?.id ?? null, threadId);
  const [text, setText] = useState("");
  const [withTab, setWithTab] = useState(true);
  const [quote, setQuote] = useState<Quote | null>(null);
  const [fresh, setFresh] = useState(false);
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [picker, setPicker] = useState(false);
  const [now, setNow] = useState(() => Date.now());
  const box = useRef<HTMLTextAreaElement>(null);
  const end = useRef<HTMLDivElement>(null);

  const choose = (id: string) => { localStorage.setItem(BOT_KEY(ws), id); setBotId(id); setThreadId(null); setPicker(false); };
  const chooseThread = (id: string) => { setThreadId(id === "main" ? null : id); setFresh(false); setPicker(false); };

  // P3: the selection the context menu sent becomes a quote in the box (never sent by itself).
  useEffect(() => {
    void request("quote.take").then((q) => q && setQuote(q)).catch(() => undefined);
    return subscribe([QUOTE_TOPIC], { event: (e) => { setQuote(e.payload as Quote); void request("quote.take").catch(() => undefined); box.current?.focus(); } });
  }, []);
  useEffect(() => { setWithTab(true); }, [tab?.id, tab?.url]);

  const running = !!thread.current && isRunning(thread.current);
  const rows = useMemo(() => chatRows(thread.events, running, thread.deltas), [thread.events, running, thread.deltas]);
  useEffect(() => { if (!running) return; const t = setInterval(() => setNow(Date.now()), 1000); return () => clearInterval(t); }, [running]);
  useEffect(() => { end.current?.scrollIntoView({ block: "end" }); }, [rows.length, rows.at(-1)?.t === "answer" ? (rows.at(-1) as { text: string }).text.length : 0]);

  const page = tab && isWebPage(tab.url) && !tab.url.startsWith("https://chromewebstore.google.com") ? tab : null;
  const attach = withTab && !!page;

  const suggest = async (s: (typeof SUGGESTIONS)[number]) => {
    let t = s.text;
    if (s.selection && page) {
      const sel = await selectionOf(page);
      if (sel) setQuote({ text: sel, url: page.url, title: page.title });
    }
    setText(t);
    requestAnimationFrame(() => { box.current?.focus(); box.current?.setSelectionRange(t.length, t.length); });
  };

  const send = () => {
    const typed = text.trim();
    if (!bot || busy || (!typed && !quote)) return;
    // The permission prompt must start in the click, before any await.
    const allowed = attach ? askSite(page.url) : Promise.resolve(false);
    setBusy("Reading the page…");
    setError(null);
    void (async () => {
      try {
        let file = null;
        if (attach) {
          await allowed;
          file = await snapshot(page).catch(() => null);
          if (!file) throw new Error("Launchpad could not read this page. Remove the page chip to send without it, or click the toolbar button on this tab and try again.");
        }
        const parts: string[] = [];
        if (quote) parts.push(quote.text.split("\n").map((l) => `> ${l}`).join("\n") + (quote.url ? `\n\n(selected on ${quote.url})` : ""));
        if (typed) parts.push(typed);
        if (file) parts.push(`Page: ${file.url} (attached as page.md)`);
        // On a GitHub issue or pull request, the item goes by reference too.
        const gh = page ? pageOf(page.url) : null;
        const refs: GitHubRef[] = gh?.repo && gh.number && ["pr_conversation", "pr_files", "pr_commits", "issue"].includes(gh.kind)
          ? [{ url: `https://github.com/${gh.repo}/${gh.kind === "issue" ? "issues" : "pull"}/${gh.number}`, kind: gh.kind === "issue" ? "issue" : "pr", repo: gh.repo, number: gh.number, title: page?.title ?? null }]
          : [];
        setBusy("Sending…");
        const s = await request("bot.message", { bot: bot.id, text: parts.join("\n\n"), attachments: file ? [{ name: file.name, data_url: file.data_url }] : [], github_refs: refs, ...(fresh ? { new_thread: true } : { thread: threadId }), key: crypto.randomUUID() });
        setText(""); setQuote(null); setFresh(false);
        // A new conversation is a new thread of the bot: the chat follows it.
        if (fresh && s?.launch_spec?.bot?.thread) setThreadId(s.launch_spec.bot.thread);
        void thread.reload();
      } catch (e) {
        setError(message(e));
      } finally {
        setBusy(null);
      }
    })();
  };

  const stop = () => { if (thread.current) void request("session.stop", { id: thread.current.id }).catch((e) => setError(message(e))); };
  const openBot = () => bot && void request("open", { path: `/bots/${encodeURIComponent(bot.id)}?ws=${encodeURIComponent(ws)}${threadId !== "main" ? `&thread=${encodeURIComponent(threadId)}` : ""}` }).catch((e) => setError(message(e)));
  const shownThread = threads.find((t) => t.id === threadId);
  const threadState = (t: BotThread) => {
    const open = [...thread.all].reverse().find((s) => threadOf(s) === t.id);
    return open ? (open.open_question ? "asks you" : isRunning(open) ? "working" : "idle") : "";
  };

  if (botsError) return <div className="x-body"><p className="x-error">{botsError}</p></div>;
  if (list && !bots.length) return <div className="x-body"><div className="x-hello"><h1>No bot in this workspace</h1><p className="x-muted">Create one in Launchpad (Bots → New bot): the page chat talks to a bot of the workspace.</p></div></div>;

  const shown = fresh ? [] : rows;
  return <>
    <div className="x-body" aria-live="polite">
      {thread.error && <p className="x-error">{thread.error}</p>}
      {!shown.length && <div className="x-hello">
        <h1>{fresh ? "New conversation" : `Chat with ${bot?.name ?? "a bot"}`}</h1>
        <p className="x-muted">{fresh ? "Your next message starts a new thread of the bot, next to its others." : "About this page or anything else. The page is read once when you send, and attached."}</p>
      </div>}
      {shown.map((r) => <Row key={r.key} row={r} now={now} onStop={stop} onOpen={openBot} />)}
      {!fresh && thread.current && !running && shown.length > 0 && <div className="x-btns">
        <button className="x-btn x-sm x-link" onClick={openBot}><Icon name="ext" size={14} /> Open in Launchpad</button>
        <button className="x-btn x-sm x-link" onClick={() => { const a = [...rows].reverse().find((r) => r.t === "answer"); if (a && a.t === "answer") void navigator.clipboard.writeText(a.text); }}>Copy</button>
      </div>}
      <div ref={end} />
    </div>
    <div className="x-foot">
      {error && <p className="x-error" role="alert" style={{ margin: "0 4px 6px" }}>{error}</p>}
      <div className="x-box">
        {(page || quote) && <div className="x-chips">
          {page && (withTab
            ? <span className="x-chip" title={`${page.title}\n${page.url}\nRead once when you send, and attached`}><Favicon url={page.url} title={page.title} src={page.favicon} size={14} /><span>{hostOf(page.url)}</span><button aria-label="Send without this page" title="Send without this page" onClick={() => setWithTab(false)}><Icon name="close" size={12} /></button></span>
            : <button className="x-chip x-ghost" onClick={() => setWithTab(true)}><Icon name="plus" size={12} /><span>This tab</span></button>)}
          {quote && <span className="x-chip x-quote" title={quote.text}><Icon name="quote" size={12} /><span>{quote.text.replace(/\s+/g, " ").slice(0, 80)}</span><button aria-label="Remove the quote" title="Remove the quote" onClick={() => setQuote(null)}><Icon name="close" size={12} /></button></span>}
        </div>}
        <textarea ref={box} rows={2} value={text} placeholder={page ? "Ask about this page…" : "Ask a bot…"} aria-label="Message" disabled={!bot}
          onChange={(e) => setText(e.target.value)}
          onKeyDown={(e) => { if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) { e.preventDefault(); send(); } }} />
        {!text && !shown.length && page && <div className="x-chips">
          {SUGGESTIONS.map((s) => <button key={s.label} className="x-pill" onClick={() => void suggest(s)}><Icon name={s.icon} size={14} /><span>{s.label}</span></button>)}
        </div>}
        <div className="x-row">
          <div className="x-pills">
            <button className="x-pill x-shrink" aria-haspopup="menu" aria-expanded={picker} title="The bot and the thread this chat talks to" onClick={() => setPicker(!picker)}><span>{bot?.name ?? "Choose a bot"}{!fresh && threadId !== "main" && shownThread ? ` · ${shownThread.title}` : ""}</span><Icon name="chev" size={14} /></button>
            {!fresh && (thread.current || threadId !== "main") && <button className="x-pill" title="Your next message starts a new thread of the bot, next to its others" onClick={() => setFresh(true)}><Icon name="plus" size={14} /><span>New conversation</span></button>}
            {fresh && <button className="x-pill x-on" title="Back to the current conversation" onClick={() => setFresh(false)}><span>New conversation ×</span></button>}
          </div>
          {busy ? <span className="x-line" style={{ marginLeft: "auto" }}><span className="x-spin" />{busy}</span>
            : <button className="x-send" aria-label="Send" title="Send (Enter)" disabled={!bot || (!text.trim() && !quote)} onClick={send}><Icon name="send" size={18} /></button>}
        </div>
      </div>
    </div>
    {picker && <>
      <div className="x-scrim" onClick={() => setPicker(false)} />
      <div className="x-menu" role="menu" aria-label="Bots">
        <h2>Talk to</h2>
        {bots.map((b) => <button key={b.id} role="menuitemradio" aria-checked={b.id === bot?.id} className={`x-opt ${b.id === bot?.id ? "x-sel" : ""}`} onClick={() => choose(b.id)}>
          <span className={`x-radio ${b.id === bot?.id ? "x-on" : ""}`} /><span className="x-d"><b>{b.name}</b><small>{b.id === list?.ask ? "The workspace's Ask bot" : `@${b.slug}`}{b.paused ? " · paused" : ""}</small></span>
        </button>)}
        {threads.length > 1 && <>
          <h2>Thread</h2>
          {threads.slice(0, 12).map((t) => <button key={t.id} role="menuitemradio" aria-checked={!fresh && t.id === threadId} className={`x-opt ${!fresh && t.id === threadId ? "x-sel" : ""}`} onClick={() => chooseThread(t.id)}>
            <span className={`x-radio ${!fresh && t.id === threadId ? "x-on" : ""}`} /><span className="x-d"><b>{t.title}</b><small>{threadState(t) || (t.id === "main" ? "The bot's main conversation" : "No open session")}</small></span>
          </button>)}
        </>}
      </div>
    </>}
  </>;
}

function Row({ row: r, now, onStop, onOpen }: { row: ChatRow; now: number; onStop: () => void; onOpen: () => void }) {
  switch (r.t) {
    case "user": return <div className="x-user">{r.from && <div className="x-small x-faint">From {r.from}</div>}{r.text}{r.queued && <div className="x-small x-faint">Waits for the current turn</div>}</div>;
    case "answer": return <div className="x-ai" style={{ whiteSpace: "pre-wrap" }}>{r.text}{r.streaming && <span className="x-faint"> …</span>}</div>;
    case "steps": return <div className="x-line">{r.count} {r.count === 1 ? "step" : "steps"}{r.durationMs !== null && ` · ${duration(r.durationMs)}`}</div>;
    case "ask": return <div className="x-card x-ask"><p>{r.question}</p><div className="x-btns"><button className="x-btn x-sm" onClick={onOpen}>Answer in Launchpad</button></div></div>;
    case "ended": return <div className="x-line x-bad">{r.text}</div>;
    case "working": return <div className="x-banner x-run"><span className="x-spin" /><span>{r.step ?? "Working"} · {duration(now - r.since)}</span><button className="x-btn x-sm" onClick={onStop}>Stop</button></div>;
  }
}
