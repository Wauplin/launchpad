// The chart block (lazy-loaded): bar, line, area and pie charts drawn as SVG from a block's rows or a CSV of the
// folio, no library and no network. Series colors are the theme's categorical slots (`--folio-series-N` in
// styles.css, light and dark), in fixed order; text uses the ink tokens; each mark has a native tooltip; a legend
// shows for two series or more. Bad data throws, and the block reports it as F702.

import { memo, useState } from "react";

export type ChartData = { kind: string; labels: string[]; series: { name: string; values: number[] }[]; unit: string };

const W = 640, H = 260, PAD = { l: 44, r: 12, t: 12, b: 40 };
const color = (i: number) => `var(--folio-series-${(i % 8) + 1})`;
const fmt = (n: number) => (Math.abs(n) >= 1000 ? n.toLocaleString(undefined, { maximumFractionDigits: 0 }) : String(Math.round(n * 100) / 100));

/** Pure: a chart's data from a header and rows of cells (x: the label column, y: the value columns). */
export function chartData(kind: string, header: string[], rows: string[][], x: string | null, y: string[], unit: string): ChartData {
  const xi = x ? header.indexOf(x) : 0;
  if (xi < 0) throw new Error(`no column ${x}`);
  const ys = (y.length ? y : header.filter((_, i) => i !== xi)).map((name) => { const i = header.indexOf(name); if (i < 0) throw new Error(`no column ${name}`); return { name, i }; });
  if (!ys.length) throw new Error("no value column");
  if (rows.length > 500) throw new Error("more than 500 rows: summarize the data first");
  const series = ys.map(({ name, i }) => ({ name, values: rows.map((r) => { const v = Number(String(r[i] ?? "").replace(/[\s,%]/g, "")); if (!Number.isFinite(v)) throw new Error(`${name}: “${r[i] ?? ""}” is not a number`); return v; }) }));
  return { kind: ["bar", "line", "area", "pie"].includes(kind) ? kind : "bar", labels: rows.map((r) => String(r[xi] ?? "")), series, unit };
}

function Legend({ d }: { d: ChartData }) {
  if (d.series.length < 2 && d.kind !== "pie") return null;
  const names = d.kind === "pie" ? d.labels : d.series.map((s) => s.name);
  return <div className="folio-chart-legend">{names.slice(0, 8).map((n, i) => <span key={n + i}><i style={{ background: color(i) }} />{n}</span>)}</div>;
}

function Pie({ d }: { d: ChartData }) {
  const values = d.series[0].values.map((v) => Math.max(0, v));
  const total = values.reduce((a, b) => a + b, 0) || 1;
  let angle = -Math.PI / 2;
  const r = 100, cx = W / 2, cy = H / 2;
  return <svg viewBox={`0 0 ${W} ${H}`} role="img" aria-label="Pie chart">{values.map((v, i) => {
    const a0 = angle, a1 = angle + (v / total) * Math.PI * 2;
    angle = a1;
    const large = a1 - a0 > Math.PI ? 1 : 0;
    const p = (a: number) => `${cx + r * Math.cos(a)} ${cy + r * Math.sin(a)}`;
    const path = values.length === 1 ? `M ${cx - r} ${cy} a ${r} ${r} 0 1 0 ${2 * r} 0 a ${r} ${r} 0 1 0 ${-2 * r} 0` : `M ${cx} ${cy} L ${p(a0)} A ${r} ${r} 0 ${large} 1 ${p(a1)} Z`;
    return <path key={i} d={path} fill={color(i)} stroke="var(--color-paper)" strokeWidth={2}><title>{`${d.labels[i]}: ${fmt(v)}${d.unit ? ` ${d.unit}` : ""} (${Math.round((v / total) * 100)} %)`}</title></path>;
  })}</svg>;
}

