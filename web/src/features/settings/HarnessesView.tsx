// Settings → Harnesses: named variants of the registered harnesses, saved in config.toml.

import { useQueryClient } from "@tanstack/react-query";
import { useEffect, useState } from "react";
import type { Config } from "@/gen/Config";
import type { HarnessVariant } from "@/gen/HarnessVariant";
import { HarnessMark, useHarnesses } from "@/components/HarnessMark";
import { Icon } from "@/components/Icon";
import { toast, toastError } from "@/components/Toast";
import { api } from "@/lib/api";
import { useConfig, useHarnessDefaults } from "@/lib/queries";
import { parseUsd } from "@/lib/variants";
import { Card, SaveBar, SettingsPage } from "./SettingsView";

/** Editable form of a variant: environment rows keep their order and allow empty names while typing. */
type Draft = Omit<HarnessVariant, "env" | "budget_usd"> & { env: [string, string][]; budget: string; saved: boolean };

const toDraft = (v: HarnessVariant): Draft => ({ ...v, env: Object.entries(v.env).map(([k, x]) => [k, x ?? ""]), budget: v.budget_usd == null ? "" : String(v.budget_usd), saved: true });

/** Key order does not matter in the comparison with the saved variants. */
const canonical = (v: HarnessVariant) => [v.id, v.name, v.harness, Object.entries(v.env).sort(), v.model, v.effort, v.billing, v.budget_usd];

function slug(name: string, taken: string[]) {
  const base = name.toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-+|-+$/g, "").slice(0, 32) || "variant";
  let id = base;
  for (let i = 2; taken.includes(id); i++) id = `${base}-${i}`;
  return id;
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
  const invalid = drafts.some((d) => !d.name.trim() || parseUsd(d.budget) === undefined || d.env.some(([k]) => !k.trim()) || new Set(d.env.map(([k]) => k.trim())).size !== d.env.length);
  const toVariants = (list: Draft[]): HarnessVariant[] => list.map(({ env, budget, saved: _, ...v }) => ({
    ...v,
    name: v.name.trim(),
    env: Object.fromEntries(env.map(([k, x]) => [k.trim(), x])),
    budget_usd: harnesses.find((h) => h.id === v.harness)?.capabilities.cost ? (parseUsd(budget) ?? null) : null,
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
      const res = await api.put<{ config: Config }>("/api/config", { ...saved, variants: toVariants(list) });
      qc.setQueryData(["config"], { config: res.config, error: null });
      setDrafts(res.config.variants.map(toDraft));
      toast("Harness variants saved");
    } catch (e) {
      toastError(e);
    }
  };
  const setAt = (i: number, f: (d: Draft) => Draft) => setDrafts((all) => all && all.map((d, j) => (j === i ? f(d) : d)));
  const add = () => {
    const harness = harnesses[0]?.id;
    if (!harness) return;
    setDrafts((all) => all && [...all, { id: "", name: "", harness, env: [], model: null, effort: null, billing: harnesses[0]?.capabilities.quota ? "subscription" : "pay_as_you_go", budget: "", saved: false }]);
  };

  return (
    <SettingsPage>
      {q.data.error && <div className="rounded-lg bg-attn-soft px-3 py-2 text-xs text-attn">config.toml has an error, the previous settings are in use: {q.data.error}</div>}
      <Card
        title="Harness variants"
        end={
          <>
            <button className="btn btn-sm" onClick={add}><Icon name="plus" size={12} />New variant</button>
          </>
        }
      >
        <p className="text-[12.5px] text-ink-2">A variant runs a registered harness with its own environment variables, default model and effort, and billing mode. It appears beside the harnesses in the composer. Use it for another account (<code className="font-mono">CLAUDE_CONFIG_DIR</code>, <code className="font-mono">CODEX_HOME</code>), an API key or a provider endpoint. Values are saved in config.toml (readable only by you); sessions store the variant ID, so deleting or renaming the ID prevents resuming them.</p>
        {drafts.length === 0 && <div className="text-xs text-ink-3">No variant yet.</div>}
        {drafts.map((d, i) => <VariantEditor key={i} fileEnvironment={!!saved.server.public_origin} draft={d} onChange={(f) => setAt(i, f)} onDelete={() => setDrafts((all) => all && all.filter((_, j) => j !== i))} />)}
      </Card>
      <SaveBar dirty={dirty} invalid={invalid ? "Each variant needs a name, a valid cost limit and unique variable names." : null} onSave={() => void save()} onRevert={() => setDrafts(saved.variants.map(toDraft))} />
    </SettingsPage>
  );
}

