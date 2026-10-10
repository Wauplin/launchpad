// Named harness variants come from config.toml; sessions store only the variant ID.

import type { HarnessVariant } from "@/gen/HarnessVariant";
import type { Session } from "@/gen/Session";
import { useConfig } from "./queries";

const none: HarnessVariant[] = [];
export const useVariants = () => useConfig().data?.config.variants ?? none;

export function useVariant(id: string | null | undefined) {
  return useVariants().find((v) => v.id === id) ?? null;
}

export const usd = (n: number) => `$${n.toFixed(2)}`;

export const budgetReached = (s: Session) => s.launch_spec.budget_usd != null && (s.usage.cost_usd ?? 0) >= s.launch_spec.budget_usd;

/** A positive amount, or null for an empty field; undefined when the text is invalid. */
export function parseUsd(text: string): number | null | undefined {
  const t = text.trim().replace(/^\$/, "");
  if (!t) return null;
  const n = Number(t);
  return Number.isFinite(n) && n > 0 ? n : undefined;
}
