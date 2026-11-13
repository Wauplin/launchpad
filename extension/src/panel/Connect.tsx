// Connection screens: Welcome (Sign in with Hugging Face, or an address), Find your Home (the Spaces discovery found),
// the address with a pairing code, and Home that disconnected this browser.

import { useEffect, useRef, useState } from "react";
import type { ConnectionState, FoundHome } from "../shared/messages";
import { request } from "../shared/messages";
import { Icon } from "../icons";
import type { Screen } from "./main";

const message = (e: unknown) => (e instanceof Error ? e.message : String(e));
const logo = () => chrome.runtime.getURL("icons/128.png");

function Problem({ text }: { text: string | null | undefined }) {
  return text ? <p className="x-error" role="alert">{text}</p> : null;
}

export function Welcome({ state, open }: { state: ConnectionState; open: (s: Screen) => void }) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const signIn = async () => {
    setBusy(true); setError(null);
    try { await request("signin"); } catch (e) { setError(message(e)); } finally { setBusy(false); }
  };
  return <div className="x-app"><div className="x-center">
    <img src={logo()} width="56" height="56" alt="" style={{ borderRadius: 16 }} />
    <h1>Launchpad for Chrome</h1>
    <p className="x-muted">Your agents on GitHub and in a chat about any page.</p>
    {state.client_id
      ? <button className="x-btn x-hf" disabled={busy} onClick={() => void signIn()}><span aria-hidden="true">🤗</span> {busy ? "Signing in…" : "Sign in with Hugging Face"}</button>
      : <>
        <button className="x-btn x-hf" disabled><span aria-hidden="true">🤗</span> Sign in with Hugging Face</button>
        <p className="x-small x-muted">Sign in with Hugging Face is not configured yet. <button className="x-inline" onClick={() => open("settings")}>Set its client ID</button></p>
      </>}
    <Problem text={error ?? state.error} />
    <p className="x-small x-faint" style={{ marginTop: 6 }}>Home on another computer? <button className="x-inline" onClick={() => open("address")}>Use an address</button></p>
  </div></div>;
}

function stageText(h: FoundHome) {
  if (h.answers) return "Running · Home answers ✓";
  switch (h.stage) {
    case "RUNNING": return h.own ? "Running · Home does not answer" : "Running · an organization's Space";
    case "SLEEPING": case "PAUSED": return "Sleeping · starts when chosen";
    case "BUILDING": case "APP_STARTING": case "RUNNING_BUILDING": case "RUNNING_APP_STARTING": return "Starting";
    case null: return "State unknown";
    default: return h.stage.charAt(0) + h.stage.slice(1).toLowerCase().replace(/_/g, " ");
  }
}

