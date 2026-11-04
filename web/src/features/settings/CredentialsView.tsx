// Settings → Credentials and tools: Home's vault (harness logins, tokens, the SSH key), the tool registry, the Git
// identity of agents' commits and the active session leases. Secrets are write-only: the page never reads them.

import { useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import type { LeaseInfo } from "@/gen/LeaseInfo";
import type { ProxyAuth } from "@/gen/ProxyAuth";
import type { Tool } from "@/gen/Tool";
import type { ToolInstallKind } from "@/gen/ToolInstallKind";
import type { VaultKind } from "@/gen/VaultKind";
import type { VaultOverview } from "@/gen/VaultOverview";
import { toast, toastError } from "@/components/Toast";
import { Badge, ConfirmButton, Empty, ErrorState, FilterChip, FilterChips, Loading, SettingRow, Token } from "@/components/ui";
import { api } from "@/lib/api";
import { formatAge } from "@/lib/format";
import { useSessions } from "@/lib/queries";
import { useWorkspaces } from "@/lib/workspace";
import { Card, SettingsPage } from "./SettingsView";

const slug = (s: string) => s.toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-|-$/g, "").slice(0, 60);

export function useVault() {
  return useQuery({ queryKey: ["vault"], queryFn: () => api.get<VaultOverview>("/api/vault"), staleTime: 30_000 });
}

export function CredentialsView() {
  const vault = useVault();
  return (
    <SettingsPage title="Credentials and tools" intro="Home keeps these and lends them to each session through its runner's proxy: agents only see placeholders. What an agent may do is decided by each token's scopes. Secrets are never shown again.">
      {vault.isLoading ? <Loading what="credentials" /> : vault.error ? <ErrorState error={vault.error} /> : vault.data && <Vault v={vault.data} />}
    </SettingsPage>
  );
}

function Vault({ v }: { v: VaultOverview }) {
  const qc = useQueryClient();
  const refresh = () => qc.invalidateQueries({ queryKey: ["vault"] });
  const others = v.entries.filter((e) => e.kind === "token" || e.kind === "ssh_key");
  return (
    <>
      <Card title="Harness logins">
        {v.logins.map((l) => {
          const mine = v.entries.filter((e) => e.kind === l.kind);
          return (
            <div key={l.harness} className="grid gap-1.5 border-b border-line-2 pb-2.5 last:border-b-0 last:pb-0">
              <div className="text-[13px] font-semibold">{l.name}</div>
              {mine.length === 0 && <span className="text-[12px] text-ink-3">No login yet: {l.name} sessions cannot start.</span>}
              {mine.map((e, i) => <EntryRow key={e.id} id={e.id} name={e.name} detail={e.detail} note={i === 0 ? "default" : undefined} updatedAt={e.updated_at} onDone={refresh} />)}
              <AddSecret label={`Add a ${l.name} login`} kind={l.kind} hint={l.how} multiline={l.kind === "codex_login"} onDone={refresh} />
            </div>
          );
        })}
      </Card>
      <Card title="Tokens and SSH key">
        {others.length === 0 && <Empty icon="info">No token yet. Add the tokens your tools use, for example a fine-grained GitHub token.</Empty>}
        {others.map((e) => <EntryRow key={e.id} id={e.id} name={e.name} detail={`${e.kind === "ssh_key" ? "SSH key · " : ""}${e.detail}`} updatedAt={e.updated_at} onDone={refresh} />)}
        <div className="flex flex-wrap gap-2">
          <AddSecret label="Add a token" kind="token" hint="A token or API key. Tools reference it by its ID." onDone={refresh} />
          <AddSecret label="Add the SSH key" kind="ssh_key" hint="A private key without a passphrase. It never leaves Home: sessions use it through a forwarded ssh-agent." multiline onDone={refresh} />
        </div>
      </Card>
      <Tools v={v} onDone={refresh} />
      <Identity v={v} onDone={refresh} />
      <Leases />
    </>
  );
}

