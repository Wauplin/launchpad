// Today: the agents' unread private notes first (each opens its item on github.com), then the workspace's Today items
// grouped like Launchpad's Today page, in decision order. Each card opens in Launchpad: deciding stays there.

import { useEffect, useState } from "react";
import type { ExtUnreadNotes } from "@/gen/ExtUnreadNotes";
import type { TodayFilter } from "@/gen/TodayFilter";
import type { TodayItem } from "@/gen/TodayItem";
import { age } from "@/lib/format";
import { Icon } from "../icons";
import { request, subscribe, type ConnectionState } from "../shared/messages";
import type { ActiveTab } from "./tab";

const message = (e: unknown) => (e instanceof Error ? e.message : String(e));

const SECTIONS: [TodayFilter, string][] = [["answer", "To answer"], ["review", "To review"], ["approve", "To approve"], ["send", "To send"], ["github", "GitHub"], ["fix", "To fix"]];

/** Opens a github.com page in the panel window's active tab when it shows GitHub (the reading stays in one tab),
 * else in a new tab. */
function openOnGitHub(url: string, tab: ActiveTab | null): Promise<unknown> {
  return tab && tab.url.startsWith("https://github.com/") ? chrome.tabs.update(tab.id, { url }) : chrome.tabs.create({ url });
}

/** `owner/repo#12` as an issue URL: GitHub redirects it to the pull request when it is one. */
function itemUrl(ref: string): string | null {
  const m = /^([^/\s#]+\/[^/\s#]+)#(\d+)$/.exec(ref);
  return m ? `https://github.com/${m[1]}/issues/${m[2]}` : null;
}

// Stable loaders, so `useLive` loads again only when the workspace or the connection changes.
const loadToday = () => request("today");
const loadNotes = () => request("notes.unread");

/** Loads with `load`, then again (debounced) on each event of `topic` and on a resync. */
function useLive<T>(state: ConnectionState, topic: string, load: () => Promise<T>): [T | null, string | null, (f: (v: T | null) => T | null) => void] {
  const [value, setValue] = useState<T | null>(null);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    if (!state.workspace || state.phase !== "online") return;
    let alive = true;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const run = () => void load().then((v) => { if (alive) { setValue(v); setError(null); } }).catch((e) => alive && setError(message(e)));
    const later = () => { clearTimeout(timer); timer = setTimeout(run, 400); };
    run();
    const off = subscribe([topic], { event: later, resync: later });
    return () => { alive = false; clearTimeout(timer); off(); };
  }, [state.workspace, state.phase, topic, load]);
  return [value, error, setValue];
}

export function Today({ state, tab }: { state: ConnectionState; tab: ActiveTab | null }) {
  const [items, itemsError] = useLive(state, "today", loadToday);
  const [notes, notesError, setNotes] = useLive(state, "notes", loadNotes);
  const [error, setError] = useState<string | null>(null);

  const ws = state.workspace ?? "";
  const wsName = state.workspaces.find((w) => w.id === ws)?.name ?? "this workspace";
  const open = (path: string) => void request("open", { path: `${path}${path.includes("?") ? "&" : "?"}ws=${encodeURIComponent(ws)}` }).catch((e) => setError(message(e)));
  const github = (url: string) => void openOnGitHub(url, tab).catch((e) => setError(message(e)));
  // Hidden at once; the `notes` event then brings the list from Home.
  const markRead = (n?: ExtUnreadNotes) => {
    setError(null);
    setNotes((l) => (n ? l?.filter((x) => x.repo !== n.repo || x.number !== n.number) ?? null : []));
    void request("note.read", n ? { repo: n.repo, number: n.number } : {}).catch((e) => setError(message(e)));
  };

  const failed = itemsError ?? notesError;
  if (failed && !items) return <div className="x-body"><p className="x-error">{failed}</p></div>;
  if (!items) return <div className="x-body"><div className="x-line"><span className="x-spin" />Loading…</div></div>;

  return <div className="x-body" style={{ gap: 2 }}>
    {(error ?? failed) && <p className="x-error">{error ?? failed}</p>}
    {notes && notes.length > 0 && <>
      <div className="x-sect x-sect-act"><span>Unread notes</span><button title="Mark every note of this workspace read" onClick={() => markRead()}>Mark all read</button></div>
      {notes.map((n) => <div key={`${n.repo}#${n.number}`} className="x-task" role="group">
        <button className="x-d x-hit" title="Open on GitHub" onClick={() => github(`https://github.com/${n.repo}/${n.kind === "pr" ? "pull" : "issues"}/${n.number}`)}>
          <b>{n.repo}#{n.number}{n.title ? ` · ${n.title}` : ""}</b>
          <small>{n.unread} new · {n.authors.join(", ")} · {age(n.newest_at)}</small>
          <small>{n.excerpt}</small>
        </button>
        <button className="x-ib" aria-label="Mark read" title="Mark read" onClick={() => markRead(n)}><Icon name="check" size={16} /></button>
      </div>)}
    </>}
    {SECTIONS.map(([filter, label]) => {
      const list = items.filter((i) => i.filter === filter);
      return list.length > 0 && <div key={filter} style={{ display: "contents" }}>
        <div className="x-sect">{label}</div>
        {list.map((i) => <TodayRow key={i.id} i={i} open={() => open(`/?card=${encodeURIComponent(i.id)}`)} github={github} />)}
      </div>;
    })}
    {items.length === 0 && <p className="x-muted" style={{ margin: "8px 4px" }}>Nothing waits for you in {wsName}.</p>}
    <div className="x-btns" style={{ marginTop: 8 }}>
      <button className="x-btn x-sm x-link" onClick={() => open("/")}><Icon name="ext" size={14} /> Open Today in Launchpad</button>
    </div>
  </div>;
}

function TodayRow({ i, open, github }: { i: TodayItem; open: () => void; github: (url: string) => void }) {
  const url = i.github ? itemUrl(i.github) : null;
  return <div className="x-task" role="group">
    <button className="x-d x-hit" title="Open its card in Launchpad" onClick={open}>
      <b>{i.title}</b>
      {i.detail && <small>{i.detail}</small>}
    </button>
    {i.grouped.length > 0 && <span className="x-num" title={`${i.grouped.length} more inside this card`}>+{i.grouped.length}</span>}
    <span className="x-small x-faint x-age">{age(i.at)}</span>
    {url && <button className="x-ib" aria-label="On GitHub" title={`On GitHub: ${i.github}`} onClick={() => github(url)}><Icon name="github" size={16} /></button>}
  </div>;
}
