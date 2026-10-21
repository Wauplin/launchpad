// Named harness profiles come from config.toml; sessions store only the profile ID.

import { useQuery } from "@tanstack/react-query";
import type { Profile } from "@/gen/Profile";
import type { ModelInfo } from "@/gen/ModelInfo";
import type { Session } from "@/gen/Session";
import type { SpendWindow } from "@/gen/SpendWindow";
import { formatReset } from "./format";
import { api } from "./api";
import { useConfig } from "./queries";

const none: Profile[] = [];
export const useProfiles = () => useConfig().data?.config.profiles ?? none;

export function useProfile(id: string | null | undefined) {
  return useProfiles().find((v) => v.id === id) ?? null;
}

/** Models of a profile's OpenAI-compatible list (`models_url`), fetched by the daemon. */
export const useProfileModels = (url: string | null | undefined) =>
  useQuery({
    queryKey: ["profile-models", url],
    queryFn: () => api.post<ModelInfo[]>("/api/profiles/models", { url }),
    enabled: !!url,
    staleTime: 30 * 60_000,
    retry: false,
  });

/** `${LAUNCHPAD_MODEL}` in an environment value needs a model at launch. */
export const usesModelReference = (env: Iterable<[string, string]>) => [...env].some(([, v]) => v.includes("${LAUNCHPAD_MODEL}"));

export const usd = (n: number) => `$${n.toFixed(2)}`;

export const budgetReached = (s: Session) => s.launch_spec.budget_usd != null && (s.usage.cost_usd ?? 0) >= s.launch_spec.budget_usd;

/** A positive amount, or null for an empty field; undefined when the text is invalid. */
export function parseUsd(text: string): number | null | undefined {
  const t = text.trim().replace(/^\$/, "");
  if (!t) return null;
  const n = Number(t);
  return Number.isFinite(n) && n > 0 ? n : undefined;
}

/** "5 hours", "1 week", "3 days", like the daemon's notices. */
export function windowLabel(hours: number) {
  const [n, unit] = hours % 168 === 0 ? [hours / 168, "week"] : hours % 24 === 0 ? [hours / 24, "day"] : [hours, "hour"];
  return `${n} ${unit}${n === 1 ? "" : "s"}`;
}

export const spendPercent = (w: SpendWindow) => (w.limit_usd > 0 ? Math.min(100, (w.used_usd / w.limit_usd) * 100) : 0);

/** "$12.30 of $50.00 · room again Mon 09:00". */
export const spendLabel = (w: SpendWindow) => `${usd(w.used_usd)} of ${usd(w.limit_usd)}${w.available_at !== null ? ` · limit reached, room again ${formatReset(w.available_at)}` : ""}`;
