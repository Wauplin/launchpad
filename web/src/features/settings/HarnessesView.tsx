// Settings → Harnesses: named variants of the registered harnesses, saved in config.toml.
// Variable descriptions, defaults and templates come from the adapters (`GET /api/harnesses`).

import { useQueryClient } from "@tanstack/react-query";
import { useEffect, useRef, useState } from "react";
import type { EnvVarDoc } from "@/gen/EnvVarDoc";
import type { HarnessDescriptor } from "@/gen/HarnessDescriptor";
import type { HarnessKind } from "@/gen/HarnessKind";
import type { HarnessVariant } from "@/gen/HarnessVariant";
import type { SpendLimit } from "@/gen/SpendLimit";
import type { VariantTemplate } from "@/gen/VariantTemplate";
import { HarnessMark, useHarnesses } from "@/components/HarnessMark";
import { Icon } from "@/components/Icon";
import { ModelSearch } from "@/components/ModelSearch";
import { findModel, pickModel, ProviderPicker } from "@/components/ProviderPicker";
import { MenuItem, Popover } from "@/components/Popover";
import { toast, toastError } from "@/components/Toast";
import { api } from "@/lib/api";
import { copyText } from "@/lib/clipboard";
import { useConfig, useHarnessDefaults, useStatus, type ConfigResponse } from "@/lib/queries";
import { parseUsd, usesModelReference, useVariantModels } from "@/lib/variants";
import { Card, SaveBar, SettingsPage } from "./SettingsView";
import { SignInCard } from "./SignInCard";

/** Editable form of a variant: environment rows keep their order and allow empty names while typing. */
type Draft = Omit<HarnessVariant, "env" | "budget_usd" | "spend_limits"> & { env: [string, string][]; budget: string; limits: LimitDraft[]; saved: boolean; template: VariantTemplate | null };

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

const toDraft = (v: HarnessVariant): Draft => ({ ...v, env: Object.entries(v.env).map(([k, x]) => [k, x ?? ""]), budget: v.budget_usd == null ? "" : String(v.budget_usd), limits: v.spend_limits.map(toLimitDraft), saved: true, template: null });

/** Key order does not matter in the comparison with the saved variants. */
const canonical = (v: HarnessVariant) => [v.id, v.name, v.harness, Object.entries(v.env).sort(), v.model, v.effort, v.billing, v.budget_usd, v.models_url, v.spend_limits.map((l) => [l.usd, l.hours])];

function slug(name: string, taken: string[]) {
  const base = name.toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-+|-+$/g, "").slice(0, 32) || "variant";
  let id = base;
  for (let i = 2; taken.includes(id); i++) id = `${base}-${i}`;
  return id;
}

/** Open popovers upward from triggers low in the window, so they stay visible. */
const lowerHalf = (e: React.MouseEvent) => e.currentTarget.getBoundingClientRect().top > window.innerHeight / 2;

/** The first reason a draft cannot be saved. */
function problem(d: Draft): string | null {
  const label = d.name.trim() || "each variant";
  if (!d.name.trim()) return "Each variant needs a name.";
  if (parseUsd(d.budget) === undefined) return `Fix the cost limit of ${label}.`;
  if (d.limits.some((l) => !parseLimit(l))) return `Fix the spending limits of ${label}: a positive amount per 1 hour to 31 days.`;
  if (d.env.some(([k]) => !k.trim())) return `Name every environment variable of ${label}.`;
  if (new Set(d.env.map(([k]) => k.trim())).size !== d.env.length) return `${label} sets a variable twice.`;
  const empty = d.env.find(([, v]) => !v.trim());
  if (empty) return `Enter a value for ${empty[0]} in ${label}, or remove it.`;
  if (!d.model && usesModelReference(d.env)) return `Choose a default model for ${label}.`;
  return null;
}

