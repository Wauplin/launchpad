// Demo routes of Step 6: the activity list (what Ask Launchpad changed directly), ready cards that run
// from their own button, and the shared settings list that admin sessions may change.

import type { Proposal } from "@/gen/Proposal";
import { clock, DemoError, publish, route } from "../daemon";
import { world } from "../world";
import { saveConfig } from "./settings";

route("GET", "/api/activity", (r) => {
  const w = r.query.get("workspace");
  return [...world.activity].filter((a) => !w || a.workspace_id === w).sort((a, b) => b.created_at - a.created_at);
});
route("POST", "/api/activity/:id/undone", (r) => {
  const a = world.activity.find((x) => x.id === r.params.id);
  if (!a) throw new DemoError(404, "no such activity");
  a.undone_at ??= clock.now();
  publish("activity", "changed", a);
  return a;
});

route("POST", "/api/proposals/:id/ran", (r) => {
  const p = world.proposals.find((x) => x.id === r.params.id) as Proposal | undefined;
  if (!p) throw new DemoError(404, "no such proposal");
  if (p.body.kind !== "ready") throw new DemoError(400, "only ready cards run from their button");
  p.state = "started";
  p.result_id = p.body.action;
  p.updated_at = clock.now();
  publish("proposals", "changed", p);
  return p;
});

const shared = () => ({ presets: world.config.presets, worktrees_keep: world.config.worktrees.keep });
route("GET", "/api/settings/shared", shared);
route("PATCH", "/api/settings/shared", ({ body }) => {
  const p = (body ?? {}) as Partial<ReturnType<typeof shared>>;
  const c = structuredClone(world.config);
  if (p.presets !== undefined) c.presets = p.presets;
  if (p.worktrees_keep !== undefined) c.worktrees.keep = p.worktrees_keep;
  saveConfig(c);
  return shared();
});
