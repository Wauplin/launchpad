// Slack profiles (`crates/core/src/slack.rs`): named Slack tokens on Home that agents use with
// `launchpad slack --profile <handle>`. Which sessions hold one follows the default tools with the key `slack:<handle>`
// (`lib/toolDefaults.ts`), starting from the profile's own default.

import { useQuery } from "@tanstack/react-query";
import type { SlackProfile } from "@/gen/SlackProfile";
import { api } from "@/lib/api";

export function useSlackProfiles() {
  return useQuery({ queryKey: ["slack-profiles"], queryFn: () => api.get<SlackProfile[]>("/api/slack/profiles"), staleTime: 30_000, retry: false });
}

/** The handle a name suggests, like Home's `slack::handle_of` (`Read as me` → `read-as-me`). */
export function handleOf(name: string): string {
  let out = "";
  for (const c of name.trim().toLowerCase()) {
    if (/[a-z0-9]/.test(c)) out += c;
    else if (out && !out.endsWith("-")) out += "-";
  }
  return out.replace(/-+$/, "").slice(0, 40).replace(/-+$/, "");
}

export const validHandle = (h: string) => /^[a-z0-9]+(-[a-z0-9]+)*$/.test(h) && h.length <= 40;

/** In whose name a profile acts, in words. */
export const actsAs = (p: SlackProfile) => (p.token_kind === "user" ? "as the person who installed it" : "as the app's bot");
