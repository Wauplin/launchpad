// Home updates on a Hugging Face Space (launchpad_setup::home_update): Home checks every hour for a newer release
// (or CI package of main, as its Space's Dockerfile installs), and Update Home commits the newer Dockerfile to the
// Space, which rebuilds and restarts. The same click can update the runners once Home runs the new build.

import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import type { HomeUpdate } from "@/gen/HomeUpdate";
import { ActionButton } from "@/components/ActionButton";
import { api } from "@/lib/api";

export const useHomeUpdate = () => useQuery({ queryKey: ["home-update"], queryFn: () => api.get<HomeUpdate>("/api/home/update"), staleTime: Infinity });

/** A release tag as is, a main commit cut to 8 characters. */
export const shortVersion = (v: string) => (/^[0-9a-f]{40}$/.test(v) ? v.slice(0, 8) : v);

export const versionLabel = (u: HomeUpdate, v: string) => (u.channel === "main" ? `main ${shortVersion(v)}` : v);

/** Update Home to `version`, with the choice to update the runners afterwards (on by default). */
export function UpdateHomeButton({ u, primary = true }: { u: HomeUpdate; primary?: boolean }) {
  const qc = useQueryClient();
  const [runners, setRunners] = useState(true);
  const latest = u.latest;
  if (!latest) return null;
  return (
    <span className="flex flex-wrap items-center gap-3">
      <ActionButton<HomeUpdate> name="home.update" primary={primary} vars={{ version: versionLabel(u, latest.version) }} body={{ version: latest.version, update_runners: runners }} onDone={(r) => qc.setQueryData(["home-update"], r)}>
        Update Home to {versionLabel(u, latest.version)}
      </ActionButton>
      <label className="flex items-center gap-2 text-[12px] text-ink-2" title="Once Home runs the new build, each connected runner with another build updates itself when none of its agents is in a turn (for 24 hours).">
        <input type="checkbox" checked={runners} onChange={(e) => setRunners(e.target.checked)} />Then update the runners
      </label>
    </span>
  );
}
