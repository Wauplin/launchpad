// Connected: the workspace in the header, Today's count of that workspace, where this browser is listed on Home, and the
// first steps. The page chat and the item view replace the body in later steps.

import { useState } from "react";
import type { ConnectionState } from "../shared/messages";
import { request } from "../shared/messages";
import { Icon } from "../icons";
import { openInLaunchpad } from "../lp";
import type { Screen } from "./main";

export function Header({ state, open }: { state: ConnectionState; open: (s: Screen) => void }) {
  const [error, setError] = useState<string | null>(null);
  const choose = (id: string) => {
    setError(null);
    request("workspace.set", { id }).catch((e) => setError(e instanceof Error ? e.message : String(e)));
  };
  return <>
    <header className="x-top">
      <img className="x-logo" src={chrome.runtime.getURL("icons/48.png")} alt="" />
      <span className="x-title">Launchpad</span>
      {state.workspaces.length > 0 && <select className="x-ws" aria-label="Workspace" title="The extension's workspace: its Today count shows on the toolbar button" value={state.workspace ?? ""} onChange={(e) => choose(e.target.value)}>
        {state.workspaces.map((w) => <option key={w.id} value={w.id}>{w.name}</option>)}
      </select>}
      <button className="x-ib" aria-label="Settings" title="Settings" onClick={() => open("settings")}><Icon name="gear" /></button>
    </header>
    {error && <p className="x-error" style={{ margin: "0 14px" }}>{error}</p>}
  </>;
}

export function Connected({ state, open }: { state: ConnectionState; open: (s: Screen) => void }) {
  const home = state.home!;
  return <div className="x-app">
    <Header state={state} open={open} />
    <div className="x-body">
      {state.phase === "starting" && <div className="x-banner x-off"><span className="x-spin" /><span>Home is starting…</span></div>}
      {state.phase === "offline" && <div className="x-banner x-warn"><Icon name="warn" size={14} /><span>Home does not answer. Trying again…</span></div>}
      {state.phase === "connecting" && <div className="x-banner x-off"><span className="x-spin" /><span>Connecting…</span></div>}
      <div className="x-card">
        <h3>Connected</h3>
        <p className="x-muted">
          {state.today === null ? "Today: …" : state.today === 0 ? "Today: nothing waits for you" : <>Today: <span className="x-count">{state.today}</span> waiting for you</>}
        </p>
        <p className="x-small x-muted">This browser is listed in Home's <b>Settings → Devices</b> as “{home.device_name}”. Disconnect it there at any time.</p>
      </div>
      <p className="x-muted" style={{ margin: "0 4px" }}>The page chat and Launchpad on GitHub are coming soon.</p>
      <div className="x-sugg">
        <button onClick={() => void chrome.tabs.create({ url: "https://github.com/" })}><Icon name="folder" size={16} /> Open a GitHub repository</button>
        <button onClick={() => openInLaunchpad(home, "/")}><Icon name="ext" size={16} /> Open Launchpad</button>
      </div>
    </div>
  </div>;
}