export function HarnessesView() {
  const q = useConfig();
  const qc = useQueryClient();
  const harnesses = useHarnesses();
  const [drafts, setDrafts] = useState<Draft[] | null>(null);
  useEffect(() => {
    if (q.data && !drafts) setDrafts(q.data.config.variants.map(toDraft));
  }, [q.data, drafts]);
  if (!drafts || !q.data) return <SettingsPage><div className="text-ink-3">Loading…</div></SettingsPage>;
  const saved = q.data.config;
  const invalid = drafts.map(problem).find(Boolean) ?? null;
  const toVariants = (list: Draft[]): HarnessVariant[] => list.map(({ env, budget, limits, saved: _, template: __, ...v }) => ({
    ...v,
    name: v.name.trim(),
    env: Object.fromEntries(env.map(([k, x]) => [k.trim(), x])),
    budget_usd: harnesses.find((h) => h.id === v.harness)?.capabilities.cost ? (parseUsd(budget) ?? null) : null,
    models_url: v.models_url?.trim() || null,
    // Limits apply only to pay-as-you-go variants of harnesses that report costs.
    spend_limits: v.billing === "pay_as_you_go" && harnesses.find((h) => h.id === v.harness)?.capabilities.cost ? limits.map(parseLimit).filter((l): l is SpendLimit => !!l) : [],
  }));
  const dirty = JSON.stringify(toVariants(drafts).map(canonical)) !== JSON.stringify(saved.variants.map(canonical));
  const save = async () => {
    // New variants get a stable ID from their name; saved IDs never change.
    const taken = drafts.filter((d) => d.id).map((d) => d.id);
    const list = drafts.map((d) => {
      if (d.id) return d;
      const id = slug(d.name, taken);
      taken.push(id);
      return { ...d, id };
    });
    try {
      const res = await api.put<ConfigResponse>("/api/config", { ...saved, variants: toVariants(list) });
      qc.setQueryData(["config"], res);
      setDrafts(res.config.variants.map(toDraft));
      toast("Harness variants saved");
    } catch (e) {
      toastError(e);
    }
  };
  const setAt = (i: number, f: (d: Draft) => Draft) => setDrafts((all) => all && all.map((d, j) => (j === i ? f(d) : d)));
  const add = (h: HarnessDescriptor, t: VariantTemplate | null) => {
    const taken = drafts.map((d) => d.name.trim());
    let name = t?.name ?? "";
    for (let i = 2; name && taken.includes(name); i++) name = `${t!.name} ${i}`;
    const billing = t?.billing ?? (h.capabilities.quota ? "subscription" : "pay_as_you_go");
    setDrafts((all) => all && [...all, { id: "", name, harness: h.id, env: t ? t.env.map(([k, v]) => [k, v]) : [], model: null, effort: null, billing, budget: "", limits: [], models_url: t?.models_url ?? null, saved: false, template: t }]);
  };
  const templates = harnesses.flatMap((h) => h.variant_templates.map((t) => ({ h, t })));
  const templateItem = ({ h, t }: { h: HarnessDescriptor; t: VariantTemplate }, close?: () => void) => (
    <MenuItem key={`${h.id}/${t.name}`} icon={<HarnessMark harness={h.id} size={14} />} label={`${h.name} · ${t.name}`} detail={t.description} onClick={() => (add(h, t), close?.())} />
  );

  return (
    <SettingsPage>
      <SignInCard />
      {q.data.error && <div className="rounded-lg bg-attn-soft px-3 py-2 text-xs text-attn">config.toml has an error, the previous settings are in use: {q.data.error}</div>}
      <Card
        title="Harness variants"
        end={
          <Popover align="right" width={380} trigger={({ toggle }) => <button className="btn btn-sm" onClick={toggle}><Icon name="plus" size={12} />New variant<Icon name="chev" size={11} /></button>}>
            {(close) => (
              <>
                <div className="px-2 pt-1 pb-0.5 text-[11px] font-semibold text-ink-3 uppercase">Templates</div>
                {templates.map((x) => templateItem(x, close))}
                <div className="px-2 pt-2 pb-0.5 text-[11px] font-semibold text-ink-3 uppercase">Blank</div>
                {harnesses.map((h) => <MenuItem key={h.id} icon={<HarnessMark harness={h.id} size={14} />} label={`${h.name} · blank variant`} detail="Choose the environment variables yourself" onClick={() => (add(h, null), close())} />)}
              </>
            )}
          </Popover>
        }
      >
        <p className="text-[12.5px] text-ink-2">A variant runs a registered harness with its own environment variables, default model and effort, and billing mode. It appears beside the harnesses in the composer. Use it for another account, an API key or another model provider. Values are saved in config.toml (readable only by you); sessions store the variant ID, so deleting or renaming the ID prevents resuming them.</p>
        {drafts.length === 0 && (
          <div className="grid gap-1.5">
            <div className="text-xs text-ink-3">No variant yet. Start from a template:</div>
            <div className="grid gap-px rounded-lg border border-line p-[5px]">{templates.map((x) => templateItem(x))}</div>
          </div>
        )}
        {drafts.map((d, i) => <VariantEditor key={i} draft={d} onChange={(f) => setAt(i, f)} onDelete={() => setDrafts((all) => all && all.filter((_, j) => j !== i))} />)}
      </Card>
      <SaveBar dirty={dirty} invalid={invalid} onSave={() => void save()} onRevert={() => setDrafts(saved.variants.map(toDraft))} />
    </SettingsPage>
  );
}

