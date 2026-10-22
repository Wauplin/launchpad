// Named harness profiles come from config.toml; sessions store only the profile ID.

import { useQuery } from "@tanstack/react-query";
import { useMemo } from "react";
import type { HarnessKind } from "@/gen/HarnessKind";
import type { QuotaStatus } from "@/gen/QuotaStatus";
import type { Status } from "@/gen/Status";
import type { Profile } from "@/gen/Profile";
import type { ModelInfo } from "@/gen/ModelInfo";
import type { Session } from "@/gen/Session";
import type { SpendWindow } from "@/gen/SpendWindow";
import { formatReset } from "./format";
import { api } from "./api";
import { useConfig, useStatus } from "./queries";

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

export const costUnavailableMessage = "Reliable cost reporting is unavailable for this model. Cost management is unavailable: session and profile spending limits cannot protect this usage. Check your provider's billing dashboard.";

/** The adapter declares the model families it can price. */
export function supportsModelCost(harness: { capabilities: { cost: boolean }; cost_model_prefixes: string[] } | null | undefined, model: string | null | undefined): boolean {
  return !!harness?.capabilities.cost && (!harness.cost_model_prefixes.length || model == null || harness.cost_model_prefixes.some((p) => model.startsWith(p)));
}

export const budgetReached = (s: Session) => !s.usage.cost_unavailable && s.launch_spec.budget_usd != null && (s.usage.cost_usd ?? 0) >= s.launch_spec.budget_usd;

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

/** A monitored subscription login and the names of the profiles that use it. */
export type QuotaEntry = { key: string; harness: HarnessKind; names: string[]; q: QuotaStatus };

/** Subscription logins with their quota readings, in profile configuration order. */
export function useQuotas(): QuotaEntry[] {
  const quotas = useStatus().data?.quotas;
  const profiles = useProfiles();
  return useMemo(() => {
    const order = (e: QuotaEntry) => {
      const i = profiles.findIndex((p) => e.q.profiles.includes(p.id));
      return i < 0 ? profiles.length : i;
    };
    return Object.entries(quotas ?? {})
      .flatMap(([key, q]) => (q ? [{ key, harness: q.harness as HarnessKind, names: q.profiles.map((id) => profiles.find((p) => p.id === id)?.name ?? id), q }] : []))
      .sort((a, b) => order(a) - order(b));
  }, [quotas, profiles]);
}

/** The reading of the login that protects work of this harness and profile: the profile's account, else the default login. */
export function quotaFor(status: Status | undefined, harness: string, profile: string | null | undefined): QuotaStatus | null {
  const quotas = status?.quotas ?? {};
  if (profile) {
    const q = Object.values(quotas).find((x) => x?.profiles.includes(profile));
    if (q) return q;
    // A profile of another account that is not monitored (yet) has no reading.
    return null;
  }
  return quotas[harness] ?? null;
}

/** A reading older than 15 minutes, or with an error, does not confirm the quota. */
export const quotaStale = (q: QuotaStatus | null | undefined) => !q || !!q.quota_error || q.quota_at === null || Date.now() - q.quota_at > 15 * 60_000;
