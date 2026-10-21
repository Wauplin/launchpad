// Settings → Profiles: how Launchpad runs each coding agent (a harness, the login or key that pays for it, and its
// defaults), saved in config.toml. Variable descriptions, defaults and templates come from the adapters
// (`GET /api/harnesses`); sign-in status comes from this computer (`/api/setup/sign-in`) and each host's checks.

import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useEffect, useRef, useState } from "react";
import type { EnvVarDoc } from "@/gen/EnvVarDoc";
import type { HarnessDescriptor } from "@/gen/HarnessDescriptor";
import type { HarnessKind } from "@/gen/HarnessKind";
import type { Profile } from "@/gen/Profile";
import type { ProfileKind } from "@/gen/ProfileKind";
import type { ProfileTemplate } from "@/gen/ProfileTemplate";
import type { SignInCheck } from "@/gen/SignInCheck";
import type { SpendLimit } from "@/gen/SpendLimit";
import { HarnessMark, useHarnesses } from "@/components/HarnessMark";
import { Icon } from "@/components/Icon";
import { ModelSearch } from "@/components/ModelSearch";
import { findModel, pickModel, ProviderPicker } from "@/components/ProviderPicker";
import { MenuItem, Popover } from "@/components/Popover";
import { toast, toastError } from "@/components/Toast";
import { api } from "@/lib/api";
import { copyText } from "@/lib/clipboard";
import { useHosts } from "@/lib/hosts";
import { useConfig, useHarnessDefaults, useStatus, type ConfigResponse } from "@/lib/queries";
import { parseUsd, quotaFor, usd, usesModelReference, useProfileModels, windowLabel } from "@/lib/profiles";
import { useOpenTerminal } from "@/features/terminals/lib";
import { Card, SaveBar, SettingsPage } from "./SettingsView";

/** Editable form of a profile: environment rows keep their order and allow empty names while typing. */
type Draft = Omit<Profile, "env" | "budget_usd" | "spend_limits"> & { key: string; env: [string, string][]; budget: string; limits: LimitDraft[]; saved: boolean; template: ProfileTemplate | null };

/** A rolling spending limit while typing: amount, window length and unit. */
type LimitDraft = { usd: string; count: string; unit: Unit };
type Unit = "hours" | "days" | "weeks";
const UNIT_HOURS: Record<Unit, number> = { hours: 1, days: 24, weeks: 168 };

function toLimitDraft(l: SpendLimit): LimitDraft {
  const unit: Unit = l.hours % 168 === 0 ? "weeks" : l.hours % 24 === 0 ? "days" : "hours";
  return { usd: String(l.usd), count: String(l.hours / UNIT_HOURS[unit]), unit };
}

/** A valid limit, or undefined. Windows are 1 hour to 31 days (744 hours). */
function parseLimit(l: LimitDraft): SpendLimit | undefined {
  const usd = parseUsd(l.usd);
  const count = Number(l.count.trim());
  const hours = count * UNIT_HOURS[l.unit];
  return usd && Number.isInteger(count) && hours >= 1 && hours <= 744 ? { usd, hours } : undefined;
}

let nextKey = 0;
const toDraft = (v: Profile): Draft => ({ ...v, key: `saved-${v.id}`, env: Object.entries(v.env).map(([k, x]) => [k, x ?? ""]), budget: v.budget_usd == null ? "" : String(v.budget_usd), limits: v.spend_limits.map(toLimitDraft), saved: true, template: null });

/** Key order does not matter in the comparison with the saved profiles. */
const canonical = (v: Profile) => [v.id, v.name, v.harness, Object.entries(v.env).sort(), v.model, v.effort, v.billing, v.budget_usd, v.models_url, v.spend_limits.map((l) => [l.usd, l.hours]), v.send_credentials];

function slug(name: string, taken: string[]) {
  const base = name.toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-+|-+$/g, "").slice(0, 32) || "profile";
  let id = base;
  for (let i = 2; taken.includes(id); i++) id = `${base}-${i}`;
  return id;
}

/** Open popovers upward from triggers low in the window, so they stay visible. */
const lowerHalf = (e: React.MouseEvent) => e.currentTarget.getBoundingClientRect().top > window.innerHeight / 2;

/** The first reason a draft cannot be saved. */
function problem(d: Draft): string | null {
  const label = d.name.trim() || "each profile";
  if (!d.name.trim()) return "Each profile needs a name.";
  if (parseUsd(d.budget) === undefined) return `Fix the cost limit of ${label}.`;
  if (d.limits.some((l) => !parseLimit(l))) return `Fix the spending limits of ${label}: a positive amount per 1 hour to 31 days.`;
  if (d.env.some(([k]) => !k.trim())) return `Name every environment variable of ${label}.`;
  if (new Set(d.env.map(([k]) => k.trim())).size !== d.env.length) return `${label} sets a variable twice.`;
  const empty = d.env.find(([, v]) => !v.trim());
  if (empty) return `Enter a value for ${empty[0]} in ${label}, or remove it.`;
  if (!d.model && usesModelReference(d.env)) return `Choose a default model for ${label}.`;
  return null;
}