function VariantEditor({ draft: d, onChange, onDelete, fileEnvironment }: { fileEnvironment: boolean; draft: Draft; onChange: (f: (d: Draft) => Draft) => void; onDelete: () => void }) {
  const harnesses = useHarnesses();
  const harness = harnesses.find((h) => h.id === d.harness);
  const defaults = useHarnessDefaults(d.harness).data;
  const model = defaults?.models.find((m) => m.id === (d.model ?? defaults?.model));
  const efforts = model?.efforts.length ? model.efforts : (defaults?.efforts ?? []);
  const budgetValid = parseUsd(d.budget) !== undefined;
  const row = "grid grid-cols-1 @min-[480px]:grid-cols-[130px_1fr] items-start gap-3 text-[12.5px]";
  return (
    <div className="@container grid gap-2.5 rounded-lg border border-line p-3">
      <div className="flex flex-wrap items-center gap-2">
        {harness && <HarnessMark harness={d.harness} size={16} />}
        <input className={`field-input h-[30px] min-w-0 flex-1 basis-[180px] font-semibold ${d.name.trim() ? "" : "border-attn-line!"}`} aria-label="Variant name" placeholder="Name shown in the composer, for example Claude · work" value={d.name} onChange={(e) => onChange((x) => ({ ...x, name: e.target.value }))} />
        <span className="text-xs text-ink-3">{d.saved ? <>ID <code className="font-mono">{d.id}</code></> : "ID set on save"}</span>
        <button className="iconbtn" title="Delete variant" onClick={onDelete}><Icon name="trash" size={14} /></button>
      </div>
      <div className={row}>
        <b className="pt-1 font-[650]">Harness</b>
        <span className="flex flex-wrap gap-1.5">
          {harnesses.map((h) => <button key={h.id} disabled={d.saved} title={d.saved ? "Saved sessions depend on the base harness; create another variant to change it" : undefined} className={`btn btn-sm ${d.harness === h.id ? "bg-select! font-semibold" : ""}`} onClick={() => onChange((x) => ({ ...x, harness: h.id, model: null, effort: null, billing: h.capabilities.quota ? x.billing : "pay_as_you_go", budget: h.capabilities.cost ? x.budget : "" }))}><HarnessMark harness={h.id} size={14} />{h.name}</button>)}
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
          <span className={`text-xs ${budgetValid ? "text-ink-3" : "text-attn"}`}>{budgetValid ? "USD per session, prefilled in the composer and editable on each session." : "Enter a positive amount, or leave empty."}</span>
        </span>
      </div>}
      <div className={row}>
        <b className="pt-1 font-[650]">Default model</b>
        <span className="flex flex-wrap gap-2">
          <select className="field-input h-[30px] w-auto text-xs" aria-label="Default model" value={d.model ?? ""} onChange={(e) => onChange((x) => ({ ...x, model: e.target.value || null, effort: null }))}>
            <option value="">Harness default{defaults?.model ? ` (${defaults.model})` : ""}</option>
            {d.model && !defaults?.models.some((m) => m.id === d.model) && <option value={d.model}>{d.model}</option>}
            {(defaults?.models ?? []).map((m) => <option key={m.id} value={m.id}>{m.display_name || m.id}</option>)}
          </select>
          {efforts.length > 0 && <select className="field-input h-[30px] w-auto text-xs" aria-label="Default effort" value={d.effort ?? ""} onChange={(e) => onChange((x) => ({ ...x, effort: e.target.value || null }))}>
            <option value="">Default effort</option>
            {efforts.map((e) => <option key={e} value={e}>{e}</option>)}
          </select>}
        </span>
      </div>
      <div className={row}>
        <b className="pt-1 font-[650]">Environment</b>
        <span className="grid gap-1.5">
          {fileEnvironment ? <span className="text-xs text-ink-3">Edit environment variables in config.toml on the host. Existing values are kept when saving here.</span> : <>
          {d.env.map(([k, v], j) => (
            <span key={j} className="grid grid-cols-[minmax(0,1fr)_26px] @min-[480px]:grid-cols-[180px_minmax(0,1fr)_26px] gap-1.5">
              <input className={`field-input col-span-full @min-[480px]:col-span-1 h-[28px] min-w-0 font-mono text-xs ${k.trim() ? "" : "border-attn-line!"}`} aria-label="Variable name" placeholder="NAME" value={k} onChange={(e) => onChange((x) => ({ ...x, env: x.env.map((p, n) => (n === j ? [e.target.value.toUpperCase(), p[1]] : p)) }))} />
              <input className="field-input h-[28px] min-w-0 font-mono text-xs" aria-label={`Value of ${k}`} placeholder="value (a leading ~/ is expanded)" value={v} onChange={(e) => onChange((x) => ({ ...x, env: x.env.map((p, n) => (n === j ? [p[0], e.target.value] : p)) }))} />
              <button className="iconbtn" title="Remove variable" onClick={() => onChange((x) => ({ ...x, env: x.env.filter((_, n) => n !== j) }))}><Icon name="x" size={13} /></button>
            </span>
          ))}
          <button className="btn btn-sm w-fit" onClick={() => onChange((x) => ({ ...x, env: [...x.env, ["", ""]] }))}><Icon name="plus" size={12} />Add variable</button>
          </>}
          <span className="text-xs text-ink-3">Added to the harness process, its terminal TUI and title requests. Other variables come from your login shell.</span>
        </span>
      </div>
    </div>
  );
}
