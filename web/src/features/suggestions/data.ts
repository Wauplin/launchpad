// Agent suggestions (comments and reviews waiting for the owner's Post), shared by Today and the Inbox.

import { useQuery } from "@tanstack/react-query";
import { useMemo } from "react";
import type { Suggestion } from "@/gen/Suggestion";
import { api } from "@/lib/api";

export const useSuggestionList = () => useQuery({ queryKey: ["suggestions"], queryFn: () => api.get<Suggestion[]>("/api/suggestions") });

/** Unposted suggestions of a GitHub item, keyed `owner/repo#n` (lowercase repo). Reviews come first. */
export function useSuggestionsByItem() {
  const all = useSuggestionList().data;
  return useMemo(() => {
    const out = new Map<string, Suggestion[]>();
    for (const s of all ?? []) {
      if (s.status === "posted" || s.status === "posted_by_hand" || s.target.kind !== "git_hub") continue;
      const k = `${s.target.repo.toLowerCase()}#${s.target.number}`;
      out.set(k, [...(out.get(k) ?? []), s].sort((a, b) => (a.kind === "review" ? 0 : 1) - (b.kind === "review" ? 0 : 1)));
    }
    return out;
  }, [all]);
}
export const itemSuggestionKey = (repo: string, number: number | null) => `${repo.toLowerCase()}#${number}`;