const KINDS: { kind: ProfileKind; label: string; detail: string }[] = [
  { kind: "subscription", label: "A subscription login", detail: "Claude Pro/Max or ChatGPT Plus/Pro: your usual login or another account. Launchpad watches its 5-hour and weekly quota." },
  { kind: "api_key", label: "An API key, pay as you go", detail: "Billed per use by the harness vendor. Cost limits and rolling spending limits apply." },
  { kind: "provider", label: "Another model provider", detail: "Open models through Hugging Face, or the providers configured in OpenCode. Billed by the provider." },
];

/** What pays for the work of a profile, in a few words. */
function source(d: Draft, harness: HarnessDescriptor | undefined): string {
  const keys = d.env.map(([k]) => k.trim());
  if (d.billing === "subscription") return harness?.account_env.some((n) => keys.includes(n)) ? "Subscription · own login folder" : "Subscription · default login";
  if (d.models_url?.trim()) return "Pay as you go · provider";
  if (harness?.env_vars.some((v) => v.secret && keys.includes(v.name)) || harness?.account_env.some((n) => keys.includes(n))) return "Pay as you go · API key";
  return "Pay as you go · native configuration";
}

const useSignIn = () => useQuery({ queryKey: ["sign-in"], queryFn: () => api.get<SignInCheck[]>("/api/setup/sign-in"), staleTime: 60_000 });

/** Whether a profile can run on one machine: installed and signed in, from the last check. */
type Machine = { host: string | null; label: string; installed: boolean | null; signedIn: boolean | null; detail: string; command: string | null; reachable: boolean };

function useMachines(d: Draft): Machine[] {
  const local = useSignIn().data;
  const hosts = useHosts();
  const own = d.env.length > 0;
  const mine = local?.find((c) => (own ? c.profile === d.id : c.profile === null && c.id === d.harness));
  const rows: Machine[] = [{
    host: null, label: "This computer", reachable: true,
    installed: mine ? mine.cli_found : null, signedIn: mine?.cli_found ? mine.signed_in : null,
    detail: !mine ? "Checking…" : !mine.cli_found ? "Not installed" : mine.signed_in === true ? `Signed in${mine.detail ? ` · ${mine.detail}` : ""}` : mine.signed_in === false ? `Not signed in${mine.detail ? ` · ${mine.detail}` : ""}` : `Sign-in unknown${mine.detail ? ` · ${mine.detail}` : ""}`,
    command: mine && mine.signed_in !== true ? mine.command : null,
  }];
  for (const h of hosts) {
    const c = h.checks.find((x) => x.id === (own ? `profile:${d.id}` : d.harness));
    const reachable = h.state === "connected";
    rows.push({
      host: h.host.id, label: h.host.id, reachable,
      installed: c?.found ?? null, signedIn: c?.signed_in ?? null,
      detail: !reachable ? (h.state === "connecting" ? "Connecting…" : "Not connected") : !c ? "Not checked yet" : c.detail,
      command: c?.command ?? null,
    });
  }
  return rows;
}

const machineTone = (m: Machine) => (!m.reachable || m.installed === null ? "bg-ink-3" : m.installed && m.signedIn ? "bg-gh-open" : m.installed && m.signedIn === null ? "bg-attn" : "bg-gh-closed");

