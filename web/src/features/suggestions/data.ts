// Agent suggestions (comments and reviews waiting for the owner's Post), shared by Today and the bot chat.

import { useQuery } from "@tanstack/react-query";
import type { Suggestion } from "@/gen/Suggestion";
import { api } from "@/lib/api";

export const useSuggestionList = () => useQuery({ queryKey: ["suggestions"], queryFn: () => api.get<Suggestion[]>("/api/suggestions") });
