// Connected: the header (workspace, Today's count, Settings) and three tabs: Chat (a bot about the current page, on any
// site), This item (on a GitHub pull request or issue: what Launchpad has about it) and Agents (the workspace's agents).
// Everything heavy opens in Launchpad.

import { useEffect, useState } from "react";
import type { ConnectionState } from "../shared/messages";
import { request } from "../shared/messages";
import { Icon } from "../icons";
import { pageOf } from "../content/github/page";
import { Agents } from "./Agents";
import { Chat } from "./Chat";
import { ITEM_KINDS, Item } from "./Item";
import type { Screen } from "./main";
import { useActiveTab } from "./tab";

export function Header({ state, open }: { state: ConnectionState; open: (s: Screen) => void }) {
  const [error, setError] = useState<string | null>(null);
  const choose = (id: string) => {
    setError(null);
    request("workspace.set", { id }).catch((e) => setError(e instanceof Error ? e.message : String(e)));
  };
  const today = state.today;
  return <>
    <header className="x-top">
      <img className="x-logo" src={chrome.runtime.getURL("icons/48.png")} alt="" />
      <span className="x-title">Launchpad</span>
      {today !== null && today > 0 && <button className="x-count x-today" title={`Today: ${today} waiting for you. Open Today in Launchpad`} onClick={() => void request("open", { path: `/?ws=${encodeURIComponent(state.workspace ?? "")}` })}>{today}</button>}
      {state.workspaces.length > 0 && <select className="x-ws" aria-label="Workspace" title="The extension's workspace: its Today count shows on the toolbar button" value={state.workspace ?? ""} onChange={(e) => choose(e.target.value)}>
        {state.workspaces.map((w) => <option key={w.id} value={w.id}>{w.name}</option>)}
      </select>}
      <button className="x-ib" aria-label="Settings" title="Settings" onClick={() => open("settings")}><Icon name="gear" /></button>
    </header>
    {error && <p className="x-error" style={{ margin: "0 14px" }}>{error}</p>}
  </>;
}

type Tab = "chat" | "item" | "agents";

export function Connected({ state, open }: { state: ConnectionState; open: (s: Screen) => void }) {
  const tab = useActiveTab();
  const page = tab ? pageOf(tab.url) : null;
  const onItem = !!page && (ITEM_KINDS as readonly string[]).includes(page.kind) && !!page.repo && !!page.number;
  const [view, setView] = useState<Tab>("chat");
  const [bot, setBot] = useState<string | null>(null);
  // The chat's thread of that bot (null: its main thread); This item's Continue opens the item's thread.
  const [thread, setThread] = useState<string | null>(null);
  // Leaving a GitHub item goes back to the chat.
  useEffect(() => { if (!onItem) setView((v) => (v === "item" ? "chat" : v)); }, [onItem]);

  const tabs: [Tab, string][] = [["chat", "Chat"], ...(onItem ? [["item", "This item"] as [Tab, string]] : []), ["agents", "Agents"]];
  return <div className="x-app">
    <Header state={state} open={open} />
    {state.phase === "starting" && <div className="x-banner x-off" style={{ margin: "0 12px 6px" }}><span className="x-spin" /><span>Home is starting…</span></div>}
    {state.phase === "offline" && <div className="x-banner x-warn" style={{ margin: "0 12px 6px" }}><Icon name="warn" size={14} /><span>Home does not answer. Trying again…</span></div>}
    {state.phase === "connecting" && <div className="x-banner x-off" style={{ margin: "0 12px 6px" }}><span className="x-spin" /><span>Connecting…</span></div>}
    <nav className="x-tabs" role="tablist" aria-label="Views">
      {tabs.map(([id, label]) => <button key={id} role="tab" aria-selected={view === id} className={view === id ? "x-on" : ""} onClick={() => setView(id)}>{label}</button>)}
    </nav>
    {view === "item" && onItem && tab && page
      ? <Item state={state} tab={tab} page={page} chatWith={(id, t) => { setBot(id); setThread(t ?? null); setView("chat"); }} />
      : view === "agents" ? <Agents state={state} />
      : <Chat state={state} tab={tab} botId={bot} setBotId={setBot} threadId={thread} setThreadId={setThread} />}
  </div>;
}