function VariantEditor({ draft: d, onChange, onDelete }: { draft: Draft; onChange: (f: (d: Draft) => Draft) => void; onDelete: () => void }) {
  const harnesses = useHarnesses();
  const harness = harnesses.find((h) => h.id === d.harness);
  const defaults = useHarnessDefaults(d.harness).data;
  const ownModels = !!d.models_url?.trim();
  const providerModels = useVariantModels(ownModels ? d.models_url!.trim() : null);
  const model = defaults?.models.find((m) => m.id === (d.model ?? defaults?.model));
  const efforts = ownModels ? [] : model?.efforts.length ? model.efforts : (defaults?.efforts ?? []);
  const budgetValid = parseUsd(d.budget) !== undefined;
  const needsModel = !d.model && usesModelReference(d.env);
  const row = "grid grid-cols-1 @min-[480px]:grid-cols-[130px_1fr] items-start gap-3 text-[12.5px]";
  const ref = useRef<HTMLDivElement>(null);
  const [modelUp, setModelUp] = useState(false);
  // A new variant is added at the end of the list: bring it into view.
  useEffect(() => {
    if (!d.saved) ref.current?.scrollIntoView({ block: "start", behavior: "smooth" });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
  const setHarness = (h: HarnessDescriptor) => onChange((x) => ({ ...x, harness: h.id, model: null, effort: null, billing: h.capabilities.quota ? x.billing : "pay_as_you_go", budget: h.capabilities.cost ? x.budget : "" }));
  return (
    <div ref={ref} className="@container grid scroll-mt-4 gap-2.5 rounded-lg border border-line p-3">
      <div className="flex flex-wrap items-center gap-2">
        {harness && <HarnessMark harness={d.harness} size={16} />}
        <input className={`field-input h-[30px] min-w-0 flex-1 basis-[180px] font-semibold ${d.name.trim() ? "" : "border-attn-line!"}`} aria-label="Variant name" placeholder="Name shown in the composer, for example Claude · work" value={d.name} onChange={(e) => onChange((x) => ({ ...x, name: e.target.value }))} />
        <span className="text-xs text-ink-3">{d.saved ? <>ID <code className="font-mono">{d.id}</code></> : "ID set on save"}</span>
        <button className="iconbtn" title="Delete variant" onClick={onDelete}><Icon name="trash" size={14} /></button>
      </div>
      {d.template && !d.saved && (
        <div className="grid gap-1.5 rounded-md bg-chrome px-3 py-2 text-xs text-ink-2">
          <span>{d.template.description}</span>
          {d.template.setup && <span>{d.template.setup}</span>}
          {d.template.setup_command && (
            <span className="flex flex-wrap items-center gap-2">
              <code className="rounded bg-paper px-1.5 py-0.5 font-mono">{d.template.setup_command}</code>
              <button className="iconbtn" title="Copy the command" aria-label={`Copy ${d.template.setup_command}`} onClick={() => void copyText(d.template!.setup_command!).then(() => toast("Command copied"))}><Icon name="copy" size={12} /></button>
            </span>
          )}
          <span className="text-ink-3">Review the settings below, then save.</span>
        </div>
      )}
      <div className={row}>
        <b className="pt-1 font-[650]">Harness</b>
        <span className="flex flex-wrap gap-1.5">
          {harnesses.map((h) => <button key={h.id} disabled={d.saved} title={d.saved ? "Saved sessions depend on the base harness; create another variant to change it" : undefined} className={`btn btn-sm ${d.harness === h.id ? "bg-select! font-semibold" : ""}`} onClick={() => setHarness(h)}><HarnessMark harness={h.id} size={14} />{h.name}</button>)}
        </span>
      </div>
      <div className={row}>
        <b className="pt-1 font-[650]">Billing</b>
        <span className="grid gap-1">
          {harness?.capabilities.quota && <label className="flex items-start gap-2"><input type="radio" className="mt-0.5 accent-teal" checked={d.billing === "subscription"} onChange={() => onChange((x) => ({ ...x, billing: "subscription" }))} /><span>Subscription <span className="text-ink-3">· protected by the {harness.name} quota monitor, so use it only with the same account. Paid usage stays off unless allowed per session.</span></span></label>}
          <label className="flex items-start gap-2"><input type="radio" className="mt-0.5 accent-teal" checked={d.billing === "pay_as_you_go" || !harness?.capabilities.quota} onChange={() => onChange((x) => ({ ...x, billing: "pay_as_you_go" }))} /><span>Pay as you go <span className="text-ink-3">· no subscription quota protection{harness?.capabilities.cost ? "; use a cost limit" : ""}.</span></span></label>
        </span>
      </div>
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
      <div className={row}>
        <b className="pt-1 font-[650]">Model list</b>
        <span className="grid gap-1">
          <input className="field-input h-[28px] min-w-0 font-mono text-xs" aria-label="Model list URL" placeholder="Optional: https://…/v1/models" value={d.models_url ?? ""} onChange={(e) => onChange((x) => ({ ...x, models_url: e.target.value || null }))} />
          <span className="text-xs text-ink-3">An OpenAI-compatible model list whose models replace the {harness?.name ?? "harness"} models in the composer, for a variant that uses another provider.{providerModels.data ? ` ${providerModels.data.length} models with tool use.` : ""}</span>
        </span>
      </div>
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
          <span className="text-xs text-ink-3">Added to the harness process, its terminal TUI and title requests. Other variables come from your login shell. In values, a leading <code className="font-mono">~/</code> is your home folder, <code className="font-mono">{"${NAME}"}</code> reads a variable of your login shell (<code className="font-mono">{"${HF_TOKEN}"}</code> also reads the hf CLI login), and <code className="font-mono">{"${LAUNCHPAD_MODEL}"}</code> is the model of the session.</span>
        </span>
      </div>
    </div>
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

/** Rolling limits over all sessions of the variant, with the spending reported in each window now. */
function SpendLimits({ draft: d, onChange }: { draft: Draft; onChange: (limits: LimitDraft[]) => void }) {
  const spend = useStatus().data?.spend.find((v) => v.variant === d.id && d.saved);
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
      <span className="text-xs text-ink-3">Rolling windows over the reported cost of all sessions of this variant, shown in the status bar. At a limit, new turns are refused and running turns stop (best effort) until older costs leave the window or you raise the limit. Costs are those reported by the harness, so the provider bill may differ.</span>
    </span>
  );
}
