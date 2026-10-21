// Settings → Hosts: machines reached with the system `ssh` where projects live and agents run. Adding a host
// installs only the Launchpad helper; preflight is read-only and each failed check names the command to run,
// which the owner can start in a visible terminal on the host.

import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import type { HostStatus } from "@/gen/HostStatus";
import type { SshConfigHost } from "@/gen/SshConfigHost";
import { HarnessMark, useHarnesses } from "@/components/HarnessMark";
import { Icon } from "@/components/Icon";
import { toast, toastError } from "@/components/Toast";
import { api } from "@/lib/api";
import { formatAge } from "@/lib/format";
import { linkLabel, useHosts } from "@/lib/hosts";
import { useProjects, useSessions, useStatus } from "@/lib/queries";
import { useOpenTerminal } from "@/features/terminals/lib";
import { Card, SettingsPage } from "./SettingsView";

const tone: Record<HostStatus["state"], string> = {
  connected: "text-gh-open",
  connecting: "text-attn",
  unreachable: "text-gh-closed",
  needs_setup: "text-gh-closed",
};

function copy(text: string) {
  navigator.clipboard.writeText(text).then(() => toast("Command copied"), toastError);
}

function AddHost({ onDone }: { onDone: () => void }) {
  const config = useQuery({ queryKey: ["ssh-config"], queryFn: () => api.get<SshConfigHost[]>("/api/hosts/ssh-config"), staleTime: 60_000 });
  const hosts = useHosts();
  const qc = useQueryClient();
  const [destination, setDestination] = useState("");
  const [id, setId] = useState("");
  const [port, setPort] = useState("");
  const [root, setRoot] = useState("~/projects");
  const [busy, setBusy] = useState(false);
  const known = new Set(hosts.map((h) => h.host.destination));
  const add = async () => {
    setBusy(true);
    try {
      await api.post("/api/hosts", { destination: destination.trim(), id: id.trim() || null, port: port.trim() ? Number(port) : null, projects_root: root.trim() || null });
      await qc.invalidateQueries({ queryKey: ["status"] });
      toast("Host added. Launchpad connects and installs its helper.");
      onDone();
    } catch (e) {
      toastError(e);
    } finally {
      setBusy(false);
    }
  };
  return (
    <div className="grid gap-2.5 rounded-md border border-line p-3 text-[12.5px]">
      <b className="font-[650]">Add host</b>
      {(config.data ?? []).length > 0 && (
        <div className="grid gap-1">
          <span className="text-ink-2">From ~/.ssh/config (jump hosts, certificates and agent keys work as in a terminal):</span>
          <div className="flex flex-wrap gap-1.5">
            {config.data!.map((c) => (
              <button key={c.alias} className={`btn btn-sm ${destination === c.alias ? "bg-select!" : ""}`} disabled={known.has(c.alias)} title={[c.user && `${c.user}@`, c.hostname, c.proxy_jump && ` via ${c.proxy_jump}`].filter(Boolean).join("") || c.alias} onClick={() => setDestination(c.alias)}>
                {c.alias}{c.proxy_jump && <span className="text-ink-3">via {c.proxy_jump}</span>}
              </button>
            ))}
          </div>
        </div>
      )}
      <label className="grid gap-1">
        <span className="text-ink-2">SSH destination: an alias or user@host</span>
        <input className="field-input" value={destination} placeholder="devbox or wauplin@devbox.lan" onChange={(e) => setDestination(e.target.value)} />
      </label>
      <div className="grid grid-cols-[1fr_90px_1fr] gap-2">
        <label className="grid gap-1"><span className="text-ink-2">Name (optional)</span><input className="field-input" value={id} placeholder="from the destination" onChange={(e) => setId(e.target.value)} /></label>
        <label className="grid gap-1"><span className="text-ink-2">Port</span><input className="field-input" value={port} inputMode="numeric" placeholder="22" onChange={(e) => setPort(e.target.value.replace(/\D/g, ""))} /></label>
        <label className="grid gap-1"><span className="text-ink-2">Projects root on the host</span><input className="field-input" value={root} onChange={(e) => setRoot(e.target.value)} /></label>
      </div>
      <p className="text-xs text-ink-3">Launchpad uses the system ssh with your configuration. Password, passphrase and host key questions appear in a dialog; Launchpad never stores the answers. It uploads its own helper to ~/.local/share/launchpad-host and nothing else.</p>
      <div className="flex gap-1.5">
        <button className="btn btn-sm btn-primary" disabled={busy || !destination.trim()} onClick={() => void add()}>{busy ? "Adding…" : "Add host"}</button>
        <button className="btn btn-sm" onClick={onDone}>Cancel</button>
      </div>
    </div>
  );
}

