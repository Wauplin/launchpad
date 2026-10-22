// Inference provider of a router model (Hugging Face `org/model:provider`), with reported prices.
// The provider is a suffix of the model ID, so launch specs and profiles store only the model.

import type { ModelInfo } from "@/gen/ModelInfo";
import type { ModelProvider } from "@/gen/ModelProvider";
import { Icon } from "./Icon";
import { MenuItem, Popover } from "./Popover";

/** Routing policies of the Hugging Face router; no suffix is the same as `:fastest`. */
const POLICIES: { id: string; label: string; detail: string }[] = [
  { id: "fastest", label: "Fastest", detail: "The provider with the highest throughput" },
  { id: "cheapest", label: "Cheapest", detail: "The provider with the lowest price per output token" },
  { id: "preferred", label: "Preferred", detail: "Your order in Hugging Face settings → Inference Providers" },
];

/** "org/model:provider" → base and provider ("" for none). */
export function splitModel(id: string): { base: string; provider: string } {
  const colon = id.lastIndexOf(":");
  if (colon <= id.lastIndexOf("/")) return { base: id, provider: "" };
  return { base: id.slice(0, colon), provider: id.slice(colon + 1) };
}

const joinModel = (base: string, provider: string) => (provider ? `${base}:${provider}` : base);

/** The model picked in a list, keeping the current provider choice when the new model offers it. */
export function pickModel(models: ModelInfo[] | undefined, current: string | null, picked: string): string {
  if (splitModel(picked).provider) return picked;
  const provider = current ? splitModel(current).provider : "";
  const keep = POLICIES.some((p) => p.id === provider) || !!models?.find((m) => m.id === picked)?.providers.some((p) => p.name === provider);
  return keep ? joinModel(picked, provider) : picked;
}

export const findModel = (models: ModelInfo[] | undefined, id: string | null) => (id ? models?.find((m) => m.id === splitModel(id).base) : undefined);

const price = (n: number) => (n === 0 ? "free" : `$${n < 0.1 ? n.toFixed(3).replace(/0$/, "") : n.toFixed(2)}`);

function range(values: (number | null)[]): string | null {
  const known = values.filter((v): v is number => v !== null);
  if (!known.length) return null;
  const lo = Math.min(...known), hi = Math.max(...known);
  return lo === hi ? price(lo) : `${price(lo)}–${price(hi)}`;
}

/** The provider a policy selects, from the router's reported numbers; null when it cannot be told. */
function resolved(providers: ModelProvider[], provider: string): ModelProvider | null {
  if (provider === "fastest" || !provider) return [...providers].filter((p) => p.throughput !== null).sort((a, b) => b.throughput! - a.throughput!)[0] ?? null;
  if (provider === "cheapest") return [...providers].filter((p) => p.output_usd !== null).sort((a, b) => a.output_usd! - b.output_usd! || (a.input_usd ?? 0) - (b.input_usd ?? 0))[0] ?? null;
  return providers.find((p) => p.name === provider) ?? null;
}

const speed = (p: ModelProvider) => [p.throughput !== null && `${Math.round(p.throughput)} tok/s`, p.context_length !== null && `${Math.round(p.context_length / 1000)}K context`].filter(Boolean).join(" · ");
const shortPrices = (p: ModelProvider) => (p.input_usd === null && p.output_usd === null ? "price not reported" : `${price(p.input_usd ?? 0)} in · ${price(p.output_usd ?? 0)} out`);

/** Provider actually priced, and what the numbers describe. */
function priced(model: ModelInfo, provider: string) {
  const hit = resolved(model.providers, provider);
  const list = hit ? [hit] : model.providers;
  const note = hit ? `${provider === hit.name ? hit.name : `currently ${hit.name}`}${speed(hit) ? ` · ${speed(hit)}` : ""}` : `range over ${model.providers.length} provider${model.providers.length === 1 ? "" : "s"}`;
  return { list, note };
}

const costSource = "Prices are reported by Hugging Face; Claude Code estimates session costs with its own prices.";
/** The tooltip of the compact chip: the prices it shows, then where they come from. */
const chipNote = (prices: string, note: string) => `${prices} USD per 1M tokens · ${note}. ${costSource}`;

/**
 * Prices of the chosen provider (or the range of all providers) in one compact chip, for the
 * composer where the option row stays on a single line. The full note is in the tooltip.
 */