export function ProfilesView() {
  const q = useConfig();
  const qc = useQueryClient();
  const harnesses = useHarnesses();
  const [drafts, setDrafts] = useState<Draft[] | null>(null);
  const [open, setOpen] = useState<Set<string>>(() => new Set());
  const [adding, setAdding] = useState(false);
  useEffect(() => {
    if (q.data && !drafts) setDrafts(q.data.config.profiles.map(toDraft));
  }, [q.data, drafts]);
  if (!drafts || !q.data) return <SettingsPage><div className="text-ink-3">Loading…</div></SettingsPage>;
  const saved = q.data.config;
  const invalid = drafts.map(problem).find(Boolean) ?? null;
  const toProfiles = (list: Draft[]): Profile[] => list.map(({ env, budget, limits, saved: _, template: __, key: ___, ...v }) => ({
    ...v,
    name: v.name.trim(),
    env: Object.fromEntries(env.map(([k, x]) => [k.trim(), x])),
    budget_usd: harnesses.find((h) => h.id === v.harness)?.capabilities.cost ? (parseUsd(budget) ?? null) : null,
    models_url: v.models_url?.trim() || null,
    // Limits apply only to pay-as-you-go profiles of harnesses that report costs.
    spend_limits: v.billing === "pay_as_you_go" && harnesses.find((h) => h.id === v.harness)?.capabilities.cost ? limits.map(parseLimit).filter((l): l is SpendLimit => !!l) : [],
  }));
  const dirty = JSON.stringify(toProfiles(drafts).map(canonical)) !== JSON.stringify(saved.profiles.map(canonical));
  const save = async () => {
    // New profiles get a stable ID from their name; saved IDs never change.
    const taken = drafts.filter((d) => d.id).map((d) => d.id);
    const list = drafts.map((d) => {
      if (d.id) return d;
      const id = slug(d.name, taken);
      taken.push(id);
      return { ...d, id };
    });
    try {
      const res = await api.put<ConfigResponse>("/api/config", { ...saved, profiles: toProfiles(list) });
      qc.setQueryData(["config"], res);
      const next = res.config.profiles.map(toDraft);
      // Keep the cards that were open open, under their saved keys.
      setOpen((o) => new Set(list.flatMap((d, i) => (o.has(d.key) && next[i] ? [next[i].key] : []))));
      setDrafts(next);
      void qc.invalidateQueries({ queryKey: ["sign-in"] });
      toast("Profiles saved");
    } catch (e) {
      toastError(e);
    }
  };
  const setAt = (key: string, f: (d: Draft) => Draft) => setDrafts((all) => all && all.map((d) => (d.key === key ? f(d) : d)));
  const toggle = (key: string) => setOpen((o) => { const n = new Set(o); if (n.has(key)) n.delete(key); else n.add(key); return n; });
  const add = (h: HarnessDescriptor, t: ProfileTemplate | null) => {
    const taken = drafts.map((d) => d.name.trim());
    const base = t ? `${h.name} · ${t.name}` : "";
    let name = base;
    for (let i = 2; name && taken.includes(name); i++) name = `${base} ${i}`;
    const billing = t?.billing ?? (h.capabilities.quota ? "subscription" : "pay_as_you_go");
    const key = `new-${nextKey++}`;
    setDrafts((all) => all && [...all, { key, id: "", name, harness: h.id, env: t ? t.env.map(([k, v]) => [k, v]) : [], model: null, effort: null, billing, budget: "", limits: [], models_url: t?.models_url ?? null, send_credentials: false, saved: false, template: t }]);
    setOpen((o) => new Set(o).add(key));
    setAdding(false);
  };
  // A card with a problem opens so the owner sees what to fix.
  const isOpen = (d: Draft) => open.has(d.key) || !d.saved || (!!invalid && problem(d) === invalid);

  return (
    <SettingsPage>
      {q.data.error && <div className="rounded-lg bg-attn-soft px-3 py-2 text-xs text-attn">config.toml has an error, the previous settings are in use: {q.data.error}</div>}
      <Card
        title="Profiles"
        end={!adding && drafts.length > 0 && <button className="btn btn-sm" onClick={() => setAdding(true)}><Icon name="plus" size={12} />New profile</button>}
      >
        <p className="text-[12.5px] text-ink-2">A profile is one way to run a coding agent: a harness (Claude Code, Codex, OpenCode), what pays for its work (a subscription login, an API key or another provider) and its default model. The composer lists your profiles. Subscription profiles get quota meters and protection, one per login, so several accounts of one harness work side by side; pay-as-you-go profiles get cost and spending limits.</p>
        {(adding || drafts.length === 0) && <NewProfile onAdd={add} onCancel={drafts.length > 0 ? () => setAdding(false) : null} />}
        {drafts.map((d) => (
          <ProfileCard key={d.key} draft={d} open={isOpen(d)} problem={problem(d)} onToggle={() => toggle(d.key)}
            onChange={(f) => setAt(d.key, f)} onDelete={() => setDrafts((all) => all && all.filter((x) => x.key !== d.key))} />
        ))}
      </Card>
      <GitHubCard />
      <SaveBar dirty={dirty} invalid={invalid} onSave={() => void save()} onRevert={() => setDrafts(saved.profiles.map(toDraft))} />
    </SettingsPage>
  );
}

/** Two questions: how the work is paid, then which harness and setup. */
function NewProfile({ onAdd, onCancel }: { onAdd: (h: HarnessDescriptor, t: ProfileTemplate | null) => void; onCancel: (() => void) | null }) {
  const harnesses = useHarnesses();
  const [kind, setKind] = useState<ProfileKind | "custom" | null>(null);
  const templates = harnesses.flatMap((h) => h.profile_templates.filter((t) => t.kind === kind).map((t) => ({ h, t })));
  return (
    <div className="grid gap-2 rounded-lg border border-line bg-chrome p-3">
      <div className="flex items-center gap-2">
        <b className="text-[12.5px]">{kind === null ? "New profile · how is the work paid?" : "New profile · choose a setup"}</b>
        <span className="ml-auto flex gap-1.5">
          {kind !== null && <button className="btn btn-sm" onClick={() => setKind(null)}>Back</button>}
          {onCancel && <button className="btn btn-sm" onClick={onCancel}>Cancel</button>}
        </span>
      </div>
      {kind === null ? (
        <div className="grid gap-px rounded-lg border border-line bg-paper p-[5px]">
          {KINDS.map((k) => <MenuItem key={k.kind} label={k.label} detail={k.detail} onClick={() => setKind(k.kind)} />)}
          <MenuItem label="Custom" detail="Start from a blank profile and choose the environment variables yourself" onClick={() => setKind("custom")} />
        </div>
      ) : (
        <div className="grid gap-px rounded-lg border border-line bg-paper p-[5px]">
          {kind === "custom"
            ? harnesses.map((h) => <MenuItem key={h.id} icon={<HarnessMark harness={h.id} size={14} />} label={`${h.name} · blank profile`} detail="No environment variables yet" onClick={() => onAdd(h, null)} />)
            : templates.map(({ h, t }) => <MenuItem key={`${h.id}/${t.name}`} icon={<HarnessMark harness={h.id} size={14} />} label={`${h.name} · ${t.name}`} detail={t.description} onClick={() => onAdd(h, t)} />)}
        </div>
      )}
    </div>
  );
}

