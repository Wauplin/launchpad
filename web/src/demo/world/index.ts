import type { Topic } from "@/gen/Topic";
// The demo world: one fictional organization (acme-labs) and one fictional user, with every state Launchpad can
// show at least once. Each area builds its own part; `resetWorld` rebuilds everything, which each tour chapter
// does to start from a known state. Timestamps are offsets from `clock.now()`.

import { coreWorld } from "./core";
import { agentsWorld } from "./agents";
import { fakeSha, githubWorld } from "./github";
import { codeWorld } from "./code";
import { settingsWorld } from "./settings";
import { knowledgeWorld } from "./knowledge";
import { botsWorld } from "./bots";
import { connectionsWorld } from "./connections";
import { channelsWorld } from "./channels";
import { WS } from "./ids";
import { clock } from "../daemon";

export type World = { topics: Topic[] } & ReturnType<typeof coreWorld> & ReturnType<typeof agentsWorld> & ReturnType<typeof githubWorld> & ReturnType<typeof codeWorld> & ReturnType<typeof settingsWorld> & ReturnType<typeof knowledgeWorld> & ReturnType<typeof botsWorld> & ReturnType<typeof connectionsWorld> & ReturnType<typeof channelsWorld>;

export let world: World = null as unknown as World;

export function resetWorld() {
  world = { topics: [], ...coreWorld(), ...agentsWorld(), ...githubWorld(), ...codeWorld(), ...settingsWorld(), ...knowledgeWorld(), ...botsWorld(), ...connectionsWorld(), ...channelsWorld() };
  world.topics = [{ id: "demo-topic-launch", workspace_id: WS.acme, name: "Launch experience", naming: false, session_ids: world.sessions.filter((s) => s.project_id === world.projects.find((p) => p.workspace_id === WS.acme)?.id).slice(0, 4).map((s) => s.id) }];
  // A ready click from Ask Launchpad: the waiting workflows of a contributor's pull request, run only by your click.
  world.proposals.push({ id: "demo-prop-ready-ci", workspace_id: WS.acme, session_id: "demo-s-ask-acme", title: "Approve and run the workflows of storefront#119",
    note: "The pull request comes from a fork, so its checks wait for a maintainer. I read the diff: it changes only the cart view, no workflow file.",
    body: { kind: "ready", action: "github.run-ci", args: ["acme-labs", "storefront", "119"], input: { head: fakeSha("acme-labs/storefront#119:head") } }, state: "proposed", result_id: null, replaced_by: null, error: null, created_at: clock.ago(14), updated_at: clock.ago(14) });
}
