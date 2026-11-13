// Side panel once connected: the connection, the text size and a link to Launchpad. The page chat and the GitHub
// layer replace it.

import { useState } from "react";
import { createRoot } from "react-dom/client";
import { daemonUrl } from "@/lib/connection";
import { Icon } from "./icons";
import { openInLaunchpad } from "./lp";
import { TEXT_SIZES, applyTextSize, textSize } from "./textSize";

function Root() {
  const [size, setSize] = useState(textSize);
  const disconnect = async () => {
    await chrome.storage.local.remove("daemon");
    localStorage.removeItem("launchpad-daemon-origin");
    location.reload();
  };
  return <div className="x-app">
    <header className="x-top">
      <img className="x-logo" src={chrome.runtime.getURL("icons/48.png")} alt="" />
      <span className="x-title">Launchpad</span>
    </header>
    <div className="x-body" style={{ gap: 0 }}>
      <p className="x-muted" style={{ margin: "8px 4px 16px" }}>The page chat and Launchpad on GitHub are coming soon. Your agents stay one click away in Launchpad.</p>
      <button className="x-btn x-pri" onClick={() => openInLaunchpad("/")}>Open Launchpad <Icon name="ext" size={14} /></button>
      <div className="x-sect" style={{ marginLeft: 4, marginTop: 16 }}>Connection</div>
      <div className="x-setrow"><Icon name="plug" /><span className="x-d"><b style={{ fontWeight: 500 }}>Connected to Launchpad</b><small>{daemonUrl("")}</small></span><button className="x-btn x-sm" onClick={() => void disconnect()}>Disconnect</button></div>
      <div className="x-sect" style={{ marginLeft: 4, marginTop: 16 }}>Text size</div>
      <div className="x-sizes" role="radiogroup" aria-label="Text size">{TEXT_SIZES.map((v) => <button key={v} role="radio" aria-checked={v === size} className={`x-btn x-sm ${v === size ? "x-pri" : ""}`} onClick={() => { applyTextSize(v); setSize(v); }}>{v}%</button>)}</div>
    </div>
  </div>;
}

export function mount() {
  document.documentElement.classList.add("x-panel");
  createRoot(document.getElementById("root")!).render(<Root />);
}