function EntryRow({ id, name, detail, note, updatedAt, onDone }: { id: string; name: string; detail: string; note?: string; updatedAt: number; onDone: () => unknown }) {
  return (
    <div className="flex flex-wrap items-center gap-2 text-[13px]">
      <span className="min-w-0 flex-1"><b>{name}</b> <span className="font-mono text-[12px] text-ink-3">{id}</span> <span className="text-[12px] text-ink-3">· {detail} · updated {formatAge(updatedAt)} ago</span></span>
      {note && <Badge tone="teal">{note}</Badge>}
      <ConfirmButton icon="trash" consequence={`Removes ${name} from Home. Sessions that use it fail to start or lose it at their next lease renewal.`} confirmLabel="Remove" onConfirm={() => api.delete(`/api/vault/entries/${id}`).then(onDone, toastError)}>Remove</ConfirmButton>
    </div>
  );
}

function AddSecret({ label, kind, hint, multiline, onDone }: { label: string; kind: VaultKind; hint: string; multiline?: boolean; onDone: () => unknown }) {
  const [open, setOpen] = useState(false);
  const [name, setName] = useState("");
  const [id, setId] = useState("");
  const [value, setValue] = useState("");
  const [busy, setBusy] = useState(false);
  if (!open) return <button className="btn btn-sm w-fit" onClick={() => setOpen(true)}>{label}</button>;
  const save = () => {
    setBusy(true);
    void api.put(`/api/vault/entries/${id || slug(name)}`, { kind, name, value }).then(() => { toast("Saved on Home"); setOpen(false); setName(""); setId(""); setValue(""); return onDone(); }, toastError).finally(() => setBusy(false));
  };
  return (
    <div className="grid gap-2 rounded-[8px] border border-line-2 p-2.5 text-[13px]">
      <span className="text-[12px] text-ink-3">{hint}</span>
      <div className="flex flex-wrap gap-2">
        <label className="grid gap-1"><span className="text-ink-2">Name</span><input className="field-input" value={name} onChange={(e) => setName(e.target.value)} placeholder={kind === "token" ? "GitHub" : "Personal"} /></label>
        <label className="grid gap-1"><span className="text-ink-2">ID</span><input className="field-input font-mono" value={id || slug(name)} onChange={(e) => setId(slug(e.target.value))} placeholder="github" /></label>
      </div>
      <label className="grid gap-1"><span className="text-ink-2">Secret</span>
        {multiline ? <textarea className="field-input min-h-[90px] font-mono text-[12px]" value={value} onChange={(e) => setValue(e.target.value)} autoComplete="off" spellCheck={false} /> : <input className="field-input font-mono" type="password" value={value} onChange={(e) => setValue(e.target.value)} autoComplete="off" />}
      </label>
      <div className="flex gap-2"><button className="btn btn-sm btn-primary" disabled={busy || !value.trim() || !(id || slug(name))} onClick={save}>Save on Home</button><button className="btn btn-sm" onClick={() => setOpen(false)}>Cancel</button></div>
    </div>
  );
}

const AUTH_LABEL: Record<ProxyAuth["kind"], string> = { bearer: "Bearer header", basic: "Basic auth", header: "Custom header", query: "Query parameter" };

function authText(a: ProxyAuth): string {
  switch (a.kind) {
    case "basic": return `basic ${a.user}`;
    case "header": return `header ${a.name}${a.prefix ? ` ${a.prefix}` : ""}`;
    case "query": return `query ${a.param}`;
    default: return "bearer";
  }
}

