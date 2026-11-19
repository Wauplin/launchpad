// Default tools (`crates/core/src/tool_defaults.rs`): among the tools and MCP servers granted to a workspace, each
// layer (workspace, folder, bot) may switch an item on or off by default; the most specific layer wins, and an item
// no layer names is on, except opt-in tools (`OPT_IN`: Slack (write)), which stay off until switched on on purpose;
// a bot gets one only from its own settings (`forBot`). A launch then switches items on (`tools_on`) or off
// (`tools_off`) for itself. Keys are tool IDs and `mcp:<server id>`.

import { useQuery } from "@tanstack/react-query";
import type { ToolDefaults } from "@/gen/ToolDefaults";
import { api } from "@/lib/api";

export type ToolChoice = { [key in string]: boolean };

export const mcpKey = (id: string) => `mcp:${id}`;

/** Tools that act towards other people for the owner: off unless switched on on purpose (`tool_defaults::OPT_IN`). */
export const OPT_IN: readonly string[] = ["slack-write"];
export const optIn = (key: string) => OPT_IN.includes(key);

/** A workspace or folder layer as a bot reads it: it may switch an opt-in tool off, never on (`tool_defaults::for_bot`). */
export function forBot(layer: ToolChoice | undefined): ToolChoice | undefined {
  if (!layer) return layer;
  return Object.fromEntries(Object.entries(layer).filter(([k, on]) => !(optIn(k) && on)));
}

/** The default of `key` under these layers (most general first). */
export function defaultOn(layers: (ToolChoice | undefined)[], key: string): boolean {
  for (let i = layers.length - 1; i >= 0; i--) {
    const v = layers[i]?.[key] as boolean | undefined;
    if (v !== undefined) return v;
  }
  return !optIn(key);
}

/** Whether `key` is on for a launch: its own choice, else the defaults. */
export function launchOn(layers: (ToolChoice | undefined)[], off: string[], on: string[], key: string): boolean {
  if (on.includes(key)) return true;
  if (off.includes(key)) return false;
  return defaultOn(layers, key);
}

/** The launch choice after a click on `key`: an override only when it differs from the default. */
export function flipLaunch(layers: (ToolChoice | undefined)[], off: string[], on: string[], key: string): { tools_off: string[]; tools_on: string[] } {
  const now = launchOn(layers, off, on, key);
  const base = defaultOn(layers, key);
  const tools_off = off.filter((k) => k !== key);
  const tools_on = on.filter((k) => k !== key);
  if (!now === base) return { tools_off, tools_on };
  return now ? { tools_off: [...tools_off, key], tools_on } : { tools_off, tools_on: [...tools_on, key] };
}

export function useToolDefaults() {
  return useQuery({ queryKey: ["tool-defaults"], queryFn: () => api.get<ToolDefaults>("/api/tool-defaults"), staleTime: 30_000, retry: false });
}

/** The workspace and folder layers of a launch, then the bot's from its settings; `isBot`: a bot's session, which
 *  reads the first two `forBot`. */
export function useLayers(workspaceId: string | null | undefined, folderId: string | null | undefined, bot?: ToolChoice, isBot = false): (ToolChoice | undefined)[] {
  const d = useToolDefaults().data;
  const shared = [workspaceId ? d?.workspaces[workspaceId] : undefined, folderId ? d?.folders[folderId] : undefined];
  return [...(isBot ? shared.map(forBot) : shared), bot];
}