/** After sign-in: the Launchpad Spaces of the account and its organizations. One Home of the account connects at once. */
export function FindHome({ state, open }: { state: ConnectionState; open: (s: Screen) => void }) {
  const [homes, setHomes] = useState<FoundHome[] | null>(null);
  const [chosen, setChosen] = useState<FoundHome | null>(null);
  const [connecting, setConnecting] = useState<FoundHome | null>(null);
  const [error, setError] = useState<string | null>(null);
  const attempt = useRef(0);

  const connect = async (home: FoundHome) => {
    const mine = ++attempt.current;
    setConnecting(home); setError(null);
    // A sleeping Space answers with the Hub's page until it runs: ask again every 5 s, for up to 5 minutes.
    const until = Date.now() + 5 * 60_000;
    try {
      while (mine === attempt.current) {
        const s = await request("connect.space", { space: home.space, origin: home.origin });
        if (s.phase !== "starting") return;
        if (Date.now() > until) throw new Error("Home did not start within 5 minutes. Try again.");
        await new Promise((r) => setTimeout(r, 5_000));
      }
    } catch (e) {
      if (mine === attempt.current) { setError(message(e)); setConnecting(null); }
    }
  };
  const find = async () => {
    setHomes(null); setError(null);
    try {
      const found = await request("discover");
      setHomes(found);
      setChosen(found.find((h) => h.answers) ?? found[0] ?? null);
      // Only a Space of your own account connects without a choice (connecting sends it your Hugging Face token).
      if (found.length === 1 && found[0].own) void connect(found[0]);
    } catch (e) { setError(message(e)); setHomes([]); }
  };
  useEffect(() => { void find(); }, []);

  if (connecting) {
    const starting = state.phase === "starting";
    return <div className="x-app"><div className="x-center">
      <span className="x-spin" />
      <h1>{starting ? "Home is starting" : "Connecting…"}</h1>
      <p className="x-muted">{connecting.space}</p>
      {starting && <p className="x-small x-muted">A sleeping Space takes a minute or two to wake up. Keep this panel open.</p>}
      <button className="x-btn x-link" onClick={() => { attempt.current++; setConnecting(null); }}>Cancel</button>
    </div></div>;
  }
  return <div className="x-app">
    <header className="x-top"><img className="x-logo" src={logo()} alt="" /><span className="x-title">Your Launchpad Home</span></header>
    <div className="x-body">
      {state.user && <p className="x-small x-muted" style={{ margin: "0 2px" }}>Signed in to Hugging Face as <b>{state.user.name}</b></p>}
      {homes === null && <div className="x-line"><span className="x-spin" /> Looking for your Home in your account and organizations…</div>}
      {homes?.length === 0 && !error && <p className="x-muted">No Launchpad Home found in your account or your organizations. If it belongs to an organization, sign in again and allow that organization on Hugging Face's page.</p>}
      {!!homes?.length && <div className="x-homes" role="radiogroup" aria-label="Your Launchpad Home">
        {homes.map((h) => <button key={h.space} role="radio" aria-checked={chosen?.space === h.space} className={`x-home ${chosen?.space === h.space ? "x-on" : ""}`} onClick={() => setChosen(h)}>
          <span className={`x-dot ${h.answers ? "x-dot-on" : ""}`} />
          <span className="x-d"><b>{h.space}</b><small>{stageText(h)}</small></span>
        </button>)}
      </div>}
      <Problem text={error ?? state.error} />
      {!!homes?.length && <button className="x-btn x-pri" disabled={!chosen} onClick={() => chosen && void connect(chosen)}>Connect</button>}
      <div className="x-btns">
        {homes !== null && <button className="x-btn x-sm" onClick={() => void find()}>Search again</button>}
        <button className="x-btn x-sm x-link" onClick={() => open("address")}>Use an address</button>
        <button className="x-btn x-sm x-link" onClick={() => void request("disconnect")}>Sign out</button>
      </div>
    </div>
  </div>;
}

/** A dev or local Home: an address, then a pairing code the owner approves in Settings → Devices. */
export function Address({ state, back }: { state: ConnectionState; back: () => void }) {
  const [address, setAddress] = useState(state.home?.kind === "address" ? state.home.origin : "");
  const [code, setCode] = useState<{ code: string; expires_at: number } | null>(null);
  const [status, setStatus] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const attempt = useRef(0);
  useEffect(() => () => { attempt.current++; }, []);

  const start = (event: React.FormEvent) => {
    event.preventDefault();
    const mine = ++attempt.current;
    setError(null); setCode(null); setStatus(null);
    let origin: string;
    try {
      const url = new URL(address.trim());
      origin = url.origin;
    } catch { setError("Enter an address like https://your-machine.ts.net or http://127.0.0.1:7317"); return; }
    // Requested in the click itself: Chrome asks only during a user gesture.
    void chrome.permissions.request({ origins: [`${origin}/*`] }).then(async (granted) => {
      if (!granted) throw new Error("Launchpad needs permission to reach this address");
      const pair = await request("connect.address.start", { origin: address.trim() });
      if (mine !== attempt.current) return;
      setCode(pair);
      setStatus("Waiting for approval… keep this panel open.");
      while (mine === attempt.current) {
        await new Promise((r) => setTimeout(r, 3_000));
        if (mine !== attempt.current) return;
        const r = await request("connect.address.poll");
        if (r.approved) { back(); return; }
      }
    }).catch((e) => { if (mine === attempt.current) { setError(message(e)); setStatus(null); } });
  };
  return <div className="x-app">
    <header className="x-top">
      <button className="x-ib" aria-label="Back" title="Back" onClick={back}><Icon name="back" /></button>
      <span className="x-title">Connect with an address</span>
    </header>
    <div className="x-body">
      <p className="x-muted" style={{ margin: "0 2px" }}>For a Home on another computer or a dev Home. Home on a Hugging Face Space? Use Sign in with Hugging Face instead.</p>
      <form onSubmit={start} style={{ display: "grid", gap: 8 }}>
        <label className="x-small x-faint">Launchpad address<input className="x-field" style={{ marginTop: 4 }} type="url" required placeholder="https://your-machine.ts.net" value={address} onChange={(e) => setAddress(e.target.value)} /></label>
        <button className="x-btn x-pri" type="submit">Show pairing code</button>
      </form>
      {code && <>
        <p className="x-muted" style={{ margin: "0 2px" }}>Open Launchpad on a paired device → Settings → Devices, and approve this code:</p>
        <p className="x-code" aria-live="polite" style={{ justifySelf: "center", alignSelf: "center", margin: 0 }}>{code.code.slice(0, 4)}-{code.code.slice(4)}</p>
      </>}
      {status && <div className="x-line" role="status"><span className="x-spin" /> {status}</div>}
      <Problem text={error} />
    </div>
  </div>;
}

/** Home refused this browser's device: it was disconnected in Settings → Devices. */
export function Revoked({ state, open }: { state: ConnectionState; open: (s: Screen) => void }) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const home = state.home!;
  const again = async () => {
    if (home.kind === "address" || !home.space) { open("address"); return; }
    setBusy(true); setError(null);
    try { await request("connect.space", { space: home.space, origin: home.origin }); } catch (e) { setError(message(e)); } finally { setBusy(false); }
  };
  return <div className="x-app"><div className="x-center">
    <img src={logo()} width="56" height="56" alt="" style={{ borderRadius: 16 }} />
    <h1>This browser was disconnected</h1>
    <p className="x-muted">Home no longer accepts this browser ({home.space ?? home.origin}). Connect it again, or sign out.</p>
    <Problem text={error} />
    <button className="x-btn x-pri" disabled={busy} onClick={() => void again()}>Connect again</button>
    <button className="x-btn x-link" onClick={() => void request("disconnect")}>Sign out</button>
  </div></div>;
}
