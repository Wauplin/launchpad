// The demo world: one fictional organization (acme-labs) and one fictional user, with every state Launchpad can
// show at least once. Each area builds its own part; `resetWorld` rebuilds everything, which each tour chapter
// does to start from a known state. Timestamps are offsets from `clock.now()`.

import { coreWorld } from "./core";
import { agentsWorld } from "./agents";
import { githubWorld } from "./github";
import { codeWorld } from "./code";
import { settingsWorld } from "./settings";
import { knowledgeWorld } from "./knowledge";
import { botsWorld } from "./bots";
import { connectionsWorld } from "./connections";
import { channelsWorld } from "./channels";
import { WS } from "./ids";
import { clock } from "../daemon";

export type World = ReturnType<typeof coreWorld> & ReturnType<typeof agentsWorld> & ReturnType<typeof githubWorld> & ReturnType<typeof codeWorld> & ReturnType<typeof settingsWorld> & ReturnType<typeof knowledgeWorld> & ReturnType<typeof botsWorld> & ReturnType<typeof connectionsWorld> & ReturnType<typeof channelsWorld>;

export let world: World = null as unknown as World;

export function resetWorld() {
  world = { ...coreWorld(), ...agentsWorld(), ...githubWorld(), ...codeWorld(), ...settingsWorld(), ...knowledgeWorld(), ...botsWorld(), ...connectionsWorld(), ...channelsWorld() };
  // A ready click from Ask Launchpad: an Inbox thread to mark done, run only by your click.
  const old = world.inboxThreads.find((t) => t.reason === "assign" && !t.unread);
  if (old) world.proposals.push({ id: "demo-prop-ready-done", workspace_id: WS.acme, session_id: "demo-s-ask-acme", title: `Mark “${old.title}” done`,
    note: "It was assigned to you weeks ago and nothing happened since; mark it done in the Inbox and on GitHub.",
    body: { kind: "ready", action: "inbox.done", args: [old.id], input: {} }, state: "proposed", result_id: null, replaced_by: null, error: null, created_at: clock.ago(14), updated_at: clock.ago(14) });
}
