// Settings (sketch S6): General and Presets, saved to config.toml, which can also be edited by hand. Workspaces have their own page.

import { useQueryClient } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { useEffect, useState } from "react";
import type { Config } from "@/gen/Config";
import type { Preset } from "@/gen/Preset";
import { Icon } from "@/components/Icon";
import { toast, toastError } from "@/components/Toast";
import { api } from "@/lib/api";
import { DEFAULT_ZOOM, ZOOM_LEVELS, useZoom } from "@/lib/zoom";
import { useConfig } from "@/lib/queries";
import { CloseAllTerminals } from "@/features/terminals/TerminalDock";

export function Card({ title, end, children, id }: { title: string; end?: React.ReactNode; children: React.ReactNode; id?: string }) {
  return (
    <section id={id} className="overflow-hidden rounded-[10px] border border-line">
      <div className="flex flex-wrap items-center gap-2 border-b border-line bg-chrome px-3.5 py-[11px] text-[12.5px] text-ink-2">
        <Icon name="sliders" size={14} />
        Settings <span className="text-ink-3">/</span> <b className="font-[650] text-ink">{title}</b>
        {end && <span className="ml-auto flex max-w-full flex-wrap justify-end gap-1.5">{end}</span>}
      </div>
      <div className="grid gap-2.5 p-3.5">{children}</div>
    </section>
  );
}

/** The layout of every Settings page, with visible buttons to switch between them. */
export function SettingsPage({ children }: { children: React.ReactNode }) {
  return (
    <div className="h-full overflow-y-auto">
      <div className="mx-auto grid max-w-[860px] grid-cols-[minmax(0,1fr)] gap-5 p-6">
        <nav className="flex flex-wrap gap-1.5" aria-label="Settings pages">
          <Link to="/settings" className="btn" activeProps={{ className: "bg-select!" }} activeOptions={{ exact: true }}>General</Link>
          <Link to="/settings/workspaces" className="btn" activeProps={{ className: "bg-select!" }}>Workspaces</Link>
          <Link to="/settings/harnesses" className="btn" activeProps={{ className: "bg-select!" }}>Harnesses</Link>
          <Link to="/settings/devices" className="btn" activeProps={{ className: "bg-select!" }}>Devices</Link>
          <Link to="/settings/worktrees" className="btn" activeProps={{ className: "bg-select!" }}>Worktrees</Link>
        </nav>
        {children}
      </div>
    </div>
  );
}

/** Sticky at the bottom of a Settings page while it has unsaved changes. */
export function SaveBar({ dirty, onSave, onRevert, invalid }: { dirty: boolean; onSave: () => void; onRevert: () => void; invalid?: string | null }) {
  if (!dirty) return null;
  return (
    <div className="sticky bottom-3 z-10 flex items-center gap-2 rounded-[10px] border border-attn-line bg-attn-soft px-3.5 py-2 text-[12.5px] shadow-[0_8px_20px_-10px_rgba(23,32,43,.35)]">
      <b className="text-attn">Unsaved changes</b>
      {invalid && <span className="text-gh-closed">{invalid}</span>}
      <span className="ml-auto flex gap-1.5">
        <button className="btn btn-sm" onClick={onRevert}>Revert</button>
        <button className="btn btn-sm btn-primary" disabled={!!invalid} onClick={onSave}>Save</button>
      </span>
    </div>
  );
}

function Row({ title, hint, children }: { title: string; hint: string; children: React.ReactNode }) {
  return (
    <div className="grid grid-cols-[170px_1fr] items-start gap-3.5 border-line-2 not-first:border-t not-first:pt-3">
      <div className="grid gap-0.5 text-[12.5px]">
        <b className="font-[650]">{title}</b>
        <span className="text-[11.5px] text-ink-3">{hint}</span>
      </div>
      <div>{children}</div>
    </div>
  );
}

