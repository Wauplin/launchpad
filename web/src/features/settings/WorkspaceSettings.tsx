// Settings → This workspace: the window's workspace (name, its bots, the tools and MCP servers granted to it and
// which of them new sessions get by default, another window, Delete). Everything else in Settings is shared by every workspace.

import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import type { McpServer } from "@/gen/McpServer";
import type { VaultOverview } from "@/gen/VaultOverview";
import { Icon } from "@/components/Icon";
import { toastError } from "@/components/Toast";
import { ConfirmButton, Empty, Loading, SettingRow } from "@/components/ui";
import { api } from "@/lib/api";
import { botInitial, useBots } from "@/lib/bots";
import { newWindowWord, openWindow, useScope, useWorkspaces } from "@/lib/workspace";
import { Card, SavedInput, SettingsPage } from "./SettingsView";
import { ToolDefaultsEditor, useSaveToolDefaults } from "./ToolDefaults";
import { useToolDefaults } from "@/lib/toolDefaults";

const granted = (workspaces: string[], id: string) => workspaces.includes("*") || workspaces.includes(id);

export function WorkspaceSettings() {
  const qc = useQueryClient();
  const scope = useScope();
  const w = scope.workspace;
  const workspaces = useWorkspaces().data ?? [];
  const bots = (useBots().data ?? []).filter((b) => b.workspace_id === w.id);
  const vault = useQuery({ queryKey: ["vault"], queryFn: () => api.get<VaultOverview>("/api/vault"), staleTime: 30_000 });
  const mcp = useQuery({ queryKey: ["mcp"], queryFn: () => api.get<McpServer[]>("/api/mcp"), staleTime: 30_000 });
  const tools = (vault.data?.tools ?? []).filter((t) => granted(t.workspaces, w.id));
  const servers = (mcp.data ?? []).filter((s) => granted(s.workspaces, w.id));
  const defaults = useToolDefaults();
  const saveDefaults = useSaveToolDefaults("workspace", w.id);
  const last = workspaces.length <= 1;
  if (!w.id) return <SettingsPage title="This workspace"><Loading /></SettingsPage>;
  return (
    <SettingsPage title={w.name} intro="Settings of the workspace shown in this window. The other Settings pages are shared by every workspace.">
      <Card title="Workspace">
        <SettingRow label="Name" hint="Names the window and the rail square.">
          <SavedInput label="Workspace name" value={w.name} onSave={async (name) => { await api.patch(`/api/workspaces/${w.id}`, { name }); await qc.invalidateQueries({ queryKey: ["workspaces"] }); }} />
        </SettingRow>
        <SettingRow label="Folders" hint="Every folder belongs to one workspace.">
          <span className="flex flex-wrap items-center gap-2 text-[13px]">{scope.projects.filter((p) => !p.is_temp).length} folders<Link to="/projects" className="btn btn-sm no-underline">Open Folders</Link></span>
        </SettingRow>
        <SettingRow label="Another window" hint="Two workspaces side by side means two windows.">
          <button className="btn btn-sm w-fit" onClick={() => void openWindow(w.id)}><Icon name="window" size={12} />Open in a new {newWindowWord}</button>
        </SettingRow>
      </Card>
      <Card title="Bots" end={<Link to="/bots" search={{ new: true } as never} className="btn btn-sm no-underline"><Icon name="plus" size={12} />New bot</Link>}>
        {bots.length === 0 ? <Empty icon="spark">No bot in {w.name}.</Empty> : bots.map((b) => (
          <Link key={b.id} to="/bots/$id" params={{ id: b.id }} className="flex min-h-[32px] items-center gap-2 text-[13px] text-ink no-underline hover:underline">
            <span className="grid size-[18px] flex-none place-items-center rounded-[4px] bg-teal-soft text-[11px] font-semibold text-teal-ink" aria-hidden>{botInitial(b.settings.name)}</span>
            {b.settings.name}
          </Link>
        ))}
      </Card>
      <Card title="Tools and MCP servers" end={<Link to="/settings/credentials" className="btn btn-sm no-underline">Edit in Credentials and tools</Link>}>
        <span className="text-[12px] text-ink-3">Sessions of {w.name} can use these tools and MCP servers. Their tokens stay on Home; you grant them per workspace in Credentials and tools. Off: a new session gets it only when a folder, a bot or the composer switches it on.</span>
        {vault.isLoading || mcp.isLoading ? <Loading /> : tools.length + servers.length === 0 ? <Empty icon="term">Nothing is granted to {w.name}.</Empty> : (
          <ToolDefaultsEditor workspaceId={w.id} above={null} value={defaults.data?.workspaces[w.id] ?? {}} onChange={(v) => void saveDefaults(v).catch(toastError)} />
        )}
      </Card>
      <Card title="Delete">
        {last ? <span className="text-[13px] text-ink-2">The last workspace cannot be deleted: Launchpad always shows one.</span> : (
          <SettingRow label="Delete this workspace" hint="Only an empty workspace can be deleted: move or remove its folders first.">
            <ConfirmButton icon="trash" consequence={`Deletes ${w.name}. This window then shows another workspace.`} confirmLabel={`Delete ${w.name}`} onConfirm={() => api.delete(`/api/workspaces/${w.id}`).then(() => qc.invalidateQueries({ queryKey: ["workspaces"] }), toastError)}>Delete workspace</ConfirmButton>
          </SettingRow>
        )}
      </Card>
    </SettingsPage>
  );
}