function Axes({ d }: { d: ChartData }) {
  const all = d.series.flatMap((s) => s.values);
  // Round ticks: a step of 1, 2 or 5 times a power of ten, four steps or so.
  const raw = (Math.max(0, ...all) - Math.min(0, ...all)) / 4 || 1;
  const pow = 10 ** Math.floor(Math.log10(raw));
  const step = [1, 2, 5, 10].map((m) => m * pow).find((s) => s >= raw) ?? raw;
  const max = Math.ceil(Math.max(0, ...all) / step) * step, min = Math.floor(Math.min(0, ...all) / step) * step;
  const span = max - min || 1;
  const y = (v: number) => PAD.t + (1 - (v - min) / span) * (H - PAD.t - PAD.b);
  const n = d.labels.length;
  const band = (W - PAD.l - PAD.r) / Math.max(1, n);
  const ticks = Array.from({ length: Math.round(span / step) + 1 }, (_, i) => min + i * step);
  const [hover, setHover] = useState<number | null>(null);
  const every = Math.max(1, Math.ceil(n / 10));
  return <svg viewBox={`0 0 ${W} ${H}`} role="img" aria-label={`${d.kind} chart`} onMouseLeave={() => setHover(null)}>
    {ticks.map((t, i) => <g key={i}><line x1={PAD.l} x2={W - PAD.r} y1={y(t)} y2={y(t)} stroke={t === 0 ? "var(--color-line)" : "var(--color-line-2)"} strokeWidth={1} /><text x={PAD.l - 6} y={y(t) + 4} textAnchor="end" className="folio-chart-tick">{fmt(t)}</text></g>)}
    {d.labels.map((l, i) => i % every === 0 && <text key={i} x={PAD.l + band * (i + 0.5)} y={H - PAD.b + 16} textAnchor="middle" className="folio-chart-tick">{l.length > 12 ? `${l.slice(0, 11)}…` : l}</text>)}
    {d.kind === "bar" && d.series.map((s, si) => {
      const w = Math.max(2, Math.min(28, (band * 0.6) / d.series.length - 2));
      const group = d.series.length * (w + 2) - 2;
      return s.values.map((v, i) => <rect key={`${si}-${i}`} x={PAD.l + band * (i + 0.5) - group / 2 + si * (w + 2)} y={Math.min(y(v), y(0))} width={w} height={Math.max(1, Math.abs(y(v) - y(0)))} rx={Math.min(4, w / 2)} fill={color(si)}><title>{`${d.labels[i]} · ${s.name}: ${fmt(v)}${d.unit ? ` ${d.unit}` : ""}`}</title></rect>);
    })}
    {(d.kind === "line" || d.kind === "area") && d.series.map((s, si) => {
      const pts = s.values.map((v, i) => `${PAD.l + band * (i + 0.5)},${y(v)}`);
      return <g key={si}>
        {d.kind === "area" && <polygon points={`${PAD.l + band * 0.5},${y(0)} ${pts.join(" ")} ${PAD.l + band * (n - 0.5)},${y(0)}`} fill={color(si)} opacity={0.18} />}
        <polyline points={pts.join(" ")} fill="none" stroke={color(si)} strokeWidth={2} strokeLinejoin="round" />
        {s.values.map((v, i) => <circle key={i} cx={PAD.l + band * (i + 0.5)} cy={y(v)} r={hover === i ? 4 : 2.5} fill={color(si)} stroke="var(--color-paper)" strokeWidth={1.5}><title>{`${d.labels[i]} · ${s.name}: ${fmt(v)}${d.unit ? ` ${d.unit}` : ""}`}</title></circle>)}
      </g>;
    })}
    {(d.kind === "line" || d.kind === "area") && d.labels.map((_, i) => <rect key={i} x={PAD.l + band * i} y={PAD.t} width={band} height={H - PAD.t - PAD.b} fill="transparent" onMouseEnter={() => setHover(i)}><title>{`${d.labels[i]}: ${d.series.map((s) => `${s.name} ${fmt(s.values[i])}`).join(", ")}${d.unit ? ` ${d.unit}` : ""}`}</title></rect>)}
    {hover != null && <line x1={PAD.l + band * (hover + 0.5)} x2={PAD.l + band * (hover + 0.5)} y1={PAD.t} y2={H - PAD.b} stroke="var(--color-line)" strokeWidth={1} pointerEvents="none" />}
  </svg>;
}

/** The block: its data (rows or a CSV), or the error that keeps it from drawing (reported as F702). */
const ChartBlock = memo(function ChartBlock({ kind, header, rows, x, y, unit, title, onError }: { kind: string; header: string[]; rows: string[][]; x: string | null; y: string[]; unit: string; title: string; onError: (message: string) => void }) {
  let data: ChartData;
  try { data = chartData(kind, header, rows, x, y, unit); } catch (e) {
    const message = (e as Error).message;
    queueMicrotask(() => onError(message));
    return <div className="folio-problem" role="note"><div className="folio-problem-head">This chart cannot draw</div><div className="folio-problem-line">F702 · {message}</div></div>;
  }
  return <ChartView data={data} title={title} />;
});
export default ChartBlock;

function ChartView({ data, title }: { data: ChartData; title: string }) {
  return <figure className="folio-chart">
    {data.kind === "pie" ? <Pie d={data} /> : <Axes d={data} />}
    <Legend d={data} />
    {title && <figcaption className="folio-caption">{title}{data.unit ? ` (${data.unit})` : ""}</figcaption>}
  </figure>;
}
