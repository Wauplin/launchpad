import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useVirtualizer } from "@tanstack/react-virtual";
import { memo, useCallback, useEffect, useRef, useState } from "react";
import { externalLinkProps, isDesktopShell } from "@/lib/platform";
import { api } from "@/lib/api";
import { copyText } from "@/lib/clipboard";
import { toast, toastError } from "@/components/Toast";
import { formatAge } from "@/lib/format";
import { disablePush, enablePush, sendTestPush, setPushTitles, usePushState, useRefreshPush, type PushState } from "@/lib/push";
import { Card, SettingsPage } from "./SettingsView";

type Device = { id: string; name: string; created_at: number; expires_at: number; last_seen_at: number | null; push: boolean; current: boolean };
type Pair = { url: string; svg: string; expires_at: number; warning: string | null };

export function DevicesView() {
  const qc = useQueryClient();
  const devices = useQuery({ queryKey: ["devices"], queryFn: () => api.get<Device[]>("/api/devices"), refetchInterval: 5000 });
  const [pair, setPair] = useState<Pair | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [now, setNow] = useState(Date.now());
  useEffect(() => { const timer = setInterval(() => setNow(Date.now()), 1000); return () => clearInterval(timer); }, []);
  const scroll = useRef<HTMLDivElement>(null);
  const rows = devices.data ?? [];
  const virtualizer = useVirtualizer({ count: rows.length, getScrollElement: () => scroll.current, estimateSize: () => 72, useFlushSync: false });
  const createPair = async () => {
    setBusy("pair");
    try { setPair(await api.post<Pair>("/api/devices/pair")); } catch (error) { toastError(error); }
    finally { setBusy(null); }
  };
  const revoke = useCallback(async (device: Device) => {
    if (isDesktopShell && device.current) return;
    setBusy(device.id);
    try {
      await api.delete(`/api/devices/${device.id}`);
      if (device.current) { location.replace("/auth"); return; }
      await qc.invalidateQueries({ queryKey: ["devices"] });
    } catch (error) { toastError(error); }
    finally { setBusy(null); }
  }, [qc]);
  const remaining = pair ? Math.max(0, Math.ceil((pair.expires_at * 1000 - now) / 1000)) : 0;
  return <SettingsPage>
    <NotificationsCard />
    <Card title="Pair a device" end={<button className="btn btn-sm btn-primary" disabled={busy !== null} onClick={() => void createPair()}>Create pairing link</button>}>
      <p className="text-sm text-ink-2">Scan the QR code with your phone, or open the link in another browser. Give that browser a name to finish pairing. Each link works once and expires after five minutes.</p>
      <ReversePairing disabled={busy !== null} />
      <HttpsSetup disabled={busy !== null} onBusy={(active) => setBusy(active ? "https" : null)} onPair={setPair} />
      {pair?.warning && <p className="text-sm text-attn" role="status">{pair.warning}</p>}
      {pair && (remaining > 0 ? <div className="grid justify-items-start gap-2">
        <img alt="Scan to pair Launchpad" className="h-64 w-64 max-w-full" src={`data:image/svg+xml;base64,${btoa(pair.svg)}`} />
        <div className="grid w-full min-w-0 gap-1">
          <span className="text-xs text-ink-3">Pairing URL</span>
          <div className="flex min-w-0 flex-wrap items-start gap-2">
            <a className="min-w-0 max-w-full flex-1 basis-[240px] break-all font-mono text-xs text-teal-ink underline" href={pair.url} target="_blank" rel="noopener noreferrer">{pair.url}</a>
            <button className="btn btn-sm shrink-0" onClick={() => void copyText(pair.url).then(() => toast("Pairing URL copied")).catch(toastError)}>Copy URL</button>
          </div>
        </div>
        <span className="text-xs text-ink-3" role="status">Expires in {Math.floor(remaining / 60)}:{String(remaining % 60).padStart(2, "0")} · single use</span>
      </div> : <p className="text-sm text-ink-3" role="status">Pairing link expired. Create a new link.</p>)}
    </Card>
    <Card title="Paired browsers">
      <p className="text-sm text-ink-2">Revoke signs out that browser and disconnects its event and terminal viewers. Agents and terminals keep running on the host.</p>
      {devices.error && <p className="text-sm text-attn" role="alert">{devices.error.message}</p>}
      {!devices.data ? <p className="text-sm text-ink-3">Loading…</p> : !rows.length ? <p className="text-sm text-ink-3">No paired browsers.</p> :
        <div ref={scroll} className="overflow-y-auto" style={{ height: Math.min(rows.length * 72, 360) }}>
          <div style={{ height: virtualizer.getTotalSize(), position: "relative" }}>
            {virtualizer.getVirtualItems().map((item) => {
              const device = rows[item.index]!;
              return <DeviceRow key={device.id} device={device} height={item.size} top={item.start} busy={busy !== null} onRevoke={revoke} />;
            })}
          </div>
        </div>}
    </Card>
  </SettingsPage>;
}

