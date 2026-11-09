// Settings → Credentials and tools → MCP servers: configured once on Home with their token, granted per workspace,
// each tool on or off. Agents on any runner reach them through Home's gateway; the token never leaves Home.

import { useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import type { McpServer } from "@/gen/McpServer";
import { toast, toastError } from "@/components/Toast";
import { Badge, ConfirmButton, Empty, ErrorState, Choice, Choices, Loading, MenuSection, RowActions } from "@/components/ui";
import { MenuItem } from "@/components/Popover";
import { Icon } from "@/components/Icon";
import { api } from "@/lib/api";
import { ago } from "@/lib/format";
import { useScope, useWorkspaces } from "@/lib/workspace";
import { Card } from "./SettingsView";

export function useMcpServers() {
  return useQuery({ queryKey: ["mcp"], queryFn: () => api.get<McpServer[]>("/api/mcp"), staleTime: 30_000 });
}

const slug = (s: string) => s.toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-|-$/g, "").slice(0, 40);

export function McpServers() {
  const qc = useQueryClient();
  const servers = useMcpServers();
  const [editing, setEditing] = useState<McpServer | null>(null);
  const refresh = () => qc.invalidateQueries({ queryKey: ["mcp"] });
  const workspaces = useWorkspaces().data ?? [];
  const current = useScope().workspace;
  const wsName = (id: string) => (id === "*" ? "every workspace" : workspaces.find((w) => w.id === id)?.name ?? id);
  return (
    <Card title="MCP servers" tour="settings.mcp" end={!editing ? <button className="btn btn-sm" onClick={() => setEditing({ id: "", name: "", url: "", auth: null, workspaces: current ? [current.id] : [], tools: [], tools_off: [], refreshed_at: null, error: null })}>Add MCP server</button> : undefined}>
      <span className="text-[12px] text-ink-3">Remote MCP servers (Streamable HTTP). Agents on every runner reach them through Home, which adds the token: it never reaches a runner. A tool that is on acts directly with that token. Tools start off, also those a later refresh finds: turn on only what agents may do. Read only and destructive are what the server says about its tools, not a guarantee.</span>
      {servers.isLoading ? <Loading what="MCP servers" /> : servers.error ? <ErrorState error={servers.error} /> : (servers.data ?? []).length === 0 && !editing ? <Empty icon="spark">No MCP server yet.</Empty> : null}
      {(servers.data ?? []).map((s) => <ServerRow key={s.id} s={s} wsName={wsName} onEdit={() => setEditing(s)} onDone={refresh} />)}
      {editing && <ServerForm server={editing} onClose={() => setEditing(null)} onDone={refresh} />}
    </Card>
  );
}

function ServerRow({ s, wsName, onEdit, onDone }: { s: McpServer; wsName: (id: string) => string; onEdit: () => void; onDone: () => unknown }) {
  const [busy, setBusy] = useState(false);
  const toggle = (tool: string, on: boolean) => api.put(`/api/mcp/${s.id}/tools/${encodeURIComponent(tool)}`, { on }).then(onDone, toastError);
  const reload = () => { setBusy(true); void api.post<McpServer>(`/api/mcp/${s.id}/refresh`, {}).then((r) => { if (r.error) toastError(new Error(r.error)); else toast(`${r.tools.length} tools`); return onDone(); }, toastError).finally(() => setBusy(false)); };
  return (
    <div className="group grid grid-cols-[minmax(0,1fr)] gap-1.5 border-b border-line-2 pb-2.5 last:border-b-0 last:pb-0">
      <div className="flex flex-wrap items-center gap-2 text-[13px]">
        <span className="grid min-w-0 flex-1 gap-0.5">
          <span className="flex flex-wrap items-center gap-1.5"><b>{s.name}</b><span className="font-mono text-[12px] text-ink-3">{s.id}</span>{s.error && <Badge tone="attn" title={s.error}>not reachable</Badge>}</span>
          <span className="truncate text-[12px] text-ink-3">{s.url} · {s.auth ? "with token" : "no token"} · {s.workspaces.map(wsName).join(", ") || "not granted"}{s.refreshed_at ? ` · tools read ${ago(s.refreshed_at)}` : ""}</span>
        </span>
        {busy && <span className="text-[12px] text-ink-3">Reading…</span>}
        <RowActions label={`More actions for ${s.name}`} width={64} main={<button className="btn btn-sm" onClick={onEdit}>Edit</button>} more={(close) => <>
          <MenuItem icon={<Icon name="refresh" size={14} />} label="Refresh tools" detail="Read the server's tool list again" disabled={busy} onClick={() => { close(); reload(); }} />
          <MenuSection><ConfirmButton icon="trash" consequence={`Deletes ${s.name} and its token. New sessions no longer reach it.`} confirmLabel="Delete" onConfirm={() => api.delete(`/api/mcp/${s.id}`).then(() => { close(); return onDone(); }, toastError)}>Delete</ConfirmButton></MenuSection>
        </>} />
      </div>
      {s.error && <span className="text-[12px] text-attn">{s.error}</span>}
      {s.tools.length > 0 && (
        <div className="grid gap-1">
          {s.tools.map((t) => {
            const on = !s.tools_off.includes(t.name);
            return (
              <div key={t.name} className="flex flex-wrap items-center gap-x-2 gap-y-1 text-[12px]">
                <Choices label={`${t.name} on or off`}>
                  <Choice active={on} onClick={() => !on && void toggle(t.name, true)}>On</Choice>
                  <Choice active={!on} onClick={() => on && void toggle(t.name, false)}>Off</Choice>
                </Choices>
                <span className="font-mono">{t.name}</span>
                {t.read_only && <Badge tone="teal">read only</Badge>}
                {t.destructive && <Badge tone="attn">destructive</Badge>}
                <span className="min-w-0 flex-1 basis-40 truncate text-ink-3" title={t.description}>{t.description}</span>
              </div>
            );
          })}
        </div>
      )}
    </div>
  );
}

