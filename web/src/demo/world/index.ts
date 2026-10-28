// The demo world: one fictional organization (acme-labs) and one fictional user, with every state Launchpad can
// show at least once. Each area builds its own part; `resetWorld` rebuilds everything, which each tour chapter
// does to start from a known state. Timestamps are offsets from `clock.now()`.

import { coreWorld } from "./core";
import { agentsWorld } from "./agents";
import { githubWorld } from "./github";
import { codeWorld } from "./code";
import { settingsWorld } from "./settings";

export type World = ReturnType<typeof coreWorld> & ReturnType<typeof agentsWorld> & ReturnType<typeof githubWorld> & ReturnType<typeof codeWorld> & ReturnType<typeof settingsWorld>;

export let world: World = null as unknown as World;

export function resetWorld() {
  world = { ...coreWorld(), ...agentsWorld(), ...githubWorld(), ...codeWorld(), ...settingsWorld() };
}
