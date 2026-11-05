// The Outbox data: agent drafts for outgoing channels and GitHub suggestions, both waiting for the owner's click.
// Kept small so the sidebar count does not load the Outbox page.

import { useQuery } from "@tanstack/react-query";
import { api } from "@/lib/api";
import type { OutboxMessage } from "@/gen/OutboxMessage";
import type { Suggestion } from "@/gen/Suggestion";

export const useOutboxMessages = () => useQuery({ queryKey: ["outbox"], queryFn: () => api.get<OutboxMessage[]>("/api/outbox"), staleTime: 30_000 });
export const useAllSuggestions = () => useQuery({ queryKey: ["suggestions"], queryFn: () => api.get<Suggestion[]>("/api/suggestions"), staleTime: 30_000 });

/** Only GitHub suggestions reach other people; Local PR notes stay in Launchpad. */
export const isGitHubSuggestion = (s: Suggestion) => s.target.kind === "git_hub";

export type OutboxTab = "to_send" | "sent" | "discarded";

export function messageTab(m: OutboxMessage): OutboxTab {
  return m.status === "sent" ? "sent" : m.status === "discarded" ? "discarded" : "to_send";
}

export function suggestionTab(s: Suggestion): OutboxTab {
  return s.status === "posted" ? "sent" : "to_send";
}

/** Drafts to send (uncertain ones included: they need a look). */
export function useOutboxCount(): number {
  const messages = useOutboxMessages().data ?? [];
  const suggestions = useAllSuggestions().data ?? [];
  return messages.filter((m) => messageTab(m) === "to_send").length + suggestions.filter((s) => isGitHubSuggestion(s) && suggestionTab(s) === "to_send").length;
}