function ServerForm({ server, onClose, onDone }: { server: McpServer; onClose: () => void; onDone: () => unknown }) {
  const workspaces = useWorkspaces().data ?? [];
  const isNew = !server.id;
  const [name, setName] = useState(server.name);
  const [url, setUrl] = useState(server.url);
  const [token, setToken] = useState("");
  const [granted, setGranted] = useState<string[]>(server.workspaces);
  const [busy, setBusy] = useState(false);
  const id = isNew ? slug(name) : server.id;
  const everywhere = granted.includes("*");
  const save = () => {
    setBusy(true);
    void api.put<McpServer>(`/api/mcp/${id}`, { name, url, auth: server.auth ?? (token ? { kind: "bearer" } : null), workspaces: granted, token: token || null })
      .then((r) => { toast(r.error ? `Saved; ${r.error}` : `Saved · ${r.tools.length} tools`); onClose(); return onDone(); }, toastError)
      .finally(() => setBusy(false));
  };
  return (
    <div className="grid gap-2 rounded-control border border-line-2 p-3">
      <b className="text-[13px]">{isNew ? "Add an MCP server" : `Edit ${server.name}`}</b>
      <label className="grid gap-1 text-[12px]">Name<input className="field-input" value={name} onChange={(e) => setName(e.target.value)} placeholder="Gmail" /></label>
      <label className="grid gap-1 text-[12px]">Address<input className="field-input font-mono text-[12px]" value={url} onChange={(e) => setUrl(e.target.value)} placeholder="https://mcp.example.com/mcp" spellCheck={false} /></label>
      <label className="grid gap-1 text-[12px]">Token<input className="field-input" type="password" autoComplete="off" value={token} onChange={(e) => setToken(e.target.value)} placeholder={isNew ? "Sent as Authorization: Bearer (leave empty if none)" : "Leave empty to keep the current token"} /></label>
      <div className="grid gap-1 text-[12px]">Granted in
        <Choices label="Granted in">
          <Choice active={everywhere} onClick={() => setGranted(["*"])}>Every workspace</Choice>
          {workspaces.map((w) => <Choice key={w.id} active={!everywhere && granted.includes(w.id)} onClick={() => setGranted((g) => { const rest = g.filter((x) => x !== "*"); return rest.includes(w.id) ? rest.filter((x) => x !== w.id) : [...rest, w.id]; })}>{w.name}</Choice>)}
        </Choices>
        {!granted.length && <span className="text-ink-3">Choose where agents may use it.</span>}
      </div>
      <div className="flex gap-2"><button className="btn btn-sm btn-primary" disabled={busy || !id || !url.trim() || !granted.length} onClick={save}>{busy ? "Reading its tools…" : "Save"}</button><button className="btn btn-sm" onClick={onClose}>Cancel</button></div>
    </div>
  );
}