function ReversePairing({ disabled }: { disabled: boolean }) {
  const [code, setCode] = useState("");
  const [request, setRequest] = useState<{ code: string; name: string } | null>(null);
  const [busy, setBusy] = useState(false);
  const qc = useQueryClient();
  const check = async () => {
    setBusy(true);
    setRequest(null);
    try {
      const normalized = code.replace(/[\s-]/g, "").toUpperCase();
      const result = await api.post<{ name: string }>("/api/devices/reverse/check", { code: normalized });
      setRequest({ code: normalized, name: result.name });
    } catch (error) { toastError(error); }
    finally { setBusy(false); }
  };
  const approve = async () => {
    if (!request) return;
    setBusy(true);
    try {
      await api.post("/api/devices/reverse/approve", { code: request.code });
      setRequest(null); setCode("");
      toast("Approved. The waiting browser will connect shortly.");
      await qc.invalidateQueries({ queryKey: ["devices"] });
    } catch (error) { setRequest(null); toastError(error); }
    finally { setBusy(false); }
  };
  return <div className="grid gap-2 rounded border border-line p-3">
    <p className="text-sm font-medium">Pair with a code from another browser</p>
    <p className="text-xs text-ink-2">On the unpaired browser, choose Show pairing code. Enter its eight-character code here.</p>
    <form className="flex flex-wrap items-end gap-2" onSubmit={(event) => { event.preventDefault(); void check(); }}>
      <label className="grid gap-1 text-xs">Pairing code<input className="field-input w-40 font-mono uppercase" value={code} maxLength={12} autoComplete="off" autoCapitalize="characters" spellCheck={false} placeholder="ABCD 2345" required disabled={disabled || busy} onChange={(event) => { setCode(event.target.value); setRequest(null); }} /></label>
      <button className="btn btn-sm" disabled={disabled || busy}>Check code</button>
    </form>
    {request && <div className="grid gap-2" role="status">
      <p className="text-sm">Allow <strong>{request.name}</strong> with code <span className="font-mono">{request.code}</span> full access to Launchpad?</p>
      <p className="text-xs text-ink-2">Only approve a code you can see on your own device. The browser supplies its name; it is not proof of identity.</p>
      <div className="flex gap-2"><button className="btn btn-sm btn-primary" disabled={disabled || busy} onClick={() => void approve()}>Approve browser</button><button className="btn btn-sm" disabled={busy} onClick={() => setRequest(null)}>Cancel</button></div>
    </div>}
  </div>;
}

type PairingInfo = { origin: string; daemon_port: number; https_port: number | null; installed: boolean; connected: boolean };
type HttpsResult = { pair: Pair | null; output: string; approval_url: string | null };