export function SettingsView() {
  const q = useConfig();
  const qc = useQueryClient();
  const [draft, setDraft] = useState<Config | null>(null);
  const { zoom, setZoom } = useZoom();
  useEffect(() => {
    if (q.data && !draft) setDraft(q.data.config);
  }, [q.data, draft]);
  useEffect(() => {
    if (location.hash === "#presets") document.getElementById("presets")?.scrollIntoView();
  }, [draft]);
  if (!draft || !q.data) return <SettingsPage><div className="text-ink-3">Loading…</div></SettingsPage>;
  const saved = q.data.config;
  // Variants are edited on their own page: always keep the saved ones.
  const merged = { ...draft, variants: saved.variants };
  const dirty = JSON.stringify(merged) !== JSON.stringify(saved);
  const save = async () => {
    try {
      const res = await api.put<{ config: Config }>("/api/config", merged);
      qc.setQueryData(["config"], { config: res.config, error: null });
      setDraft(res.config);
      toast("Settings saved");
    } catch (e) {
      toastError(e);
    }
  };
  const set = (f: (c: Config) => Config) => setDraft((d) => (d ? f(d) : d));
  const setPreset = (i: number, p: Partial<Preset>) => set((c) => ({ ...c, presets: c.presets.map((x, j) => (j === i ? { ...x, ...p } : x)) }));
  const move = (i: number, d: -1 | 1) =>
    set((c) => {
      const presets = [...c.presets];
      const j = i + d;
      if (j < 0 || j >= presets.length) return c;
      [presets[i], presets[j]] = [presets[j], presets[i]];
      return { ...c, presets };
    });

  return (
    <SettingsPage>
      {q.data.error && <div className="rounded-lg bg-attn-soft px-3 py-2 text-xs text-attn">config.toml has an error, the previous settings are in use: {q.data.error}</div>}
      <Card title="General">
        <Row title="Zoom" hint="Make the whole interface larger on this device. Applies at once; other devices keep their own zoom. The middle position is the default.">
          <label className="grid gap-2">
            <span className="font-semibold">{ZOOM_LEVELS[zoom]}%{zoom === DEFAULT_ZOOM ? " (default)" : ""}</span>
            <input aria-label="Interface zoom" type="range" min={0} max={4} step={1} value={zoom} onChange={(e) => setZoom(Number(e.target.value))} className="w-full accent-teal" />
            <span className="flex justify-between text-xs text-ink-2">{ZOOM_LEVELS.map((z) => <span key={z}>{z}%</span>)}</span>
          </label>
        </Row>
        <Row title="Projects root" hint="Where projects are found, and where temporary folders and worktrees are created.">
          <input className="field-input font-mono text-xs" value={draft.projects_root} onChange={(e) => set((c) => ({ ...c, projects_root: e.target.value }))} />
        </Row>
        <Row title="Published pre-draft worktrees" hint="Keep the local worktree by default. Cleanup only removes clean worktrees after a successful publication.">
          <label className="flex items-center gap-2"><input type="checkbox" checked={draft.remove_published_worktree} onChange={(e) => set((c) => ({ ...c, remove_published_worktree: e.target.checked }))} />Remove after publishing</label>
        </Row>
        <Row title="Terminals" hint="Integrated terminals run in the daemon. Restarting it closes them.">
          <CloseAllTerminals />
        </Row>
      </Card>

      <Card
        id="presets"
        title="Presets"
        end={
          <>
            <button
              className="btn btn-sm"
              onClick={() => set((c) => ({ ...c, presets: [...c.presets, { id: `preset-${Date.now().toString(36)}`, label: "new preset", text: "" }] }))}
            >
              <Icon name="plus" size={12} />
              New preset
            </button>
          </>
        }
      >
        {draft.presets.map((p, i) => (
          <div key={p.id} className="grid grid-cols-[22px_150px_1fr_26px] items-start gap-2.5 rounded-lg border border-line p-2.5">
            <span className="grid">
              <button className="text-ink-3 hover:text-ink disabled:opacity-30" disabled={i === 0} title="Move up" onClick={() => move(i, -1)}>
                ▲
              </button>
              <button className="text-ink-3 hover:text-ink disabled:opacity-30" disabled={i === draft.presets.length - 1} title="Move down" onClick={() => move(i, 1)}>
                ▼
              </button>
            </span>
            <input className="field-input h-[30px] text-[12.5px] font-semibold" value={p.label} onChange={(e) => setPreset(i, { label: e.target.value })} />
            <textarea
              rows={2}
              className="field-input h-auto min-h-[30px] resize-y py-1.5 text-[12.5px]"
              value={p.text}
              placeholder="The sentence added to the prompt"
              onChange={(e) => setPreset(i, { text: e.target.value })}
            />
            <button className="iconbtn" title="Delete" onClick={() => set((c) => ({ ...c, presets: c.presets.filter((_, j) => j !== i) }))}>
              <Icon name="trash" size={14} />
            </button>
          </div>
        ))}
        <div className="text-[11.5px] text-ink-3">Every saved preset is available in the composer and adds its text to the prompt. The order here is the order used in prompts. Saved to config.toml, which you can also edit by hand.</div>
      </Card>
      <SaveBar dirty={dirty} onSave={() => void save()} onRevert={() => setDraft(saved)} />
    </SettingsPage>
  );
}
