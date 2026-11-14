// GitHub's rate limits as of Home's last GitHub call (`GET /api/gh/limits`), for the status bar, the phone status
// and Settings → GitHub.

import { useQuery } from "@tanstack/react-query";
import type { RateLimit } from "@/gen/RateLimit";
import { api } from "@/lib/api";

export const useGitHubLimits = () => useQuery({ queryKey: ["gh-limits"], queryFn: () => api.get<Record<string, RateLimit>>("/api/gh/limits"), refetchInterval: 60_000, staleTime: 30_000 });
