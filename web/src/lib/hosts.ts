// SSH hosts (Settings → Hosts). A folder on a host is written `<host>:<path>`, the same location string the
// daemon stores in project paths, session folders and terminals. Host state comes with `Status.hosts`.

import type { HostStatus } from "@/gen/HostStatus";
import { useStatus } from "./queries";

const LOCATION = /^([a-z0-9][a-z0-9._-]{1,62}):(\/.*)$/;

/** The host of a location, or null for this computer. */
export function hostOf(path: string | null | undefined): string | null {
  return path ? (LOCATION.exec(path)?.[1] ?? null) : null;
}

/** The path as the host sees it. */
export function nativePath(path: string): string {
  return LOCATION.exec(path)?.[2] ?? path;
}

export function joinHost(host: string | null, native: string): string {
  return host ? `${host}:${native}` : native;
}

export function useHosts(): HostStatus[] {
  return useStatus().data?.hosts ?? [];
}

export function useHost(id: string | null | undefined): HostStatus | null {
  const hosts = useHosts();
  return id ? (hosts.find((h) => h.host.id === id) ?? null) : null;
}

/** `~/projects/x` on a host with a known home, else the native path. */
export function shortPath(path: string, hosts: HostStatus[]): string {
  const host = hostOf(path);
  const native = nativePath(path);
  const home = host ? hosts.find((h) => h.host.id === host)?.info?.home : null;
  if (home && (native === home || native.startsWith(`${home}/`))) return `~${native.slice(home.length)}`;
  return native;
}

export const linkLabel: Record<HostStatus["state"], string> = {
  connecting: "Connecting",
  connected: "Connected",
  unreachable: "Unreachable",
  needs_setup: "Needs setup",
};

/** Whether a harness can run on a host, from its last preflight: null when unknown. */
export function harnessOnHost(h: HostStatus | null, harness: string): { ok: boolean; reason: string } | null {
  if (!h) return null;
  const c = h.checks.find((x) => x.id === harness);
  if (!c) return null;
  if (c.found === false) return { ok: false, reason: `Not installed on ${h.host.id}` };
  if (c.signed_in === false) return { ok: false, reason: `Not signed in on ${h.host.id}` };
  return { ok: true, reason: "" };
}
