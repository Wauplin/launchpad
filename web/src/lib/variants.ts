// Named harness variants come from config.toml; sessions store only the variant ID.

import { useQuery } from "@tanstack/react-query";
import type { HarnessVariant } from "@/gen/HarnessVariant";
import type { ModelInfo } from "@/gen/ModelInfo";
import type { Session } from "@/gen/Session";
import { api } from "./api";
import { useConfig } from "./queries";

const none: HarnessVariant[] = [];
export const useVariants = () => useConfig().data?.config.variants ?? none;

export function useVariant(id: string | null | undefined) {
  return useVariants().find((v) => v.id === id) ?? null;
}

/** Models of a variant's OpenAI-compatible list (`models_url`), fetched by the daemon. */
export const useVariantModels = (url: string | null | undefined) =>
  useQuery({
    queryKey: ["variant-models", url],
    queryFn: () => api.post<ModelInfo[]>("/api/variants/models", { url }),
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