/** One host rule per line: `<host> bearer`, `<host> basic <user>`, `<host> header <name> [prefix]`, `<host> query <param>`. */
export function parseHosts(text: string): { host: string; auth: ProxyAuth }[] {
  return text.split("\n").map((l) => l.trim()).filter(Boolean).map((line) => {
    const [host, kind = "bearer", a = "", ...rest] = line.split(/\s+/);
    const auth: ProxyAuth = kind === "basic" ? { kind: "basic", user: a || "x-access-token" } : kind === "header" ? { kind: "header", name: a, prefix: rest.join(" ") } : kind === "query" ? { kind: "query", param: a } : { kind: "bearer" };
    return { host, auth };
  });
}

function installText(t: Tool): string {
  const i = t.install;
  if (!i) return "no install";
  return i.kind === "github_release" ? `GitHub release ${i.source} (${i.asset})` : `${i.kind === "url" ? "download" : i.kind} ${i.source}`;
}

function Tools({ v, onDone }: { v: VaultOverview; onDone: () => unknown }) {
  const [editing, setEditing] = useState<Tool | null>(null);
  const workspaces = useWorkspaces().data ?? [];
  const wsName = (id: string) => (id === "*" ? "every workspace" : workspaces.find((w) => w.id === id)?.name ?? id);
  const exportToml = () => void api.get<{ toml: string }>("/api/vault/tools.toml").then((r) => navigator.clipboard.writeText(r.toml)).then(() => toast("Copied the tools as TOML"), toastError);
  return (
    <Card title="Tools" end={<button className="btn btn-sm" onClick={exportToml}>Copy as TOML</button>}>
      <span className="text-[12px] text-ink-3">Runners install each granted tool in their own folder, on the agent's PATH. Its variables hold a placeholder; the proxy adds the token on its hosts.</span>
      {v.tools.length === 0 && <Empty icon="term">No tool yet. Start from GitHub CLI or Hugging Face CLI below.</Empty>}
      {v.tools.map((t) => (
        <div key={t.id} className="flex flex-wrap items-center gap-2 text-[13px]">
          <span className="grid min-w-0 flex-1 gap-0.5">
            <span className="flex flex-wrap items-center gap-1.5"><b>{t.name}</b><span className="font-mono text-[12px] text-ink-3">{t.id}</span>{t.credential && !v.entries.some((e) => e.id === t.credential) && <Badge tone="attn">token {t.credential} missing</Badge>}</span>
            <span className="text-[12px] text-ink-3">{installText(t)} · {t.env.join(", ") || "no variable"} · {t.hosts.map((h) => h.host).join(", ") || "no host"} · {t.workspaces.length ? t.workspaces.map(wsName).join(", ") : "not granted"}</span>
          </span>
          <button className="btn btn-sm" onClick={() => setEditing(t)}>Edit</button>
          <ConfirmButton icon="trash" consequence={`Removes ${t.name}. New sessions no longer get it.`} confirmLabel="Remove" onConfirm={() => api.delete(`/api/vault/tools/${t.id}`).then(onDone, toastError)}>Remove</ConfirmButton>
        </div>
      ))}
      {editing ? <ToolEditor tool={editing} credentials={v.entries.filter((e) => e.kind === "token").map((e) => e.id)} onClose={() => setEditing(null)} onDone={onDone} /> : (
        <div className="flex flex-wrap gap-2">
          {v.templates.filter((t) => !v.tools.some((x) => x.id === t.id)).map((t) => <button key={t.id} className="btn btn-sm" onClick={() => setEditing(t)}>Add {t.name}</button>)}
          <button className="btn btn-sm" onClick={() => setEditing({ id: "", name: "", install: null, credential: null, env: [], hosts: [], workspaces: ["*"] })}>Add another tool</button>
        </div>
      )}
    </Card>
  );
}