function ProfileCard({ draft: d, open, problem, onToggle, onChange, onDelete }: { draft: Draft; open: boolean; problem: string | null; onToggle: () => void; onChange: (f: (d: Draft) => Draft) => void; onDelete: () => void }) {
  const harnesses = useHarnesses();
  const harness = harnesses.find((h) => h.id === d.harness);
  const machines = useMachines(d);
  const status = useStatus().data;
  const quota = d.saved && d.billing === "subscription" ? quotaFor(status, d.harness, d.id) : null;
  const spend = d.saved ? status?.spend.find((v) => v.profile === d.id) : undefined;
  const ref = useRef<HTMLDivElement>(null);
  // A new profile is added at the end of the list: bring it into view.
  useEffect(() => {
    if (!d.saved) ref.current?.scrollIntoView({ block: "start", behavior: "smooth" });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
  const pct = (q: { used_percent: number } | null) => (q ? `${Math.round(q.used_percent)}%` : "?");
  const summary = [
    source(d, harness),
    d.model ? d.model : null,
    quota ? `5 h ${pct(quota.five_hour)} · week ${pct(quota.weekly)}` : null,
    spend?.windows.length ? spend.windows.map((w) => `${usd(w.used_usd)}/${usd(w.limit_usd)} per ${windowLabel(w.hours)}`).join(" · ") : null,
  ].filter(Boolean).join(" · ");
  return (
    <div ref={ref} className={`@container grid scroll-mt-4 rounded-lg border ${problem && open ? "border-attn-line" : "border-line"}`}>
      <div className="flex min-h-[44px] flex-wrap items-center gap-x-2 gap-y-1 px-2 py-1.5">
        <button className="flex min-w-0 flex-1 basis-[260px] items-center gap-2 rounded-md px-1 py-1 text-left hover:bg-sunk" aria-expanded={open} aria-label={`${open ? "Collapse" : "Expand"} ${d.name || "new profile"}`} onClick={onToggle}>
          <Icon name="chev" size={12} className={`shrink-0 text-ink-3 ${open ? "" : "-rotate-90"}`} />
          {harness && <HarnessMark harness={d.harness} size={16} />}
          <span className="grid min-w-0">
            <b className="truncate text-[13px] font-[650]">{d.name.trim() || "New profile"}{!d.saved && <span className="ml-1.5 font-normal text-ink-3">· unsaved</span>}</b>
            <span className="truncate text-xs text-ink-3" title={summary}>{summary}</span>
          </span>
        </button>
        <span className="flex flex-wrap items-center gap-2 text-xs">
          {d.saved && machines.map((m) => (
            <span key={m.label} className="inline-flex items-center gap-1" title={`${m.label}: ${m.detail}`}>
              <i className={`inline-block size-1.5 rounded-full ${machineTone(m)}`} aria-hidden />
              <span className="text-ink-2">{m.host ?? "This computer"}</span>
            </span>
          ))}
          <button className="iconbtn" title="Delete profile" aria-label={`Delete ${d.name || "profile"}`} onClick={onDelete}><Icon name="trash" size={14} /></button>
        </span>
      </div>
      {open && <ProfileEditor draft={d} problem={problem} onChange={onChange} machines={machines} />}
    </div>
  );
}

function ProfileEditor({ draft: d, problem, onChange, machines }: { draft: Draft; problem: string | null; onChange: (f: (d: Draft) => Draft) => void; machines: Machine[] }) {
  const harnesses = useHarnesses();
  const harness = harnesses.find((h) => h.id === d.harness);
  const defaults = useHarnessDefaults(d.harness).data;
  const ownModels = !!d.models_url?.trim();
  const providerModels = useProfileModels(ownModels ? d.models_url!.trim() : null);
  const model = defaults?.models.find((m) => m.id === (d.model ?? defaults?.model));
  const efforts = ownModels ? [] : model?.efforts.length ? model.efforts : (defaults?.efforts ?? []);
  const budgetValid = parseUsd(d.budget) !== undefined;
  const needsModel = !d.model && usesModelReference(d.env);
  const row = "grid grid-cols-1 @min-[480px]:grid-cols-[130px_1fr] items-start gap-3 text-[12.5px]";
  const [modelUp, setModelUp] = useState(false);
  // Advanced settings start open while the environment still needs values.
  const [advanced, setAdvanced] = useState(() => !d.saved && d.env.length > 0);
  const setHarness = (h: HarnessDescriptor) => onChange((x) => ({ ...x, harness: h.id, model: null, effort: null, billing: h.capabilities.quota ? x.billing : "pay_as_you_go", budget: h.capabilities.cost ? x.budget : "" }));
  const references = d.env.some(([, v]) => /\$\{(?!LAUNCHPAD_MODEL\})/.test(v));
  return (
    <div className="grid gap-3 border-t border-line-2 p-3">
      {problem && <div className="text-xs text-attn">{problem}</div>}
      {d.template && !d.saved && (d.template.setup || d.template.setup_command) && (
        <div className="grid gap-1.5 rounded-md bg-chrome px-3 py-2 text-xs text-ink-2">
          {d.template.setup && <span>{d.template.setup}</span>}
          {d.template.setup_command && (
            <span className="flex flex-wrap items-center gap-2">
              <code className="rounded bg-paper px-1.5 py-0.5 font-mono">{d.template.setup_command}</code>
              <button className="iconbtn" title="Copy the command" aria-label={`Copy ${d.template.setup_command}`} onClick={() => void copyText(d.template!.setup_command!).then(() => toast("Command copied"))}><Icon name="copy" size={12} /></button>
            </span>
          )}
        </div>
      )}
      <div className={row}>
        <b className="pt-1 font-[650]">Name</b>
        <span className="flex flex-wrap items-center gap-2">
          <input className={`field-input h-[30px] min-w-0 flex-1 basis-[180px] ${d.name.trim() ? "" : "border-attn-line!"}`} aria-label="Profile name" placeholder="Name shown in the composer, for example Claude · work" value={d.name} onChange={(e) => onChange((x) => ({ ...x, name: e.target.value }))} />
          <span className="text-xs text-ink-3">{d.saved ? <>ID <code className="font-mono">{d.id}</code></> : "ID set on save"}</span>
        </span>
      </div>
      <div className={row}>
        <b className="pt-1 font-[650]">Harness</b>
        <span className="flex flex-wrap gap-1.5">
          {harnesses.map((h) => <button key={h.id} disabled={d.saved} title={d.saved ? "Saved sessions depend on the harness; create another profile to change it" : undefined} className={`btn btn-sm ${d.harness === h.id ? "bg-select! font-semibold" : ""}`} onClick={() => setHarness(h)}><HarnessMark harness={h.id} size={14} />{h.name}</button>)}
        </span>
      </div>
      <div className={row}>
        <b className="pt-1 font-[650]">Billing</b>
        <span className="grid gap-1">
          {harness?.capabilities.quota && <label className="flex items-start gap-2"><input type="radio" className="mt-0.5 accent-teal" checked={d.billing === "subscription"} onChange={() => onChange((x) => ({ ...x, billing: "subscription" }))} /><span>Subscription <span className="text-ink-3">· Launchpad reads the quota of this profile's login ({harness.account_env.length ? `${harness.account_env.join(", ")} selects another account` : "its default login"}) and pauses work at its limits. Paid usage stays off unless allowed per session.</span></span></label>}
          <label className="flex items-start gap-2"><input type="radio" className="mt-0.5 accent-teal" checked={d.billing === "pay_as_you_go" || !harness?.capabilities.quota} onChange={() => onChange((x) => ({ ...x, billing: "pay_as_you_go" }))} /><span>Pay as you go <span className="text-ink-3">· no subscription quota protection{harness?.capabilities.cost ? "; use a cost limit and spending limits" : ""}.</span></span></label>
        </span>
      </div>
      <div className={row}>
        <b className="pt-1 font-[650]">Default model</b>
        <span className="grid gap-1">
          <span className="flex flex-wrap gap-2">
            {ownModels ? (
              <Popover width={400} up={modelUp} trigger={({ toggle }) => <button className={`btn btn-sm max-w-full font-mono ${needsModel ? "border-attn-line!" : ""}`} onClick={(e) => (setModelUp(lowerHalf(e)), toggle())}><span className="truncate">{d.model ?? "Choose a model"}</span><Icon name="chev" size={11} /></button>}>
                {(close) => (
                  <>
                    {d.model && !usesModelReference(d.env) && <MenuItem label="No default model" detail="The harness picks its own" onClick={() => (onChange((x) => ({ ...x, model: null })), close())} />}
                    <ModelSearch models={providerModels.data ?? []} loading={providerModels.isLoading} error={providerModels.error?.message ?? null} active={d.model} onPick={(id) => (onChange((x) => ({ ...x, model: pickModel(providerModels.data, x.model, id), effort: null })), close())} />
                  </>
                )}
              </Popover>
            ) : (
              <select className="field-input h-[30px] w-auto text-xs" aria-label="Default model" value={d.model ?? ""} onChange={(e) => onChange((x) => ({ ...x, model: e.target.value || null, effort: null }))}>
                <option value="">Harness default{defaults?.model ? ` (${defaults.model})` : ""}</option>
                {d.model && !defaults?.models.some((m) => m.id === d.model) && <option value={d.model}>{d.model}</option>}
                {(defaults?.models ?? []).map((m) => <option key={m.id} value={m.id}>{m.display_name || m.id}</option>)}
              </select>
            )}
            {efforts.length > 0 && <select className="field-input h-[30px] w-auto text-xs" aria-label="Default effort" value={d.effort ?? ""} onChange={(e) => onChange((x) => ({ ...x, effort: e.target.value || null }))}>
              <option value="">Default effort</option>
              {efforts.map((e) => <option key={e} value={e}>{e}</option>)}
            </select>}
          </span>
          {needsModel && <span className="text-xs text-attn">Required: the environment uses <code className="font-mono">{"${LAUNCHPAD_MODEL}"}</code>. The composer can still pick another model per launch.</span>}
        </span>
      </div>
      {ownModels && !!findModel(providerModels.data, d.model)?.providers.length && <div className={row}>
        <b className="pt-1 font-[650]">Provider</b>
        <ProviderPicker models={providerModels.data} model={d.model} up={modelUp} onChange={(id) => onChange((x) => ({ ...x, model: id }))} />
      </div>}
      {harness?.capabilities.cost && <div className={row}>
        <b className="pt-1 font-[650]">Default cost limit</b>
        <span className="flex flex-wrap items-center gap-2">
          <input className="field-input w-28 font-mono text-xs" aria-label="Default cost limit in USD" placeholder="No limit" value={d.budget} onChange={(e) => onChange((x) => ({ ...x, budget: e.target.value }))} />
          <span className={`text-xs ${budgetValid ? "text-ink-3" : "text-attn"}`}>{budgetValid ? `USD per session, prefilled in the composer and editable on each session.${ownModels ? ` ${harness.name} estimates costs with its own prices, which may not match other providers.` : ""}` : "Enter a positive amount, or leave empty."}</span>
        </span>
      </div>}
      {harness?.capabilities.cost && d.billing === "pay_as_you_go" && <div className={row}>
        <b className="pt-1 font-[650]">Spending limits</b>
        <SpendLimits draft={d} onChange={(limits) => onChange((x) => ({ ...x, limits }))} />
      </div>}
      <div className={row}>
        <b className="pt-1 font-[650]">Works on</b>
        <WorksOn draft={d} machines={machines} />
      </div>
      <div className="grid gap-3">
        <button className="flex w-fit items-center gap-1.5 text-[12.5px] font-[650] text-ink-2 hover:text-ink" aria-expanded={advanced} onClick={() => setAdvanced((x) => !x)}>
          <Icon name="chev" size={12} className={advanced ? "" : "-rotate-90"} />Advanced · environment{d.env.length ? ` (${d.env.length})` : ""}, model list, SSH hosts
        </button>
        {advanced && <>
          <div className={row}>
            <b className="pt-1 font-[650]">Environment</b>
            <span className="grid gap-1.5">
              {d.env.map(([k, v], j) => (
                <EnvRow key={j} name={k} value={v} doc={harness?.env_vars.find((x) => x.name === k.trim())}
                  onName={(name) => onChange((x) => ({ ...x, env: x.env.map((p, n) => (n === j ? [name, p[1]] : p)) }))}
                  onValue={(value) => onChange((x) => ({ ...x, env: x.env.map((p, n) => (n === j ? [p[0], value] : p)) }))}
                  onRemove={() => onChange((x) => ({ ...x, env: x.env.filter((_, n) => n !== j) }))} />
              ))}
              <AddVariable harness={d.harness} docs={harness?.env_vars ?? []} used={d.env.map(([k]) => k.trim())} onAdd={(name) => onChange((x) => ({ ...x, env: [...x.env, [name, ""]] }))} />
              <span className="text-xs text-ink-3">Added to the harness process, its terminal TUI and title requests. Other variables come from the login shell of the machine that runs it. In values, a leading <code className="font-mono">~/</code> is the home folder of that machine, <code className="font-mono">{"${NAME}"}</code> reads a variable of a login shell (<code className="font-mono">{"${HF_TOKEN}"}</code> also reads the hf CLI login), and <code className="font-mono">{"${LAUNCHPAD_MODEL}"}</code> is the model of the session.</span>
            </span>
          </div>
          <div className={row}>
            <b className="pt-1 font-[650]">Model list</b>
            <span className="grid gap-1">
              <input className="field-input h-[28px] min-w-0 font-mono text-xs" aria-label="Model list URL" placeholder="Optional: https://…/v1/models" value={d.models_url ?? ""} onChange={(e) => onChange((x) => ({ ...x, models_url: e.target.value || null }))} />
              <span className="text-xs text-ink-3">An OpenAI-compatible model list whose models replace the {harness?.name ?? "harness"} models in the composer, for a profile that uses another provider.{providerModels.data ? ` ${providerModels.data.length} models with tool use.` : ""}</span>
            </span>
          </div>
          <div className={row}>
            <b className="pt-1 font-[650]">SSH hosts</b>
            <label className="flex items-start gap-2">
              <input type="checkbox" className="mt-0.5 accent-teal" checked={d.send_credentials} onChange={(e) => onChange((x) => ({ ...x, send_credentials: e.target.checked }))} />
              <span>Send credentials to SSH hosts <span className="text-ink-3">· {"${NAME}"} references{references ? "" : " (none in this profile)"} are resolved on this computer and sent with each process start on a host, so the host needs no copy of the key. When off, each host resolves them from its own login shell. Literal values are always sent; subscription logins (sign-in files) are never copied.</span></span>
            </label>
          </div>
        </>}
      </div>
    </div>
  );
}

/** Where this profile can run: this computer and each SSH host, with sign-in and install actions. */
function WorksOn({ draft: d, machines }: { draft: Draft; machines: Machine[] }) {
  const qc = useQueryClient();
  const hosts = useHosts();
  const terminal = useOpenTerminal();
  const signIn = useSignIn();
  const [checking, setChecking] = useState(false);
  if (!d.saved) return <span className="pt-1 text-xs text-ink-3">Save the profile to check where it is installed and signed in.</span>;
  const recheck = async () => {
    setChecking(true);
    try {
      await Promise.all([signIn.refetch(), ...hosts.filter((h) => h.state === "connected").map((h) => api.post(`/api/hosts/${h.host.id}/checks`))]);
      void qc.invalidateQueries({ queryKey: ["status"] });
    } catch (e) {
      toastError(e);
    } finally {
      setChecking(false);
    }
  };
  return (
    <span className="grid gap-1.5">
      <span className="grid rounded-md border border-line">
        {machines.map((m) => (
          <span key={m.label} className="grid gap-1 border-b border-line-2 px-2.5 py-1.5 text-xs last:border-0">
            <span className="flex min-w-0 flex-wrap items-center gap-2">
              <i className={`inline-block size-2 shrink-0 rounded-full ${machineTone(m)}`} aria-hidden />
              <b className="font-[650]">{m.label}</b>
              <span className="min-w-0 flex-1 truncate text-ink-3" title={m.detail}>{m.detail}</span>
              {m.reachable && m.installed === false && <button className="btn btn-sm" title="Runs the official install command in a visible terminal" onClick={() => void terminal.profileSetup(d.id, m.host, true)}><Icon name="term" size={12} />Install</button>}
              {m.reachable && m.installed !== false && m.signedIn !== true && <button className="btn btn-sm" title="Runs the sign-in command with this profile's environment in a visible terminal" onClick={() => void terminal.profileSetup(d.id, m.host)}><Icon name="term" size={12} />Sign in</button>}
            </span>
            {m.command && m.installed !== false && m.signedIn !== true && (
              <span className="flex flex-wrap items-center gap-2 pl-4">
                <code className="rounded bg-chrome px-1.5 py-0.5 font-mono">{m.command}</code>
                <button className="iconbtn" title="Copy the command" aria-label={`Copy ${m.command}`} onClick={() => void copyText(m.command!).then(() => toast("Command copied"))}><Icon name="copy" size={12} /></button>
              </span>
            )}
          </span>
        ))}
      </span>
      <span className="flex flex-wrap items-center gap-2">
        <button className="btn btn-sm w-fit" disabled={checking || signIn.isFetching} onClick={() => void recheck()}><Icon name="refresh" size={12} />{checking ? "Checking…" : "Check again"}</button>
        <span className="text-xs text-ink-3">Launchpad only reads the status; it never signs in or refreshes a token. Profiles not ready on a host are disabled in the composer for its projects.</span>
      </span>
    </span>
  );
}

function GitHubCard() {
  const q = useSignIn();
  const c = q.data?.find((x) => x.id === "github");
  return (
    <Card title="GitHub CLI">
      <p className="text-[12.5px] text-ink-2">The inbox, issues, pull requests and publication use the GitHub CLI on the computer that runs the daemon. Git push also needs credentials there (SSH keys or <code className="font-mono">gh auth setup-git</code>); projects on SSH hosts push with the host's own credentials (Settings → Hosts).</p>
      {c && (
        <div className="flex flex-wrap items-center gap-2 text-[12.5px]">
          <Icon name="pr" size={14} />
          <b className="font-[650]">GitHub CLI</b>
          <span className={`text-xs font-semibold ${!c.cli_found ? "text-gh-closed" : c.signed_in ? "text-gh-open" : "text-attn"}`}>{!c.cli_found ? "Not installed" : c.signed_in ? "Signed in" : "Not signed in"}</span>
          {c.detail && <span className="text-xs text-ink-3">{c.detail}</span>}
          {(!c.cli_found || !c.signed_in) && <>
            <code className="rounded bg-chrome px-1.5 py-0.5 font-mono text-xs">{c.command}</code>
            <button className="iconbtn" title="Copy the command" aria-label={`Copy ${c.command}`} onClick={() => void copyText(c.command).then(() => toast("Command copied"))}><Icon name="copy" size={12} /></button>
          </>}
        </div>
      )}
    </Card>
  );
}

function EnvRow({ name, value, doc, onName, onValue, onRemove }: { name: string; value: string; doc: EnvVarDoc | undefined; onName: (v: string) => void; onValue: (v: string) => void; onRemove: () => void }) {
  const [shown, setShown] = useState(false);
  // References are not secrets; literal credentials stay masked until shown.
  const masked = !!doc?.secret && !shown && !!value && !value.startsWith("${");
  const placeholder = doc?.default ? `default: ${doc.default}` : doc?.example ? `for example ${doc.example}` : "value";
  return (
    <span className="grid gap-0.5">
      <span className="grid grid-cols-[minmax(0,1fr)_26px] @min-[480px]:grid-cols-[minmax(0,240px)_minmax(0,1fr)_26px] gap-1.5">
        <input className={`field-input col-span-full @min-[480px]:col-span-1 h-[28px] min-w-0 font-mono text-xs ${name.trim() ? "" : "border-attn-line!"}`} aria-label="Variable name" placeholder="NAME" value={name} onChange={(e) => onName(e.target.value.toUpperCase())} />
        <span className="relative flex min-w-0">
          <input type={masked ? "password" : "text"} autoComplete="off" spellCheck={false} className={`field-input h-[28px] min-w-0 flex-1 font-mono text-xs ${doc?.secret ? "pr-7" : ""} ${value.trim() ? "" : "border-attn-line!"}`} aria-label={`Value of ${name}`} placeholder={placeholder} value={value} onChange={(e) => onValue(e.target.value)} />
          {doc?.secret && !!value && !value.startsWith("${") && <button className="absolute top-1/2 right-1 -translate-y-1/2 iconbtn size-5!" title={shown ? "Hide value" : "Show value"} aria-label={shown ? `Hide ${name}` : `Show ${name}`} aria-pressed={shown} onClick={() => setShown((x) => !x)}><Icon name="eye" size={12} /></button>}
        </span>
        <button className="iconbtn" title="Remove variable" onClick={onRemove}><Icon name="x" size={13} /></button>
      </span>
      {doc && <span className="text-[11.5px] text-ink-3">{doc.description}</span>}
    </span>
  );
}

function AddVariable({ harness, docs, used, onAdd }: { harness: HarnessKind; docs: EnvVarDoc[]; used: string[]; onAdd: (name: string) => void }) {
  const [text, setText] = useState("");
  const [up, setUp] = useState(false);
  const words = text.trim().toLowerCase().split(/\s+/).filter(Boolean);
  const available = docs.filter((x) => !used.includes(x.name) && words.every((w) => `${x.name} ${x.group} ${x.description}`.toLowerCase().includes(w)));
  const groups = [...new Set(available.map((x) => x.group))];
  const custom = text.trim().toUpperCase().replace(/[^A-Z0-9_]/g, "_");
  return (
    <Popover width={440} up={up} trigger={({ toggle }) => <button className="btn btn-sm w-fit" onClick={(e) => (setText(""), setUp(lowerHalf(e)), toggle())}><Icon name="plus" size={12} />Add variable<Icon name="chev" size={11} /></button>}>
      {(close) => (
        <>
          <input autoFocus className="field-input mb-1 h-[30px] text-xs" aria-label="Search variables" placeholder={docs.length ? "Search documented variables, or type a name" : "Variable name"} value={text} onChange={(e) => setText(e.target.value)} />
          <MenuItem label={custom ? <>Add <code className="font-mono">{custom}</code></> : "Custom variable"} detail={custom && docs.some((x) => x.name === custom) ? undefined : "Any other variable read by the harness or its tools"} onClick={() => (onAdd(custom), close())} />
          {groups.map((g) => (
            <div key={`${harness}/${g}`} className="grid gap-px">
              <div className="px-2 pt-2 pb-0.5 text-[11px] font-semibold text-ink-3 uppercase">{g}</div>
              {available.filter((x) => x.group === g).map((x) => <MenuItem key={x.name} label={<code className="font-mono">{x.name}</code>} detail={`${x.description}${x.default ? ` Default: ${x.default}.` : ""}`} onClick={() => (onAdd(x.name), close())} />)}
            </div>
          ))}
        </>
      )}
    </Popover>
  );
}

/** Rolling limits over all sessions of the profile, with the spending reported in each window now. */
function SpendLimits({ draft: d, onChange }: { draft: Draft; onChange: (limits: LimitDraft[]) => void }) {
  const spend = useStatus().data?.spend.find((v) => v.profile === d.id && d.saved);
  const set = (j: number, f: (l: LimitDraft) => LimitDraft) => onChange(d.limits.map((l, n) => (n === j ? f(l) : l)));
  return (
    <span className="grid gap-1.5">
      {d.limits.map((l, j) => {
        const parsed = parseLimit(l);
        const now = parsed && spend?.windows.find((w) => w.hours === parsed.hours && w.limit_usd === parsed.usd);
        return (
          <span key={j} className="flex flex-wrap items-center gap-1.5">
            <span className="text-xs text-ink-3">At most $</span>
            <input className={`field-input h-[28px] w-20 font-mono text-xs ${parseUsd(l.usd) ? "" : "border-attn-line!"}`} aria-label="Spending limit in USD" placeholder="50" value={l.usd} onChange={(e) => set(j, (x) => ({ ...x, usd: e.target.value }))} />
            <span className="text-xs text-ink-3">per</span>
            <input className={`field-input h-[28px] w-14 font-mono text-xs ${parsed ? "" : "border-attn-line!"}`} aria-label="Window length" inputMode="numeric" value={l.count} onChange={(e) => set(j, (x) => ({ ...x, count: e.target.value }))} />
            <select className="field-input h-[28px] w-auto text-xs" aria-label="Window unit" value={l.unit} onChange={(e) => set(j, (x) => ({ ...x, unit: e.target.value as Unit }))}>
              <option value="hours">hours</option>
              <option value="days">days</option>
              <option value="weeks">weeks</option>
            </select>
            <button className="iconbtn" title="Remove limit" aria-label="Remove spending limit" onClick={() => onChange(d.limits.filter((_, n) => n !== j))}><Icon name="x" size={13} /></button>
            {now && <span className={`text-xs ${now.used_usd >= now.limit_usd ? "font-semibold text-gh-closed" : "text-ink-3"}`}>${now.used_usd.toFixed(2)} used now</span>}
          </span>
        );
      })}
      {d.limits.length < 4 && <button className="btn btn-sm w-fit" onClick={() => onChange([...d.limits, d.limits.length ? { usd: "200", count: "1", unit: "weeks" } : { usd: "50", count: "5", unit: "hours" }])}><Icon name="plus" size={12} />Add limit</button>}
      <span className="text-xs text-ink-3">Rolling windows over the reported cost of all sessions of this profile, shown in the status bar. At a limit, new turns are refused and running turns stop (best effort) until older costs leave the window or you raise the limit. Costs are those reported by the harness, so the provider bill may differ.</span>
    </span>
  );
}
