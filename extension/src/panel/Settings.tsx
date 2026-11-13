// Settings: the connection, the Hugging Face OAuth client ID (until the app ships with one), text size, the GitHub
// layer's anchors and Disconnect.

import { useEffect, useState } from "react";
import type { AnchorHealth, ConnectionState } from "../shared/messages";
import { request } from "../shared/messages";
import { Icon } from "../icons";
import { TEXT_SIZES, applyTextSize, textSize } from "../textSize";

export function Settings({ state, back }: { state: ConnectionState; back: () => void }) {
  const [size, setSize] = useState(textSize);
  const [clientId, setClientId] = useState(state.client_id?.source === "settings" ? state.client_id.value : "");
  const [saved, setSaved] = useState<string | null>(null);
  const [confirm, setConfirm] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const home = state.home;
  const built = import.meta.env.VITE_HF_CLIENT_ID?.trim();

  const save = async () => {
    setError(null);
    try { await request("settings.client-id", { client_id: clientId.trim() || null }); setSaved(clientId.trim() ? "Saved" : "Cleared"); } catch (e) { setError(e instanceof Error ? e.message : String(e)); }
  };
  const disconnect = async () => {
    setBusy(true);
    try { await request("disconnect"); back(); } catch (e) { setError(e instanceof Error ? e.message : String(e)); } finally { setBusy(false); setConfirm(false); }
  };
  return <div className="x-app">
    <header className="x-top">
      <button className="x-ib" aria-label="Back" title="Back" onClick={back}><Icon name="back" /></button>
      <span className="x-title">Settings</span>
    </header>
    <div className="x-body" style={{ gap: 0 }}>
      <div className="x-sect" style={{ marginLeft: 4 }}>Connection</div>
      {home
        ? <div className="x-setrow"><Icon name="plug" /><span className="x-d"><b style={{ fontWeight: 500 }}>{home.kind === "space" ? home.space : "Home at an address"}</b><small>{home.origin}</small><small>This browser: {home.device_name}</small></span></div>
        : <div className="x-setrow"><Icon name="plug" /><span className="x-d"><b style={{ fontWeight: 500 }}>Not connected</b></span></div>}
      {state.user && <div className="x-setrow"><span style={{ width: 18, textAlign: "center" }} aria-hidden="true">🤗</span><span className="x-d"><b style={{ fontWeight: 500 }}>Signed in to Hugging Face</b><small>{state.user.name}</small></span></div>}

      <div className="x-sect" style={{ marginLeft: 4, marginTop: 16 }}>Sign in with Hugging Face</div>
      <div className="x-setrow" style={{ display: "grid", gap: 6 }}>
        <label className="x-small x-faint">OAuth client ID<input className="x-field" style={{ marginTop: 4 }} value={clientId} placeholder={built || "Client ID of the “Launchpad for Chrome” app"} onChange={(e) => { setClientId(e.target.value); setSaved(null); }} spellCheck={false} autoComplete="off" /></label>
        <small className="x-faint">{built ? "Leave empty to use the client ID this build ships with." : "This build ships without one: paste the client ID of your Hugging Face OAuth app."}</small>
        <div className="x-btns"><button className="x-btn x-sm" onClick={() => void save()}>Save</button>{saved && <span className="x-small x-muted" role="status" style={{ alignSelf: "center" }}>{saved}</span>}</div>
      </div>

      <div className="x-sect" style={{ marginLeft: 4, marginTop: 16 }}>GitHub</div>
      <label className="x-setrow" style={{ cursor: "pointer" }}>
        <input type="checkbox" checked={state.prefill} onChange={(e) => void request("settings.prefill", { on: e.target.checked }).catch((err) => setError(err instanceof Error ? err.message : String(err)))} />
        <span className="x-d"><b style={{ fontWeight: 500 }}>Prefill suggested comments</b><small>When you open a pull request or issue, an agent's suggested comment goes into GitHub's empty comment box. You still post it with GitHub's button.</small></span>
      </label>

      <Anchors />

      <div className="x-sect" style={{ marginLeft: 4, marginTop: 16 }}>Text size</div>
      <div className="x-sizes" role="radiogroup" aria-label="Text size">{TEXT_SIZES.map((v) => <button key={v} role="radio" aria-checked={v === size} className={`x-btn x-sm ${v === size ? "x-pri" : ""}`} onClick={() => { applyTextSize(v); setSize(v); }}>{v}%</button>)}</div>

      {(home || state.user) && <>
        <div className="x-sect" style={{ marginLeft: 4, marginTop: 16 }}>Disconnect</div>
        <div className="x-setrow" style={{ display: "grid", gap: 8 }}>
          <small className="x-muted">Removes this browser from Home's devices and signs out of Hugging Face in this extension.</small>
          {confirm
            ? <div className="x-btns"><button className="x-btn x-sm x-danger" disabled={busy} onClick={() => void disconnect()}>{busy ? "Disconnecting…" : "Disconnect this browser"}</button><button className="x-btn x-sm" onClick={() => setConfirm(false)}>Cancel</button></div>
            : <div className="x-btns"><button className="x-btn x-sm" onClick={() => setConfirm(true)}>Disconnect</button></div>}
        </div>
      </>}
      {error && <p className="x-error" style={{ marginTop: 8 }}>{error}</p>}
    </div>
  </div>;
}

/** "GitHub anchors: 11 of 12 work": the places the GitHub layer found on the pages you opened. A broken one shows
 * nothing on GitHub; its feature stays in this panel. */
function Anchors() {
  const [health, setHealth] = useState<AnchorHealth | null>(null);
  useEffect(() => { void request("anchors.health").then(setHealth).catch(() => setHealth({ anchors: {} })); }, []);
  const list = Object.values(health?.anchors ?? {});
  const broken = list.filter((a) => !a.ok);
  return <>
    <div className="x-sect" style={{ marginLeft: 4, marginTop: 16 }}>GitHub anchors</div>
    <div className="x-setrow" style={{ display: "grid", gap: 4 }}>
      {!health ? <small className="x-faint">Loading…</small>
        : !list.length ? <small className="x-muted">Not checked yet: open a repository on github.com.</small>
        : <>
          <span>{list.length - broken.length} of {list.length} work</span>
          {broken.map((a) => <small key={a.label} className="x-muted">Not found: {a.label} (last checked on a {a.kind.replace(/_/g, " ")} page). Its feature shows only in this panel.</small>)}
        </>}
    </div>
  </>;
}
