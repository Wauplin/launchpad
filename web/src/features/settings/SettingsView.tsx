import { useRemote } from "@/lib/remote";
import { copyText } from "@/lib/clipboard";
// Settings (sketch S6): General and Presets. Saved to config.toml, which can also be edited by hand.

import { useQueryClient } from "@tanstack/react-query";
import { useEffect, useState } from "react";
import type { Config } from "@/gen/Config";
import type { Preset } from "@/gen/Preset";
import { Icon } from "@/components/Icon";
import { toast, toastError } from "@/components/Toast";
import { api } from "@/lib/api";
import { useAppUpdate } from "@/lib/app-update";
import { ZOOM_LEVELS } from "@/lib/zoom";
import { useConfig, useTerminals } from "@/lib/queries";

function Card({ title, end, children, id }: { title: string; end?: React.ReactNode; children: React.ReactNode; id?: string }) {
  return (
    <section id={id} className="overflow-hidden rounded-[10px] border border-line">
      <div className="flex items-center gap-2 border-b border-line bg-chrome px-3.5 py-[11px] text-[12.5px] text-ink-2">
        <Icon name="sliders" size={14} />
        Settings <span className="text-ink-3">/</span> <b className="font-[650] text-ink">{title}</b>
        {end && <span className="ml-auto flex gap-1.5">{end}</span>}
      </div>
      <div className="grid gap-2.5 p-3.5">{children}</div>
    </section>
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

function UpdateControl() {
  const { data, start, busy } = useAppUpdate();
  return (
    <div className="grid gap-2 text-[12.5px]">
      <button className="btn btn-primary w-fit" disabled={!data || busy || data.state === "unavailable"} onClick={() => start.mutate()}>
        {start.isPending ? "Starting…" : busy ? "Updating…" : "Check and update Launchpad"}
      </button>
      <span className="text-ink-2">Runs UI type checks, Rust tests and release builds in the background, then waits until all agents stop before installing and relaunching Launchpad.</span>
      {data && data.state !== "idle" && <span className={data.state === "failed" ? "text-attn" : "text-ink-2"}>{data.stage}{data.stage && data.message ? ": " : ""}{data.message}</span>}
      {data?.log && data.state === "failed" && <details><summary className="cursor-pointer">Show update log</summary><pre className="mt-2 max-h-64 overflow-auto whitespace-pre-wrap rounded-md bg-chrome p-2 font-mono text-[11px]">{data.log}</pre></details>}
    </div>
  );
}

export function SettingsView() {
  const remote = useRemote();
  const q = useConfig();
  const terminals = useTerminals().data ?? [];
  const qc = useQueryClient();
  const [draft, setDraft] = useState<Config | null>(null);
  useEffect(() => {
    if (q.data && !draft) setDraft(q.data.config);
  }, [q.data, draft]);
  useEffect(() => {
    if (location.hash === "#presets") document.getElementById("presets")?.scrollIntoView();
  }, [draft]);
  if (!draft || !q.data) return <div className="p-6 text-ink-3">Loading…</div>;
  const saved = q.data.config;
  const dirty = JSON.stringify(draft) !== JSON.stringify(saved);
  const save = async () => {
    try {
      const res = await api.put<{ config: Config }>("/api/config", draft);
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
  const actions = (
    <>
      <button className="btn btn-sm btn-ghost" disabled={!dirty} onClick={() => setDraft(saved)}>
        Revert
      </button>
      <button className="btn btn-sm btn-primary" disabled={!dirty} onClick={() => void save()}>
        Save
      </button>
    </>
  );

  return (
    <div className="h-full overflow-y-auto">
      <div className="mx-auto grid max-w-[860px] grid-cols-[minmax(0,1fr)] gap-5 p-6">
        {q.data.error && <div className="rounded-lg bg-attn-soft px-3 py-2 text-xs text-attn">config.toml has an error, the previous settings are in use: {q.data.error}</div>}
        <Card title="General" end={actions}>
          <Row title="Remote browser" hint="Saved in this browser only. Use an SSH tunnel to the daemon's localhost port.">
            <label className="flex items-center gap-2 text-xs"><input type="checkbox" checked={remote.enabled} onChange={(e) => remote.set(e.target.checked)} />Copy editor and external-terminal commands for the server</label>
            <p className="mt-2 text-xs text-ink-3">The embedded terminal runs on the server. Forward the same port (7317 for production, 7318 for dev), then run launchpad url --remote on the server and open its address here.</p>
            <button className="btn btn-sm mt-2" onClick={() => void copyText("ssh -N -L 127.0.0.1:7317:127.0.0.1:7317 user@server").then(() => toast("SSH tunnel template copied")).catch(toastError)}>Copy SSH tunnel template</button>
          </Row>
          <Row title="Zoom" hint="Make the whole interface larger. The middle position is the default.">
            <label className="grid gap-2">
              <span className="font-semibold">{ZOOM_LEVELS[draft.zoom]}%{draft.zoom === 2 ? " (default)" : ""}</span>
              <input aria-label="Interface zoom" type="range" min={0} max={4} step={1} value={draft.zoom} onChange={(e) => set((c) => ({ ...c, zoom: Number(e.target.value) }))} className="w-full accent-teal" />
              <span className="flex justify-between text-xs text-ink-2">{ZOOM_LEVELS.map((z) => <span key={z}>{z}%</span>)}</span>
            </label>
          </Row>
          <Row title="Projects root" hint="Where projects are found, and where temporary folders and worktrees are created.">
            <input className="field-input font-mono text-xs" value={draft.projects_root} onChange={(e) => set((c) => ({ ...c, projects_root: e.target.value }))} />
          </Row>
          <Row title="Published pre-draft worktrees" hint="Keep the local worktree by default. Cleanup only removes clean worktrees after a successful publication.">
            <label className="flex items-center gap-2"><input type="checkbox" checked={draft.remove_published_worktree} onChange={(e) => set((c) => ({ ...c, remove_published_worktree: e.target.checked }))} />Remove after publishing</label>
          </Row>
          <Row title="Terminal" hint='Used by "Open in terminal".'>
            <div className="grid gap-1.5">
              {terminals.map((t) => (
                <button
                  key={t.id}
                  onClick={() => set((c) => ({ ...c, terminal: { ...c.terminal, app: t.id } }))}
                  className={`grid grid-cols-[14px_1fr] items-start gap-[9px] rounded-[7px] border px-2.5 py-2 text-left ${draft.terminal.app === t.id ? "border-teal bg-[#F4FAF8]" : "border-line bg-paper"}`}
                >
                  <span className={`mt-0.5 size-3.5 rounded-full ${draft.terminal.app === t.id ? "border-4 border-teal" : "border-[1.5px] border-ink-3"}`} />
                  <span className="grid min-w-0 gap-0.5 text-[12.5px]">
                    <b className="font-[650]">
                      {t.name} {t.found ? <span className="ml-1 text-[10.5px] text-gh-open">found</span> : <span className="ml-1 text-[10.5px] text-ink-3">not installed</span>}
                    </b>
                    <span className="font-mono text-[11px] break-all text-ink-2">{t.template}</span>
                  </span>
                </button>
              ))}
              <button
                onClick={() => set((c) => ({ ...c, terminal: { ...c.terminal, app: "custom" } }))}
                className={`grid grid-cols-[14px_1fr] items-start gap-[9px] rounded-[7px] border px-2.5 py-2 text-left ${draft.terminal.app === "custom" ? "border-teal bg-[#F4FAF8]" : "border-line bg-paper"}`}
              >
                <span className={`mt-0.5 size-3.5 rounded-full ${draft.terminal.app === "custom" ? "border-4 border-teal" : "border-[1.5px] border-ink-3"}`} />
                <span className="grid gap-0.5 text-[12.5px]">
                  <b className="font-[650]">Custom command</b>
                  <span className="text-[11.5px] text-ink-3">Any other terminal. Use {"{cwd}"} and {"{command}"} in the command, one argument per space.</span>
                </span>
              </button>
              {draft.terminal.app === "custom" && (
                <input
                  className="field-input font-mono text-xs"
                  value={draft.terminal.custom.join(" ")}
                  onChange={(e) => set((c) => ({ ...c, terminal: { ...c.terminal, custom: e.target.value.split(" ").filter(Boolean) } }))}
                />
              )}
            </div>
          </Row>
          <Row title="Editor" hint='Used by "Open in editor".'>
            <input className="field-input font-mono text-xs" value={draft.commands.editor} onChange={(e) => set((c) => ({ ...c, commands: { ...c.commands, editor: e.target.value } }))} />
          </Row>
          <Row title="App update" hint="Install the latest code from this checkout.">
            <UpdateControl />
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
              {actions}
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
          <div className="text-[11.5px] text-ink-3">The order here is the order used in prompts. Saved to config.toml, which you can also edit by hand.</div>
        </Card>
      </div>
    </div>
  );
}
