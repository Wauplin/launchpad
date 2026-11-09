// Default tools of a workspace, a folder or a bot: which of the tools and MCP servers granted to the workspace
// a new session gets without a click (`web/src/lib/toolDefaults.ts`). The composer can still change them per launch.

import { useQueryClient } from "@tanstack/react-query";
import { Icon } from "@/components/Icon";
import { toastError } from "@/components/Toast";
import { runAction } from "@/lib/actions";
import { defaultOn, mcpKey, useLayers, type ToolChoice } from "@/lib/toolDefaults";
import { useVault } from "./CredentialsView";
import { useMcpServers } from "./McpServers";

const granted = (workspaces: string[], id: string | null) => workspaces.includes("*") || (!!id && workspaces.includes(id));

/** The tools and MCP servers granted to a workspace, by key. */
export function useWorkspaceTools(workspaceId: string | null) {
  const vault = useVault();
  const mcp = useMcpServers();
  const tools = (vault.data?.tools ?? []).filter((t) => granted(t.workspaces, workspaceId)).map((t) => ({ key: t.id, name: t.name, mcp: false }));
  const servers = (mcp.data ?? []).filter((s) => granted(s.workspaces, workspaceId)).map((s) => ({ key: mcpKey(s.id), name: s.name, mcp: true }));
  return { items: [...tools, ...servers], loading: vault.isLoading || mcp.isLoading };
}

/** Saves the default tools of a workspace or a folder (action `tools.defaults`). */
export function useSaveToolDefaults(scope: "workspace" | "folder", id: string) {
  const qc = useQueryClient();
  return async (tools: ToolChoice) => {
    await runAction("tools.defaults", { scope, id }, { tools });
    await qc.invalidateQueries({ queryKey: ["tool-defaults"] });
  };
}

/** One row per granted item. `above` are the layers this one overrides (none for a workspace). */
export function ToolDefaultsEditor({ workspaceId, above, value, onChange, disabled }: { workspaceId: string | null; above: (ToolChoice | undefined)[] | null; value: ToolChoice; onChange: (v: ToolChoice) => void; disabled?: boolean }) {
  const { items } = useWorkspaceTools(workspaceId);
  if (!items.length) return <span className="text-[12px] text-ink-3">No tool or MCP server is granted to this workspace (Settings → Credentials and tools).</span>;
  const set = (key: string, v: string) => {
    const next = { ...value };
    if (v === "") delete next[key];
    else next[key] = v === "on";
    onChange(next);
  };
  return (
    <div className="grid gap-1">
      {items.map((t) => {
        const current = value[t.key] as boolean | undefined;
        const inherited = above ? defaultOn(above, t.key) : true;
        return (
          <label key={t.key} className="flex min-h-[30px] items-center gap-2 text-[13px]">
            <Icon name={t.mcp ? "spark" : "term"} size={12} className="flex-none text-ink-3" />
            <span className="min-w-0 flex-1 truncate">{t.name}{t.mcp && <span className="text-ink-3"> · MCP server</span>}</span>
            <select className="field-input w-auto" aria-label={`Default of ${t.name}`} disabled={disabled}
              value={above ? (current === undefined ? "" : current ? "on" : "off") : current === false ? "off" : "on"}
              onChange={(e) => set(t.key, above ? e.target.value : e.target.value === "off" ? "off" : "")}>
              {above && <option value="">{inherited ? "Inherit (on)" : "Inherit (off)"}</option>}
              <option value="on">On</option>
              <option value="off">Off</option>
            </select>
          </label>
        );
      })}
    </div>
  );
}

/** The default tools of a folder, saved as they change. */
export function FolderToolDefaults({ folderId, workspaceId }: { folderId: string; workspaceId: string | null }) {
  const layers = useLayers(workspaceId, folderId);
  const save = useSaveToolDefaults("folder", folderId);
  const value = layers[1] ?? {};
  return <ToolDefaultsEditor workspaceId={workspaceId} above={[layers[0]]} value={value} onChange={(v) => void save(v).catch(toastError)} />;
}