function HttpsSetup({ disabled, onBusy, onPair }: { disabled: boolean; onBusy: (busy: boolean) => void; onPair: (pair: Pair) => void }) {
  const qc = useQueryClient();
  const info = useQuery({ queryKey: ["pairing-info"], queryFn: () => api.get<PairingInfo>("/api/devices/pairing-info"), staleTime: 30_000 });
  const [port, setPort] = useState<string | null>(null);
  const [working, setWorking] = useState<"run" | "check" | null>(null);
  const [result, setResult] = useState<HttpsResult | null>(null);
  const data = info.data;
  if (!data?.installed) return null;
  const value = port ?? (data.https_port?.toString() ?? "");
  const httpsPort = Number(value);
  const valid = /^\d+$/.test(value) && Number.isInteger(httpsPort) && httpsPort >= 1 && httpsPort <= 65535;
  const command = valid ? `tailscale serve --bg --https=${httpsPort} --set-path=/ http://127.0.0.1:${data.daemon_port}` : null;
  const run = async (action: "run" | "check") => {
    onBusy(true);
    setWorking(action);
    try {
      const res = await api.post<HttpsResult>(`/api/devices/https/${action}`, { port: httpsPort });
      setResult(res);
      if (res.pair) { onPair(res.pair); toast("HTTPS pairing URL updated"); }
      await Promise.all([qc.invalidateQueries({ queryKey: ["pairing-info"] }), qc.invalidateQueries({ queryKey: ["config"] })]);
    } catch (error) { setResult({ pair: null, output: error instanceof Error ? error.message : String(error), approval_url: null }); }
    finally { setWorking(null); onBusy(false); }
  };
  const controls = <div className="grid min-w-0 gap-2 text-sm">
    <p>Use Tailscale Serve to pair over HTTPS. Tailscale manages the certificate and keeps the connection running in the background. Both devices must be signed in to your tailnet.</p>
    {!data.connected && <p className="text-attn">Sign in using the Tailscale app on this machine, then refresh.</p>}
    <label className="flex flex-wrap items-center gap-2">HTTPS port <input className="field-input w-24 font-mono text-xs" type="number" inputMode="numeric" min={1} max={65535} placeholder="443" value={value} disabled={disabled} onChange={(e) => { setPort(e.target.value); setResult(null); }} /></label>
    <p className="text-xs text-ink-3">{data.https_port ? `Suggested port: ${data.https_port}. Existing routes are kept. This forwards to Launchpad on port ${data.daemon_port}.` : "Choose an unused HTTPS port from 1 to 65535."}</p>
    {command && <code className="min-w-0 break-all rounded bg-chrome p-2 text-xs">{command}</code>}
    <div className="flex flex-wrap gap-2">
      <button className="btn btn-sm btn-primary" disabled={disabled || !valid || !data.connected} onClick={() => void run("run")}>{working === "run" ? "Running…" : "Run command"}</button>
      <button className="btn btn-sm" disabled={disabled || !command} onClick={() => command && void copyText(command).then(() => toast("HTTPS setup command copied")).catch(toastError)}>Copy command</button>
      <button className="btn btn-sm" disabled={disabled || !valid} onClick={() => void run("check")}>{working === "check" ? "Checking…" : "Update HTTPS URL"}</button>
      {!data.connected && <button className="btn btn-sm" disabled={disabled} onClick={() => void info.refetch()}>Refresh</button>}
    </div>
    <p className="text-xs text-ink-3">Run command creates a private HTTPS route for this Launchpad. Once ready, the QR code and URL update automatically.</p>
    {result && <div className="grid min-w-0 gap-2" role="status">
      <pre className="max-h-48 overflow-auto whitespace-pre-wrap break-all rounded bg-chrome p-2 text-xs">{result.output}</pre>
      {result.approval_url && <a className="justify-self-start text-teal-ink underline" {...externalLinkProps(result.approval_url)}>Approve HTTPS in Tailscale</a>}
      {!result.pair && result.approval_url && <p className="text-xs text-ink-3">After approving HTTPS, choose Update HTTPS URL. If setup still needs to run, click Run command again.</p>}
    </div>}
    <a className="justify-self-start text-xs text-teal-ink underline" {...externalLinkProps("https://tailscale.com/docs/reference/tailscale-cli/serve")}>Tailscale Serve help</a>
  </div>;
  return <div className="min-w-0 rounded-lg border border-line-2 p-3">
    {data.origin.startsWith("https://") ? <><p className="text-sm">Pairing uses HTTPS: <span className="break-all font-mono text-xs">{data.origin}</span></p><details className="mt-2" open={!!result || port !== null}><summary className="cursor-pointer text-xs text-teal-ink">Tailscale HTTPS settings</summary><div className="mt-2">{controls}</div></details></> : controls}
  </div>;
}