function Checks({ h }: { h: HostStatus }) {
  const terminal = useOpenTerminal();
  const qc = useQueryClient();
  const [running, setRunning] = useState(false);
  const home = h.info?.home;
  const run = async () => {
    setRunning(true);
    try {
      await api.post(`/api/hosts/${h.host.id}/checks`);
      await qc.invalidateQueries({ queryKey: ["status"] });
    } catch (e) {
      toastError(e);
    } finally {
      setRunning(false);
    }
  };
  return (
    <div className="grid gap-1.5">
      <div className="flex items-center gap-2 text-xs text-ink-3">
        <b className="text-ink-2">Preflight</b>
        {h.checked_at && <span>checked {formatAge(h.checked_at)} ago</span>}
        <button className="btn btn-sm ml-auto" disabled={running || h.state !== "connected"} onClick={() => void run()}><Icon name="refresh" size={12} />{running ? "Checking…" : "Run checks again"}</button>
      </div>
      <div className="grid rounded-md border border-line">
        {h.checks.length === 0 && <div className="px-3 py-2 text-xs text-ink-3">{h.state === "connected" ? "Checking…" : "Checks run once the host is connected."}</div>}
        {h.checks.map((c) => (
          <div key={c.id} className="grid gap-1 border-b border-line-2 px-3 py-1.5 text-[12.5px] last:border-0">
            <div className="flex items-center gap-2">
              <span className={c.ok === true ? "text-gh-open" : c.ok === false ? "text-gh-closed" : "text-attn"} aria-hidden>{c.ok === true ? "✓" : c.ok === false ? "✕" : "!"}</span>
              {c.found !== null ? <HarnessMark harness={c.id} size={14} /> : null}
              <b className="font-[650]">{c.name}</b>
              <span className="min-w-0 truncate text-xs text-ink-3" title={c.detail}>{c.detail}</span>
            </div>
            {c.command && (
              <div className="flex flex-wrap items-center gap-2 pl-5 text-xs">
                <span className="text-ink-2">Run on {h.host.id}:</span>
                <code className="rounded bg-chrome px-1.5 py-0.5 font-mono">{c.command}</code>
                <button className="iconbtn" title="Copy the command" aria-label={`Copy ${c.command}`} onClick={() => copy(c.command!)}><Icon name="copy" size={12} /></button>
                {home && !c.command.startsWith("Install") && (
                  <button className="btn btn-sm" title="Runs the command in a visible terminal on the host" onClick={() => void terminal.shell(`${h.host.id}:${home}`, c.command!, "shell")}><Icon name="term" size={12} />Run in terminal on host</button>
                )}
              </div>
            )}
          </div>
        ))}
      </div>
    </div>
  );
}

