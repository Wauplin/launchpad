// Shared primitives for dedicated background requests. Route-specific trust checks stay with each route.
import { RequestError } from "../shared/messages.ts";

export function id(v: unknown, what: string): string {
  if (typeof v !== "string" || !/^[A-Za-z0-9_-]{1,80}$/.test(v)) throw new RequestError(`Not a ${what} ID`);
  return v;
}

export function workspace(ws: string | null): string {
  if (!ws) throw new RequestError("Choose a workspace in the Launchpad panel first");
  return ws;
}