const DeviceRow = memo(function DeviceRow({ device, height, top, busy, onRevoke }: { device: Device; height: number; top: number; busy: boolean; onRevoke: (device: Device) => Promise<void> }) {
  return <div className="absolute left-0 top-0 flex w-full items-center gap-3 border-b border-line-2" style={{ height, transform: `translateY(${top}px)` }}>
    <div className="min-w-0 flex-1"><div className="truncate text-sm font-semibold">{device.name}{device.current && <span className="ml-2 text-xs font-normal text-ink-3">{isDesktopShell ? "This desktop app" : "This browser"}</span>}</div>
      <div className="truncate text-xs text-ink-3">
        {device.last_seen_at ? `Last active ${lastActive(device.last_seen_at)}` : "Not active since pairing"} · notifications {device.push ? "on" : "off"} · paired {new Date(device.created_at * 1000).toLocaleDateString()} · expires {new Date(device.expires_at * 1000).toLocaleDateString()}
      </div></div>
    {isDesktopShell && device.current ? <span className="text-xs text-ink-3">Local desktop app</span> : <button className="btn btn-sm" disabled={busy} onClick={() => void onRevoke(device)}>{device.current ? "Sign out" : "Revoke"}</button>}
  </div>;
});

/** Activity is recorded at most every five minutes. */
function lastActive(unixSeconds: number) {
  const ms = unixSeconds * 1000;
  if (Date.now() - ms < 5 * 60_000) return "in the last 5 min";
  const age = formatAge(ms);
  return age === "yesterday" ? age : `${age} ago`;
}

const UNAVAILABLE: Record<Exclude<PushState["support"], "supported">, string> = {
  desktop: "The desktop app shows notifications through the host's notification center. Use a paired browser or an installed web app on another device for push notifications.",
  "ios-home-screen": "On iPhone and iPad, notifications work only in the installed web app: open Share → Add to Home Screen, start Launchpad from the Home Screen, and come back to this page.",
  insecure: "Notifications need an HTTPS address or http://127.0.0.1.",
  unsupported: "This browser does not support push notifications.",
};

/** Web Push for this browser: completed turns, automatic stops and automatic quota resumption. */
function NotificationsCard() {
  const state = usePushState();
  const refresh = useRefreshPush();
  const [busy, setBusy] = useState(false);
  // Chosen before turning notifications on, or saved in this browser's subscription.
  const [titles, setTitles] = useState<boolean | null>(null);
  const run = async (action: () => Promise<unknown>, done?: string) => {
    setBusy(true);
    try { await action(); if (done) toast(done); } catch (error) { toastError(error); }
    finally { setBusy(false); await refresh(); }
  };
  const push = state.data;
  const on = !!push?.subscribed;
  const showTitles = on ? push.titles : titles ?? false;
  return <Card title="Notifications on this browser" end={push?.support === "supported" && (on
    ? <button className="btn btn-sm" disabled={busy} onClick={() => void run(disablePush, "Notifications turned off on this browser")}>Turn off</button>
    : <button className="btn btn-sm btn-primary" disabled={busy || push.permission === "denied"} onClick={() => void run(() => enablePush(showTitles), "Notifications turned on on this browser")}>Turn on</button>)}>
    <p className="text-sm text-ink-2">Get a notification when an agent finishes, fails, stops automatically, or resumes after quota becomes available, and no window in use shows it. Launches, questions and manual stops stay silent. Notifications show the status and the project; never code or messages.</p>
    {state.error && <p className="text-sm text-attn" role="alert">{state.error.message}</p>}
    {!push ? <p className="text-sm text-ink-3">Loading…</p> : push.support !== "supported" ? <p className="text-sm text-ink-3" role="status">{UNAVAILABLE[push.support]}</p> : <>
      <p className="text-sm" role="status">
        {on ? "On for this browser." : push.permission === "denied" ? "Blocked: allow notifications for this site in the browser's site settings, then turn them on here." : "Off for this browser."}
      </p>
      <label className="flex items-center gap-2 text-sm">
        <input type="checkbox" checked={showTitles} disabled={busy} onChange={(e) => { const value = e.target.checked; setTitles(value); if (on) void run(() => setPushTitles(value)); }} />
        Show session titles on the lock screen
      </label>
      <div><button className="btn btn-sm" disabled={busy || !on} onClick={() => void run(async () => { await sendTestPush(); }, "Test notification sent")}>Send a test notification</button></div>
    </>}
  </Card>;
}