export function ProviderPriceChip({ model, provider }: { model: ModelInfo; provider: string }) {
  const { list, note } = priced(model, provider);
  const input = range(list.map((x) => x.input_usd));
  const cached = range(list.map((x) => x.cached_input_usd));
  const output = range(list.map((x) => x.output_usd));
  const shown: [string, string][] = [
    ...(input ? [["in", input] as [string, string]] : []),
    ...(cached ? [["cached", cached] as [string, string]] : []),
    ...(output ? [["out", output] as [string, string]] : []),
  ];
  const full = [input && `Input ${input}`, cached && `Cached input ${cached}`, output && `Output ${output}`].filter(Boolean).join(" · ") || "Price not reported";
  return (
    <span title={chipNote(full, note)} aria-label={`Price per 1M tokens: ${full}`}
      className="inline-flex h-6 max-w-full items-center gap-[5px] truncate rounded-md border border-line bg-paper px-2 text-xs text-ink-3">
      {shown.length ? shown.map(([label, value], i) => (
        <span key={label} className="inline-flex items-baseline gap-1">
          {i > 0 && <span>·</span>}
          <span>{label}</span>
          <b className="font-mono font-semibold tabular-nums text-ink">{value}</b>
        </span>
      )) : <span>price not reported</span>}
      {!!shown.length && <span>/1M</span>}
    </span>
  );
}

/** Prices of the chosen provider (or the range of all providers) per million tokens. */
export function ProviderCost({ model, provider }: { model: ModelInfo; provider: string }) {
  const { list, note } = priced(model, provider);
  const cells: [string, string | null][] = [
    ["Input", range(list.map((x) => x.input_usd))],
    ["Cached input", range(list.map((x) => x.cached_input_usd))],
    ["Output", range(list.map((x) => x.output_usd))],
  ];
  return (
    <span className="grid gap-1">
      <span className="flex flex-wrap gap-1.5" aria-label="Price per million tokens">
        {cells.map(([label, value]) => (
          <span key={label} className="inline-flex items-baseline gap-1.5 rounded-md border border-line bg-paper px-2 py-0.5 text-[11.5px]">
            <span className="text-ink-3">{label}</span>
            <b className={`font-mono font-semibold tabular-nums ${value ? "text-ink" : "font-normal text-ink-3"}`}>{value ?? "not reported"}</b>
          </span>
        ))}
      </span>
      <span className="text-[11.5px] text-ink-3">USD per 1M tokens · {note}. {costSource}</span>
    </span>
  );
}

/** Provider dropdown and prices for a router model; nothing for models without provider details. */
export function ProviderPicker({ models, model, onChange, up, compact }: { models: ModelInfo[] | undefined; model: string | null; onChange: (id: string) => void; up?: boolean; compact?: boolean }) {
  const info = findModel(models, model);
  if (!model || !info?.providers.length) return null;
  const { base, provider } = splitModel(model);
  const policy = POLICIES.find((p) => p.id === provider);
  const label = !provider ? "Fastest (default)" : (policy?.label ?? provider);
  const unknown = !!provider && !policy && !info.providers.some((p) => p.name === provider);
  return (
    <span className={compact ? "flex flex-wrap items-center gap-1.5" : "grid gap-1.5"}>
      <Popover width={300} up={up} trigger={({ toggle }) => (
        <button type="button" className={`btn btn-sm w-fit max-w-full ${unknown ? "border-attn-line!" : ""}`} aria-label="Inference provider" onClick={toggle}>
          <span className="truncate">{label}</span>
          {(policy || !provider) && resolved(info.providers, provider) && <span className="text-ink-3">· {resolved(info.providers, provider)!.name}</span>}
          <Icon name="chev" size={11} />
        </button>
      )}>
        {(close) => (
          <>
            <MenuItem label="Fastest (default)" detail="No suffix: the router picks the provider with the highest throughput" active={!provider} onClick={() => (onChange(base), close())} />
            {POLICIES.filter((p) => p.id !== "fastest" || provider === "fastest").map((p) => <MenuItem key={p.id} label={p.label} detail={`${p.detail} · :${p.id}`} active={provider === p.id} onClick={() => (onChange(joinModel(base, p.id)), close())} />)}
            <div className="px-2 pt-2 pb-0.5 text-[11px] font-semibold text-ink-3 uppercase">Providers · USD per 1M tokens</div>
            {info.providers.map((p) => (
              <MenuItem key={p.name} label={p.name} detail={[shortPrices(p), p.cached_input_usd !== null && `${price(p.cached_input_usd)} cached`, speed(p)].filter(Boolean).join(" · ")} active={provider === p.name} onClick={() => (onChange(joinModel(base, p.name)), close())} />
            ))}
          </>
        )}
      </Popover>
      {unknown ? <span className="text-[11.5px] text-attn">{provider} does not currently serve this model with tool use.</span>
        : compact ? <ProviderPriceChip model={info} provider={provider} /> : <ProviderCost model={info} provider={provider} />}
    </span>
  );
}
