// Settings → Home and runners → Exit routes: URLs of a GitHub organization (or URL prefixes) that leave through one
// runner, the exit runner, for Home, Burst and the other runners (crates/core/src/egress.rs). New route and Edit
// open one form in place (`?route=new` or `?route=<name>`, its typed values kept on back); Delete is last in ⋯.
// Writes go through the action `egress.set` (the whole list); the list follows the `egress` bus event.

import { useNavigate, useSearch } from "@tanstack/react-router";
import { useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import type { ExitRoute } from "@/gen/ExitRoute";
import type { ExitRouteState } from "@/gen/ExitRouteState";
import { Badge, Choice, Choices, ConfirmButton, ErrorState, Loading, MenuSection, RowActions, SettingRow, humanError } from "@/components/ui";
import { Icon } from "@/components/Icon";
import { toast, toastError } from "@/components/Toast";
import { freeName, routeKind, routeName, routeTarget, saveRoutes, useEgress, useExitRunners } from "@/lib/egress";
import { ago, when } from "@/lib/format";
import { useKeptDraft, versionOf } from "@/lib/keptDraft";
import { Card } from "./SettingsView";

/** Online or offline, and since when. */
function stateLine(s: ExitRouteState): string {
  const since = s.since ? ` since ${ago(s.since)}` : "";
  return s.online ? `online${since}` : `offline${since}`;
}

export function ExitRoutesCard() {
  const qc = useQueryClient();
  const egress = useEgress();
  const navigate = useNavigate();
  const editing = (useSearch({ strict: false }) as { route?: string }).route;
  const open = (route?: string) => void navigate({ to: "/settings/runners", search: (route ? { route } : {}) as never, replace: true });
  const data = egress.data;
  const routes = data?.routes ?? [];
  const states = data?.states ?? [];
  const remove = (name: string) => {
    const before = routes;
    return saveRoutes(qc, routes.filter((r) => r.name !== name)).then(() => {
      toast(`Deleted route ${name}`, { label: "Undo", run: () => void saveRoutes(qc, before).catch(toastError) });
    }, toastError);
  };
  const isNew = editing === "new";
  const edited = routes.find((r) => r.name === editing);
  return (
    <Card title={routes.length ? `Exit routes · ${routes.length}` : "Exit routes"} tour="settings.egress" end={!isNew && <button className="btn btn-sm" onClick={() => open("new")}><Icon name="plus" size={12} />New route</button>}>
      {egress.isLoading ? <Loading what="exit routes" /> : egress.error ? <ErrorState error={egress.error} /> : (
        <div className="grid grid-cols-[minmax(0,1fr)]">
          {isNew && <RouteForm routes={routes} onDone={() => open()} />}
          {routes.length === 0 && !isNew && <span className="text-[13px] text-ink-3">No exit route. Add one when a GitHub organization accepts only allow-listed addresses, or an address is reachable only from one of your runners.</span>}
          {routes.map((r) => {
            const s = states.find((x) => x.name === r.name);
            if (edited && r.name === edited.name) return <RouteForm key={r.name} routes={routes} route={r} onDone={() => open()} />;
            return <RouteRow key={r.name} route={r} state={s} onEdit={() => open(r.name)} onDelete={() => remove(r.name)} />;
          })}
        </div>
      )}
    </Card>
  );
}

function RouteRow({ route, state, onEdit, onDelete }: { route: ExitRoute; state?: ExitRouteState; onEdit: () => void; onDelete: () => Promise<unknown> }) {
  const target = routeTarget(route);
  return (
    <div className="group flex items-center gap-2 border-b border-line-2 py-2.5 text-[13px] first:pt-0 last:border-b-0 last:pb-0">
      <span className="grid min-w-0 flex-1 gap-0.5">
        <span className="flex min-w-0 flex-wrap items-center gap-x-2 gap-y-1">
          <b className="truncate">{route.name}</b>
          {state && (state.online
            ? <Badge tone="ok" dot title={state.since ? `Online since ${when(state.since)}` : undefined}>online</Badge>
            : <Badge dot title={state.since ? `Offline since ${when(state.since)}` : undefined}>offline</Badge>)}
        </span>
        <span className="text-[12px] text-ink-3">
          {state ? routeKind(state) : route.github_org ? "GitHub organization" : "URL prefixes"}{route.github_org ? "" : ` · ${target}`} · <span className="whitespace-nowrap">through <b className="font-semibold text-ink-2">{state?.exit ?? route.via}</b></span>{state && <> · <span className="whitespace-nowrap">{stateLine(state)}</span></>}
        </span>
        {state?.last_error && <span className="text-[12px] text-attn">{state.last_error}</span>}
      </span>
      <RowActions
        label={`More actions for route ${route.name}`}
        width={64}
        main={<button className="btn btn-sm" onClick={onEdit}>Edit</button>}
        more={(close) => (
          <MenuSection>
            <ConfirmButton icon="trash" consequence={`${target} goes direct again, from each machine's own address.`} confirmLabel="Delete route" onConfirm={() => onDelete().then(close)}>Delete</ConfirmButton>
          </MenuSection>
        )}
      />
    </div>
  );
}

type Form = { kind: "github_org" | "urls"; org: string; urls: string; via: string; name: string };

function formOf(r?: ExitRoute, via = ""): Form {
  return { kind: r && !r.github_org ? "urls" : "github_org", org: r?.github_org ?? "", urls: (r?.urls ?? []).join("\n"), via: r?.via ?? via, name: r?.name ?? "" };
}

/** New route or Edit, in place of the row. `prefill`: an organization the Clone offer names. */
export function RouteForm({ routes, route, onDone }: { routes: ExitRoute[]; route?: ExitRoute; onDone: () => void }) {
  const qc = useQueryClient();
  const runners = useExitRunners(route?.via);
  const initial = JSON.stringify(formOf(route, runners[0]?.name ?? ""));
  const draft = useKeptDraft(`egress.${route?.name ?? "new"}.${versionOf(initial)}`, initial);
  const f = ((): Form => { try { return { ...formOf(route), ...(JSON.parse(draft.text) as Partial<Form>) }; } catch { return formOf(route); } })();
  const set = (patch: Partial<Form>) => draft.setText(JSON.stringify({ ...f, ...patch }));
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const urls = f.urls.split("\n").map((u) => u.trim()).filter(Boolean);
  const target = f.kind === "github_org" ? f.org.trim() : urls[0] ?? "";
  const suggested = target ? freeName(routeName(target), routes, route?.name) : "";
  const name = f.name.trim() || suggested;
  const ready = !!name && !!f.via && (f.kind === "github_org" ? !!f.org.trim() : urls.length > 0);
  const save = async () => {
    const next: ExitRoute = { name, github_org: f.kind === "github_org" ? f.org.trim() : null, urls: f.kind === "urls" ? urls : [], via: f.via };
    const list = route ? routes.map((r) => (r.name === route.name ? next : r)) : [...routes, next];
    setBusy(true);
    setError(null);
    try {
      await saveRoutes(qc, list);
      draft.clear();
      toast(route ? `Saved route ${name}` : `Added route ${name}`);
      onDone();
    } catch (e) {
      setError(humanError(e));
    } finally {
      setBusy(false);
    }
  };
  const cancel = () => { draft.clear(); onDone(); };
  return (
    <div className="grid border-b border-line-2 pb-3 last:border-b-0 last:pb-0 [&:not(:first-child)]:pt-3">
      <span className="flex flex-wrap items-center gap-2 text-[13px]">
        <b>{route ? `Edit ${route.name}` : "New route"}</b>
        {draft.kept && <span className="text-[12px] text-ink-3">Draft kept</span>}
      </span>
      <SettingRow label="Reaches">
        <Choices label="What the route reaches">
          <Choice active={f.kind === "github_org"} onClick={() => set({ kind: "github_org" })}>GitHub organization</Choice>
          <Choice active={f.kind === "urls"} onClick={() => set({ kind: "urls" })}>URL prefixes</Choice>
        </Choices>
      </SettingRow>
      {f.kind === "github_org" ? (
        <SettingRow label="Organization" htmlFor="egress-org" hint="Its git, API, raw and upload addresses. GraphQL and search too while the runner is online.">
          <input id="egress-org" className="field-input w-full max-w-80 font-mono text-[12px]" placeholder="huggingface-internal" autoCapitalize="off" autoCorrect="off" spellCheck={false} value={f.org} onChange={(e) => set({ org: e.target.value.replace(/^https?:\/\/github\.com\//, "").split("/")[0] })} />
        </SettingRow>
      ) : (
        <SettingRow label="URL prefixes" htmlFor="egress-urls" hint="One per line, at most 20. https://host/ sends the whole host.">
          <textarea id="egress-urls" className="field-input min-h-20 w-full max-w-120 py-1.5 font-mono text-[12px]" rows={3} placeholder="https://grafana.example.ts.net/" autoCapitalize="off" autoCorrect="off" spellCheck={false} value={f.urls} onChange={(e) => set({ urls: e.target.value })} />
        </SettingRow>
      )}
      <SettingRow label="Through" hint={runners.length ? "The runner that can reach it. While it is offline, these requests fail at once." : "Pair a runner first: the exit is one of your machines, never Burst."}>
        {runners.length > 0 && (
          <Choices label="Exit runner">
            {runners.map((r) => (
              <Choice key={r.name} active={f.via.toLowerCase() === r.name.toLowerCase()} onClick={() => set({ via: r.name })} title={r.online ? "Connected now" : "Not connected now"}>
                <span className={`size-1.5 rounded-full ${r.online ? "bg-gh-open" : "bg-ink-3"}`} aria-hidden="true" />
                {r.name}
              </Choice>
            ))}
          </Choices>
        )}
      </SettingRow>
      <SettingRow label="Name" htmlFor="egress-name" hint="Named in errors and logs.">
        <input id="egress-name" className="field-input w-full max-w-80 font-mono text-[12px]" placeholder={suggested || "huggingface-internal"} autoCapitalize="off" autoCorrect="off" spellCheck={false} value={f.name} onChange={(e) => set({ name: e.target.value.toLowerCase() })} />
      </SettingRow>
      {error && <span className="pt-2 text-[12px] text-danger-ink">{error}</span>}
      <span className="flex flex-wrap items-center gap-2 pt-3">
        <button className="btn btn-sm btn-primary" disabled={!ready || busy} onClick={() => void save()}>{busy ? "Saving…" : route ? "Save" : "Add route"}</button>
        <button className="btn btn-sm btn-ghost" disabled={busy} onClick={cancel}>Cancel</button>
      </span>
    </div>
  );
}