function HostCard({ h }: { h: HostStatus }) {
  const projects = useProjects().data ?? [];
  const sessions = useSessions().data ?? [];
  const terminal = useOpenTerminal();
  const qc = useQueryClient();
  const [root, setRoot] = useState(h.host.projects_root);
  const mine = projects.filter((p) => p.path.startsWith(`${h.host.id}:`));
  const running = sessions.filter((s) => s.cwd.startsWith(`${h.host.id}:`) && ["starting", "working", "background"].includes(s.status)).length;
  const act = (p: Promise<unknown>, done?: string) => p.then(() => { if (done) toast(done); return qc.invalidateQueries({ queryKey: ["status"] }); }).catch(toastError);
  return (
    <div className="grid gap-2.5 rounded-md border border-line p-3">
      <div className="flex flex-wrap items-center gap-2 text-[12.5px]">
        <Icon name="term" size={14} />
        <b className="font-[650]">{h.host.id}</b>
        <span className="text-ink-3">{h.host.destination}{h.host.port ? `:${h.host.port}` : ""}</span>
        <span className={`ml-auto text-xs font-semibold ${tone[h.state]}`}>{linkLabel[h.state]}{h.latency_ms !== null && h.state === "connected" ? ` · ${h.latency_ms} ms` : ""}</span>
      </div>
      <div className="text-xs text-ink-2">
        {[h.info && `${h.info.os} ${h.info.arch}`, `${mine.length} project${mine.length === 1 ? "" : "s"}`, running > 0 && `${running} agent${running === 1 ? "" : "s"} running`, h.state !== "connected" && `since ${new Date(Number(h.since)).toLocaleTimeString()}`].filter(Boolean).join(" · ")}
      </div>
      {h.error && <div className={`text-xs ${h.state === "connecting" ? "text-ink-2" : "text-gh-closed"}`}>{h.error}</div>}
      {h.state === "unreachable" && running > 0 && <div className="text-xs text-ink-2">Agents keep running on the host; their output appears when the connection returns.</div>}
      <div className="flex flex-wrap items-end gap-2 text-xs">
        <label className="grid gap-1"><span className="text-ink-2">Projects root</span><input className="field-input w-[220px]" value={root} onChange={(e) => setRoot(e.target.value)} /></label>
        {root !== h.host.projects_root && <button className="btn btn-sm" onClick={() => void act(api.patch(`/api/hosts/${h.host.id}`, { projects_root: root }), "Projects root saved")}>Save</button>}
        <span className="ml-auto flex flex-wrap gap-1.5">
          {h.info && <button className="btn btn-sm" onClick={() => void terminal.shell(`${h.host.id}:${h.info!.home}`)}><Icon name="term" size={12} />Open terminal on host</button>}
          <button className="btn btn-sm" onClick={() => void act(api.post(`/api/hosts/${h.host.id}/retry`))}><Icon name="refresh" size={12} />{h.state === "connected" ? "Reconnect" : "Retry now"}</button>
          <button className="btn btn-sm" onClick={() => { if (confirm(`Remove ${h.host.id}? Processes Launchpad started there are stopped. Files on the host are kept.`)) void act(api.delete(`/api/hosts/${h.host.id}`), "Host removed"); }}><Icon name="trash" size={12} />Remove</button>
        </span>
      </div>
      <Checks h={h} />
    </div>
  );
}

export function HostsView() {
  const hosts = useHosts();
  const status = useStatus().data;
  const projects = useProjects().data ?? [];
  const harnesses = useHarnesses();
  const [adding, setAdding] = useState(false);
  const local = projects.filter((p) => !/^[a-z0-9][a-z0-9._-]+:\//.test(p.path)).length;
  return (
    <SettingsPage>
      <Card title="Hosts" end={!adding && <button className="btn btn-sm" onClick={() => setAdding(true)}><Icon name="plus" size={12} />Add host</button>}>
        <p className="text-[12.5px] text-ink-2">Machines reached with ssh. Agents, worktrees, terminals, Changes and the code viewer of a project run on its host; this daemon keeps the agent list, quotas, schedules and GitHub. A turn continues on the host when this computer sleeps, and its output catches up when the connection returns. Scheduled launches and crons wait while this computer sleeps.</p>
        <div className="grid gap-1 rounded-md border border-line p-3 text-[12.5px]">
          <div className="flex items-center gap-2"><Icon name="folder" size={14} /><b className="font-[650]">This computer</b><span className="text-ink-3">{status?.host}</span><span className="ml-auto text-xs font-semibold text-gh-open">Ready</span></div>
          <div className="text-xs text-ink-2">{local} project{local === 1 ? "" : "s"} · {harnesses.map((h) => h.name).join(", ")} as configured in Settings → Profiles</div>
        </div>
        {adding && <AddHost onDone={() => setAdding(false)} />}
        {hosts.map((h) => <HostCard key={h.host.id} h={h} />)}
        {hosts.length === 0 && !adding && <div className="text-xs text-ink-3">No hosts yet. Add one from your ~/.ssh/config or as user@host.</div>}
      </Card>
    </SettingsPage>
  );
}