function ToolEditor({ tool, credentials, onClose, onDone }: { tool: Tool; credentials: string[]; onClose: () => void; onDone: () => unknown }) {
  const workspaces = useWorkspaces().data ?? [];
  const [t, setT] = useState<Tool>(tool);
  const [hosts, setHosts] = useState(tool.hosts.map((h) => `${h.host} ${authText(h.auth)}`).join("\n"));
  const [busy, setBusy] = useState(false);
  const set = (patch: Partial<Tool>) => setT((x) => ({ ...x, ...patch }));
  const id = t.id || slug(t.name);
  const save = () => {
    setBusy(true);
    const body: Tool = { ...t, id, hosts: parseHosts(hosts) };
    void api.put(`/api/vault/tools/${id}`, body).then(() => { toast(`Saved ${t.name}`); onClose(); return onDone(); }, toastError).finally(() => setBusy(false));
  };
  const all = t.workspaces.includes("*");
  const toggleWs = (w: string) => set({ workspaces: t.workspaces.includes(w) ? t.workspaces.filter((x) => x !== w) : [...t.workspaces.filter((x) => x !== "*"), w] });
  const kinds: [ToolInstallKind | "none", string][] = [["none", "Nothing to install"], ["github_release", "GitHub release"], ["url", "Download URL"], ["cargo", "cargo install"], ["uv", "uv tool install"], ["npm", "npm package"]];
  return (
    <div className="grid gap-1 rounded-[8px] border border-line-2 px-3 text-[13px]">
      <SettingRow label="Name"><input className="field-input" value={t.name} onChange={(e) => set({ name: e.target.value })} /><input className="field-input w-[160px] font-mono" value={id} onChange={(e) => set({ id: slug(e.target.value) })} disabled={!!tool.id && tool.name !== ""} aria-label="ID" /></SettingRow>
      <SettingRow label="Install" hint="Runs on each runner, in its own folder. {arch} becomes amd64 or arm64.">
        <select className="field-input" value={t.install?.kind ?? "none"} onChange={(e) => set({ install: e.target.value === "none" ? null : { kind: e.target.value as ToolInstallKind, source: t.install?.source ?? "", asset: t.install?.asset ?? "" } })}>
          {kinds.map(([k, label]) => <option key={k} value={k}>{label}</option>)}
        </select>
        {t.install && <input className="field-input min-w-[220px] flex-1 font-mono" value={t.install.source} placeholder={t.install.kind === "github_release" ? "owner/repo" : t.install.kind === "url" ? "https://…" : "package"} onChange={(e) => set({ install: { ...t.install!, source: e.target.value.trim() } })} />}
        {t.install?.kind === "github_release" && <input className="field-input min-w-[200px] font-mono" value={t.install.asset} placeholder="tool_*_linux_{arch}.tar.gz" onChange={(e) => set({ install: { ...t.install!, asset: e.target.value.trim() } })} />}
      </SettingRow>
      <SettingRow label="Token" hint="The vault entry the proxy adds on the hosts below.">
        <select className="field-input" value={t.credential ?? ""} onChange={(e) => set({ credential: e.target.value || null })}>
          <option value="">None</option>
          {[...new Set([...credentials, ...(t.credential ? [t.credential] : [])])].map((c) => <option key={c} value={c}>{c}{credentials.includes(c) ? "" : " (not added yet)"}</option>)}
        </select>
      </SettingRow>
      <SettingRow label="Variables" hint="Set to a placeholder in the agent's environment, for example SLACK_TOKEN."><input className="field-input min-w-[260px] flex-1 font-mono" value={t.env.join(", ")} onChange={(e) => set({ env: e.target.value.split(/[\s,]+/).filter(Boolean) })} /></SettingRow>
      <SettingRow label="Hosts" hint={<>One per line: <code className="font-mono">host bearer</code>, <code className="font-mono">host basic user</code>, <code className="font-mono">host header Name [prefix]</code> or <code className="font-mono">host query param</code>. <code className="font-mono">*.example.com</code> covers subdomains.</>}>
        <textarea className="field-input min-h-[70px] min-w-[300px] flex-1 font-mono text-[12px]" value={hosts} onChange={(e) => setHosts(e.target.value)} spellCheck={false} />
        <span className="text-[11px] text-ink-3">{parseHosts(hosts).map((h) => `${h.host}: ${AUTH_LABEL[h.auth.kind]}`).join(" · ")}</span>
      </SettingRow>
      <SettingRow label="Granted in" hint="Sessions of these workspaces get the tool and its token.">
        <FilterChips label="Granted in">
          <FilterChip active={all} onClick={() => set({ workspaces: all ? [] : ["*"] })}>Every workspace</FilterChip>
          {workspaces.map((w) => <FilterChip key={w.id} active={all || t.workspaces.includes(w.id)} onClick={() => toggleWs(w.id)}>{w.name}</FilterChip>)}
        </FilterChips>
      </SettingRow>
      <div className="flex gap-2 py-2.5"><button className="btn btn-sm btn-primary" disabled={busy || !t.name.trim() || !id} onClick={save}>Save tool</button><button className="btn btn-sm" onClick={onClose}>Cancel</button></div>
    </div>
  );
}

function Identity({ v, onDone }: { v: VaultOverview; onDone: () => unknown }) {
  const [name, setName] = useState(v.git_name);
  const [email, setEmail] = useState(v.git_email);
  const changed = name !== v.git_name || email !== v.git_email;
  return (
    <Card title="Git identity" end={changed ? <button className="btn btn-sm btn-primary" onClick={() => void api.put("/api/vault/identity", { name, email }).then(() => { toast("Saved"); return onDone(); }, toastError)}>Save</button> : undefined}>
      <span className="text-[12px] text-ink-3">The author of agents' commits on every runner. Runners never read the machine's Git config.</span>
      <div className="flex flex-wrap gap-2 text-[13px]">
        <label className="grid gap-1"><span className="text-ink-2">Name</span><input className="field-input" value={name} onChange={(e) => setName(e.target.value)} /></label>
        <label className="grid gap-1"><span className="text-ink-2">Email</span><input className="field-input" value={email} onChange={(e) => setEmail(e.target.value)} /></label>
      </div>
    </Card>
  );
}

function Leases() {
  const qc = useQueryClient();
  const leases = useQuery({ queryKey: ["vault-leases"], queryFn: () => api.get<LeaseInfo[]>("/api/vault/leases"), refetchInterval: 30_000 });
  const sessions = useSessions().data ?? [];
  const active = (leases.data ?? []).filter((l) => !l.revoked_at);
  return (
    <Card title={`Active leases · ${active.length}`}>
      {active.length === 0 ? <Empty icon="clock">No session holds credentials now.</Empty> : active.map((l) => <LeaseRow key={l.id} lease={l} title={sessions.find((s) => s.id === l.session_id)?.title ?? l.session_id} onDone={() => qc.invalidateQueries({ queryKey: ["vault-leases"] })} />)}
    </Card>
  );
}

export function LeaseRow({ lease, title, onDone }: { lease: LeaseInfo; title?: string; onDone: () => unknown }) {
  return (
    <div className="flex flex-wrap items-center gap-2 text-[13px]">
      <span className="grid min-w-0 flex-1 gap-0.5">
        {title && <span className="truncate font-semibold">{title}</span>}
        <span className="flex flex-wrap items-center gap-1">{lease.credentials.map((c) => <Token key={c}>{c}</Token>)}{lease.tools.map((t) => <Token key={t} tone="teal">{t}</Token>)}<span className="text-[12px] text-ink-3">since {formatAge(lease.created_at)} ago · renewed {formatAge(lease.renewed_at)} ago</span></span>
      </span>
      <ConfirmButton consequence="Stops adding these tokens to the session's requests at once (while its runner is connected). The agent keeps running without them." confirmLabel="Revoke" onConfirm={() => api.post(`/api/vault/leases/${lease.id}/revoke`, {}).then(() => { toast("Revoked"); return onDone(); }, toastError)}>Revoke</ConfirmButton>
    </div>
  );
}
