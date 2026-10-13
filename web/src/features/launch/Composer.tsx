// Composer drawer (sketch S2): stays on the right while you navigate, so context can be added from
// anywhere. It minimizes to a draft pill, and covers the content on narrow windows. Prompt first, then
// project, context chips and one summary line of options; "Preview prompt" shows the exact prompt.

import { abandonOperation } from "@/lib/operations";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useNavigate } from "@tanstack/react-router";
import { useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import type { AgentTraceRef } from "@/gen/AgentTraceRef";
import type { GitHubRef } from "@/gen/GitHubRef";
import type { HarnessDefaults } from "@/gen/HarnessDefaults";
import type { HarnessKind } from "@/gen/HarnessKind";
import type { HarnessVariant } from "@/gen/HarnessVariant";
import { pastedImages, readAttachments } from "@/lib/attachments";
import type { LaunchSpec } from "@/gen/LaunchSpec";
import type { Preview } from "@/gen/Preview";
import type { Project } from "@/gen/Project";
import type { Session } from "@/gen/Session";
import { HarnessMark, useHarnessName, useHarnesses, useHarnessInfo } from "@/components/HarnessMark";
import { Icon } from "@/components/Icon";
import { MenuItem, Popover } from "@/components/Popover";
import { toast, toastError } from "@/components/Toast";
import { api, ApiError } from "@/lib/api";
import { useConfig, useHarnessDefaults, useProjects, useSessions } from "@/lib/queries";
import { parseUsd, usd, useVariants } from "@/lib/variants";
import { useScope } from "@/lib/workspace";
import { zoomScale } from "@/lib/zoom";
import { rememberedRun, rememberSuccessfulSession, useComposer } from "@/stores/composer";
import { useLayout } from "@/stores/layout";
import { ProjectPicker } from "./ProjectPicker";
import { AgentTracePicker } from "./AgentTracePicker";
import { DelaySelector, conditionsError } from "./DelaySelector";
import { CronSelector, type CronDraft } from "./CronSelector";
import { itemIcon, PromptField } from "./PromptField";

function Field({ label, hint, end, children }: { label: string; hint?: string; end?: React.ReactNode; children: React.ReactNode }) {
  return (
    <div className="grid min-w-0 grid-cols-1 content-start gap-1.5">
      <div className="flex items-center gap-2 text-[11.5px] font-[650] text-ink-2">
        {label}
        {hint && <span className="font-normal text-ink-3">{hint}</span>}
        {end && <span className="ml-auto">{end}</span>}
      </div>
      {children}
    </div>
  );
}

function SelectButton({ onClick, children }: { onClick: () => void; children: React.ReactNode }) {
  return (
    <button onClick={onClick} className="flex h-8 w-full min-w-0 items-center gap-2 rounded-[7px] border border-line bg-paper px-2.5 text-left text-[12.5px] hover:border-[#c4ccd7]">
      <span className="flex min-w-0 flex-1 items-center gap-2 truncate">{children}</span>
      <Icon name="chev" size={14} className="flex-none text-ink-3" />
    </button>
  );
}

function Branch({ projectId, branch, onChange, currentLabel = "checkout" }: { projectId: string; branch: string | null; onChange: (b: string | null) => void; currentLabel?: string }) {
  const q = useQuery({
    queryKey: ["branches", projectId],
    queryFn: () => api.get<{ current: string | null; branches: string[] }>(`/api/projects/${projectId}/branches`),
    staleTime: 30_000,
  });
  return (
    <Popover
      width={260}
      trigger={({ toggle }) => (
        <button onClick={toggle} className="inline-flex h-5 items-center gap-1 rounded-[5px] border border-line bg-paper px-1.5 font-mono text-[11.5px] whitespace-nowrap">
          <Icon name="branch" size={12} />
          {branch ?? `${currentLabel}${q.data?.current ? ` (${q.data.current})` : ""}`}
          <Icon name="chev" size={12} className="text-ink-3" />
        </button>
      )}
    >
      {(close) => (
        <>
          <MenuItem label={currentLabel === "checkout" ? "Current checkout" : "Current branch"} detail={q.data?.current ?? undefined} active={branch === null} onClick={() => (onChange(null), close())} />
          {(q.data?.branches ?? []).map((b) => (
            <MenuItem key={b} icon={<Icon name="branch" size={13} />} label={<span className="font-mono text-xs">{b}</span>} active={b === branch} onClick={() => (onChange(b), close())} />
          ))}
          {q.isLoading && <div className="px-2 py-1 text-xs text-ink-3">Loading branches…</div>}
        </>
      )}
    </Popover>
  );
}

function GitHubLinkForm({ onAdd, onDone }: { onAdd: (r: GitHubRef) => void; onDone: () => void }) {
  const [url, setUrl] = useState("");
  const [busy, setBusy] = useState(false);
  const add = async () => {
    if (!url.trim()) return;
    setBusy(true);
    try {
      onAdd(await api.post<GitHubRef>("/api/gh/resolve", { url: url.trim() }));
      onDone();
    } catch (e) {
      toastError(e);
    } finally {
      setBusy(false);
    }
  };
  return (
    <div className="grid gap-1.5 p-1">
      <input
        autoFocus
        className="field-input h-7 text-xs"
        placeholder="https://github.com/owner/repo/issues/123"
        value={url}
        onChange={(e) => setUrl(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter") void add();
        }}
      />
      <span className="flex justify-end gap-1.5">
        <button className="btn btn-sm btn-ghost" onClick={onDone}>Cancel</button>
        <button className="btn btn-sm btn-primary" disabled={busy || !url.trim()} onClick={() => void add()}>{busy ? "Adding…" : "Add"}</button>
      </span>
    </div>
  );
}

/** The "+ Add context" popover: one menu for projects, GitHub links, agent traces and recent contexts. */
function AddContextMenu({ projects, recent, sessions, selectedTraces, onProject, onGitHub, onTrace }: {
  projects: Project[];
  recent: { key: string; label: string; add: () => void }[];
  sessions: Session[];
  selectedTraces: AgentTraceRef[];
  onProject: (p: Project) => void;
  onGitHub: (r: GitHubRef) => void;
  onTrace: (t: AgentTraceRef) => void;
}) {
  const [mode, setMode] = useState<"menu" | "project" | "github" | "trace">("menu");
  return (
    <Popover
      width={340}
      trigger={({ toggle }) => (
        <button className="btn btn-sm" onClick={() => { setMode("menu"); toggle(); }}>
          <Icon name="plus" size={12} />
          Add context
        </button>
      )}
    >
      {(close) => {
        const back = <MenuItem icon={<Icon name="chev" size={13} className="rotate-90" />} label="Back" onClick={() => setMode("menu")} />;
        if (mode === "project") return <>{back}{projects.map((p) => <MenuItem key={p.id} icon={<Icon name="folder" size={13} />} label={p.name} detail={p.description || undefined} onClick={() => (onProject(p), close())} />)}{!projects.length && <div className="px-2 py-1 text-xs text-ink-3">No other project.</div>}</>;
        if (mode === "github") return <>{back}<GitHubLinkForm onAdd={onGitHub} onDone={close} /></>;
        if (mode === "trace") return <>{back}<AgentTracePicker sessions={sessions} selected={selectedTraces} onPick={(t) => { onTrace(t); close(); }} /></>;
        return (
          <>
            <MenuItem icon={<Icon name="folder" size={13} />} label="Project…" detail="Another project as read-only context" onClick={() => setMode("project")} />
            <MenuItem icon={<Icon name="issue" size={13} />} label="GitHub link…" detail="An issue, PR or comment URL" onClick={() => setMode("github")} />
            <MenuItem icon={<Icon name="agents" size={13} />} label="Agent trace…" detail="The stored trace of another agent" onClick={() => setMode("trace")} />
            {!!recent.length && <div className="px-[9px] pt-2 pb-1 seclabel">Recent contexts</div>}
            {recent.slice(0, 6).map((c) => <MenuItem key={c.key} icon={<Icon name="plus" size={13} />} label={<span className="block truncate" title={c.label}>{c.label}</span>} onClick={() => (c.add(), close())} />)}
          </>
        );
      }}
    </Popover>
  );
}

function PromptPreview({ spec, title }: { spec: LaunchSpec; title: string }) {
  const debounced = useDebounced(spec, 250);
  const preview = useQuery({ queryKey: ["preview", debounced], queryFn: () => api.post<Preview>("/api/launch/preview", debounced), placeholderData: (prev) => prev });
  return (
    <div className="grid gap-1 p-1">
      <div className="flex items-center text-[11.5px] font-[650] text-ink-2">
        {title}
        <span className="ml-auto font-normal text-ink-3">{preview.isFetching ? "Updating…" : `${preview.data?.prompt.length ?? 0} characters`}</span>
      </div>
      <pre className="m-0 max-h-[50vh] overflow-y-auto rounded-md bg-chrome px-2.5 py-2 font-mono text-[11.5px] leading-normal whitespace-pre-wrap text-ink-2">
        {preview.isError ? <span className="text-red-700">{preview.error.message}</span> : preview.data?.prompt || <span className="text-ink-3">{preview.isLoading ? "Rendering…" : "The prompt is empty."}</span>}
      </pre>
    </div>
  );
}

function useDebounced<T>(value: T, ms: number): T {
  const [v, setV] = useState(value);
  useEffect(() => {
    const t = setTimeout(() => setV(value), ms);
    return () => clearTimeout(t);
  }, [value, ms]);
  return v;
}

/** Windows narrower than this layout width (1,500 px at the default 120% zoom) show the drawer over the content. */
const OVERLAY_BELOW = 1250;
function useOverlay(zoom: number) {
  const [width, setWidth] = useState(window.innerWidth);
  useEffect(() => {
    const onResize = () => setWidth(window.innerWidth);
    window.addEventListener("resize", onResize);
    return () => window.removeEventListener("resize", onResize);
  }, []);
  // The root `zoom` scales CSS pixels; `innerWidth` is not zoomed.
  return width / zoomScale(zoom) < OVERLAY_BELOW;
}

function Token({ warn, title, children }: { warn?: boolean; title?: string; children: React.ReactNode }) {
  return <span title={title} className={`inline-flex h-[22px] max-w-full items-center gap-1 truncate rounded-md px-1.5 text-[12px] ${warn ? "bg-attn-soft font-semibold text-attn shadow-[inset_0_0_0_1px_var(--color-attn-line)]" : "bg-sunk text-ink-2"}`}>{children}</span>;
}

function OptionRow({ label, end, children }: { label: string; end?: React.ReactNode; children: React.ReactNode }) {
  return (
    <div className="grid grid-cols-[76px_1fr] items-start gap-2">
      <span className="flex items-center gap-1 pt-1.5 text-[11.5px] font-[650] text-ink-2">{label}{end}</span>
      <div className="min-w-0">{children}</div>
    </div>
  );
}

export function Composer() {
  const harnessName = useHarnessName();
  const harnesses = useHarnesses();
  const { open, minimized, setMinimized, spec, update, close, discard, openAfterLaunch, setOpenAfterLaunch, conditions, setConditions } = useComposer();
  const projects = useProjects().data ?? [];
  const scope = useScope();
  const sessions = useSessions().data;
  useEffect(() => {
    // A queued launch may have started while the browser was disconnected.
    for (const session of sessions ?? []) rememberSuccessfulSession(session);
  }, [sessions]);
  const config = useConfig().data?.config;
  const overlay = useOverlay(config?.zoom ?? 2);
  const [pillSlot, setPillSlot] = useState<HTMLElement | null>(null);
  useEffect(() => setPillSlot(document.getElementById("draft-pill-slot")), []);
  const defaults = useHarnessDefaults(spec.harness).data;
  const harness = useHarnessInfo(spec.harness);
  const variants = useVariants();
  const variant = variants.find((v) => v.id === spec.variant) ?? null;
  const payg = variant?.billing === "pay_as_you_go";
  const [budgetText, setBudgetText] = useState<string | null>(null);
  // Typed text is kept only while editing; reopening shows the draft's limit.
  useEffect(() => setBudgetText(null), [open]);
  const queryClient = useQueryClient();
  const refreshModels = useMutation({
    mutationFn: (h: HarnessKind) => api.post<HarnessDefaults>(`/api/harness/${h}/defaults`),
    onSuccess: (data, h) => {
      queryClient.setQueryData(["harness-defaults", h], data);
      toast(`${harnesses.find((entry) => entry.id === h)?.name ?? h} models refreshed`);
    },
    onError: toastError,
  });
  const navigate = useNavigate();
  const showSession = useLayout((s) => s.show);
  const [delayValid, setDelayValid] = useState(true);
  const [cron, setCron] = useState<CronDraft | null>(null);
  const [cronValid, setCronValid] = useState(true);
  const [launching, setLaunching] = useState(false);
  const [unconfirmedLaunch, setUnconfirmedLaunch] = useState<LaunchSpec | null>(null);
  const [readingAttachments, setReadingAttachments] = useState(false);
  const attachmentRead = useRef(false);
  const attachmentInput = useRef<HTMLInputElement>(null);
  const addAttachments = async (input: File[] | Promise<File[]>) => {
    if (launching || attachmentRead.current) return;
    attachmentRead.current = true;
    setReadingAttachments(true);
    const draft = useComposer.getState().spec;
    try {
      const files = await input;
      if (!files.length) return;
      const added = await readAttachments(files, draft.attachments ?? []);
      if (added.some((f) => f.data_url.startsWith("data:image/")) && harness && !harness.capabilities.images) throw new Error("This harness does not support images.");
      // Do not add a completed read to a different workspace or a discarded draft.
      if (useComposer.getState().spec.attachments === draft.attachments) update((s) => ({ ...s, attachments: [...(s.attachments ?? []), ...added] }));
    } catch (e) { toastError(e); }
    finally { attachmentRead.current = false; setReadingAttachments(false); }
  };
  const [showOptions, setShowOptions] = useState(false);
  const items = (spec.attachments ?? []).length + spec.github_refs.length + (spec.files ?? []).length + spec.context_projects.length + (spec.agent_traces ?? []).length;
  // The draft pill flashes when "+ Add" buttons add context to a minimized draft.
  const [flash, setFlash] = useState(false);
  const lastItems = useRef(items);
  useEffect(() => {
    const grew = items > lastItems.current;
    lastItems.current = items;
    if (!grew || !minimized) return;
    setFlash(true);
    const t = setTimeout(() => setFlash(false), 900);
    return () => clearTimeout(t);
  }, [items, minimized]);

  const targetId = spec.target.kind === "project" || spec.target.kind === "predraft" ? spec.target.project_id : null;
  const targetBranches = useQuery({
    queryKey: ["branches", targetId],
    queryFn: () => api.get<{ current: string | null; branches: string[] }>(`/api/projects/${targetId}/branches`),
    enabled: open && !!targetId,
    staleTime: 30_000,
    retry: false,
  });
  // Worktrees need a Git repository with a commit; other folders run in place.
  const gitTarget = !!targetId && !targetBranches.isError;
  const localPrs = useQuery({ queryKey: ["predrafts"], queryFn: () => api.get<import("@/gen/Predraft").Predraft[]>("/api/predrafts"), enabled: open });
  const existing = spec.target.kind === "existing_predraft" ? localPrs.data?.find((p) => p.id === (spec.target as { predraft_id: string }).predraft_id) : undefined;
  const source = spec.target.kind === "session_folder" ? sessions?.find((s) => s.id === (spec.target as { session_id: string }).session_id) : undefined;
  const recentSessions = (sessions ?? []).filter(scope.hasSession);
  if (!open) return null;

  const refreshingModels = refreshModels.isPending;
  const refreshModelList = () => {
    if (spec.harness && !refreshingModels) refreshModels.mutate(spec.harness);
  };
  const set = (f: (s: LaunchSpec) => LaunchSpec) => update(f);
  const target = (spec.target.kind === "project" || spec.target.kind === "predraft") ? projects.find((p) => p.id === (spec.target as { project_id: string }).project_id) : existing ? projects.find((p) => p.id === existing.project_id) : source ? projects.find((p) => p.id === source.project_id) : null;
  const repo = target?.gh_repo ?? target?.gh_upstream ?? null;
  const inScope = (p: Project) => Number(!!scope.workspace && scope.hasProject(p.id));
  const sorted = [...projects.filter((p) => !p.is_temp)].sort((a, b) => inScope(b) - inScope(a) || Number(b.pinned) - Number(a.pinned) || a.name.localeCompare(b.name));
  const model = defaults?.models.find((m) => m.id === (spec.model ?? variant?.model ?? defaults?.model));
  const efforts = model?.efforts.length ? model.efforts : (defaults?.efforts ?? []);
  const scheduled = conditions.length > 0;
  const budgetInput = budgetText ?? (spec.budget_usd != null ? String(spec.budget_usd) : "");
  const budgetValid = parseUsd(budgetInput) !== undefined;
  const missingVariant = !!spec.variant && !variant;
  // The daemon renders and checks the prompt at launch; Launch does not wait for a preview.
  const hasPrompt = !!spec.text.trim() || items > 0 || spec.presets.length > 0;
  const blocker = !spec.harness ? "Pick a harness in the options" : missingVariant ? "The harness variant no longer exists" : spec.target.kind === "none" ? "Pick a project" : !hasPrompt ? "Write a prompt" : !budgetValid ? "Fix the cost limit" : (cron ? !cronValid : !delayValid || !!conditionsError(conditions)) ? "Fix the start or repeat options" : null;
  const unsupportedImages = !!harness && !harness.capabilities.images && (spec.attachments ?? []).some((f) => f.data_url.startsWith("data:image/"));
  const canLaunch = !blocker && !readingAttachments && !unsupportedImages;

  const addProjectContext = (p: Project) =>
    set((s) => (s.context_projects.some((c) => c.project_id === p.id) || ((s.target.kind === "project" || s.target.kind === "predraft") && s.target.project_id === p.id) ? s : { ...s, context_projects: [...s.context_projects, { project_id: p.id, branch: null, editable: false }] }));

  const recentContexts: { key: string; label: string; add: () => void }[] = [];
  const seenContexts = new Set<string>();
  for (const session of recentSessions.filter((s) => s.launch_spec.harness && s.last_seq > 0 && s.status !== "error")) {
    for (const c of session.launch_spec.context_projects) {
      const p = projects.find((p) => p.id === c.project_id);
      const key = `project:${c.project_id}:${c.branch ?? ""}`;
      if (!p || seenContexts.has(key)) continue;
      seenContexts.add(key);
      if (p.id !== target?.id && !spec.context_projects.some((p) => p.project_id === c.project_id)) recentContexts.push({ key, label: `${p.name}${c.branch ? ` · ${c.branch}` : ""}`, add: () => set((s) => ({ ...s, context_projects: [...s.context_projects,c] })) });
    }
    for (const g of session.launch_spec.github_refs) {
      if (seenContexts.has(g.url)) continue;
      seenContexts.add(g.url);
      if (!spec.github_refs.some((r) => r.url === g.url)) recentContexts.push({ key:g.url, label:g.number ? `${g.repo}#${g.number}` : g.title ?? g.url, add:() => set((s) => ({ ...s, github_refs:[...s.github_refs,g] })) });
    }
    for (const f of session.launch_spec.files ?? []) {
      const key = JSON.stringify(f);
      if (seenContexts.has(key)) continue;
      seenContexts.add(key);
      if (!(spec.files ?? []).some((r) => JSON.stringify(r) === key)) recentContexts.push({ key, label:`${f.path}:${f.start_line}`, add:() => set((s) => ({ ...s, files:[...(s.files ?? []),f] })) });
    }
  }
  const launch = async () => {
    if (!canLaunch || launching || attachmentRead.current) return;
    setLaunching(true);
    // The target project joins this window's workspace, so the new agent shows up here.
    const draft = useComposer.getState().spec;
    const spec = { ...draft, workspace_id: scope.workspace?.id ?? null, worktree: draft.target.kind === "project" && !gitTarget ? null : draft.worktree };
    try {
      if (cron) {
        await api.post("/api/launch/crons", { spec, schedule: cron.schedule, conditions: cron.conditions });
        setCron(null);
        discard();
        void navigate({ to: "/agents" });
        toast("Cron agent created. Find it in the Cron group in Agents.");
        return;
      }
      if (scheduled) {
        await api.post("/api/launch/schedules", { spec, conditions });
        discard();
        void navigate({ to: "/agents" });
        toast("Agent scheduled. You can start it now or cancel in Agents.");
        return;
      }
      const s = await api.post<Session>("/api/sessions", spec);
      setUnconfirmedLaunch(null);
      if (s.status !== "error") useComposer.getState().rememberLaunch(spec, s.started_at);
      discard();
      const where = projects.find((p) => p.id === s.project_id)?.name ?? s.cwd.split("/").pop();
      const openIt = () => {
        showSession(s.id);
        void navigate({ to: "/agents" });
      };
      if (openAfterLaunch) { openIt(); toast("✓ Delivered: agent launched."); }
      else toast(`✓ Delivered: ${harnessName(s.harness)} started in ${where}`, { label: "Open", run: openIt });
    } catch (e) {
      if (!cron && !scheduled && !(e instanceof ApiError && e.operationCompleted)) setUnconfirmedLaunch(spec);
      toastError(e);
    } finally {
      setLaunching(false);
    }
  };
  const launchLabel = launching ? (cron ? "Creating…" : scheduled ? "Scheduling…" : "◷ Sending…") : cron ? "Create cron" : scheduled ? "Schedule" : "Launch";
  const launchButton = (small?: boolean) => (
    <button className={`btn btn-primary ${small ? "btn-sm" : ""}`} disabled={!canLaunch || launching} title={unsupportedImages ? "This harness does not support attached images" : blocker ?? undefined} onClick={() => void launch()}>
      <Icon name="play" size={small ? 12 : 14} />
      {small && launchLabel === "Launch" ? "Launch draft" : launchLabel}
    </button>
  );
  const where = spec.target.kind === "temp" ? "temporary folder" : (target?.name ?? "no project");

  if (minimized) {
    if (!pillSlot) return null;
    return createPortal(
      <div className={`flex max-w-[520px] min-w-0 items-center gap-2 rounded-xl border border-line bg-paper py-1 pr-1 pl-3 transition-shadow ${flash ? "ring-[3px] ring-teal" : ""}`}>
        <b className="text-[12.5px]">Draft</b>
        <span className="min-w-0 truncate text-xs text-ink-2" title={spec.text || undefined}>{where}{spec.text.trim() ? ` · ${spec.text.trim().split("\n")[0]}` : ""}</span>
        <span className={`flex-none rounded-full px-2 py-0.5 text-[11.5px] ${flash ? "bg-teal-soft font-semibold text-teal-ink" : "bg-sunk text-ink-2"}`}>{items} {items === 1 ? "item" : "items"}</span>
        <button className="btn btn-sm" title="Open the composer" onClick={() => setMinimized(false)}>Open</button>
        {launchButton(true)}
      </div>,
      pillSlot,
    );
  }

  const harnessOptions = [...harnesses.map((h) => ({ key: h.id, harness: h.id, variant: null as HarnessVariant | null, label: h.name })), ...variants.map((v) => ({ key: `variant:${v.id}`, harness: v.harness, variant: v, label: v.name }))];
  const modelLabel = spec.model ?? variant?.model ?? defaults?.model ?? "default model";
  const effortLabel = spec.effort ?? variant?.effort ?? defaults?.effort ?? null;
  const worktreeMode = spec.target.kind === "project" && !!spec.worktree && gitTarget;
  const runIn = spec.target.kind === "session_folder" ? "previous agent’s folder" : spec.target.kind === "temp" ? "new folder" : spec.target.kind === "predraft" ? "pre-draft PR" : spec.target.kind === "existing_predraft" ? "existing pre-draft" : worktreeMode ? "new worktree" : spec.target.kind === "project" ? "checkout" : null;
  const baseLabel = spec.worktree?.base ?? targetBranches.data?.current ?? "current branch";
  const billingText = payg ? `Pay as you go: Launchpad does not monitor subscription quotas for ${variant!.name}. ${spec.budget_usd != null ? `New turns stop once the reported cost reaches ${usd(spec.budget_usd)}.` : "No cost limit is set."}` : harness && !harness.capabilities.quota ? `This harness uses its native provider billing settings. Launchpad does not monitor its subscription quotas.${harness.capabilities.cost ? " A cost limit stops new turns at a reported cost." : ""}` : spec.allow_paid_usage ? "Additional credits may be used after included quota runs out." : "Pause at included quota limits and continue after reset. Best effort; some paid usage may occur before interruption.";
  const presetLabels = (config?.presets ?? []).filter((p) => spec.presets.includes(p.id)).map((p) => p.label);

  return (
    <aside onDragOver={(e) => { if (e.dataTransfer.types.includes("Files")) { e.preventDefault(); e.dataTransfer.dropEffect = "copy"; } }} onDrop={(e) => { if (e.dataTransfer.types.includes("Files")) { e.preventDefault(); void addAttachments(Array.from(e.dataTransfer.files)); } }} className={`flex w-[440px] max-w-full flex-none flex-col border-l border-line bg-paper ${overlay ? "absolute top-0 right-0 bottom-0 z-40 shadow-[-18px_0_40px_-12px_rgba(23,32,43,.45)]" : "relative shadow-[-14px_0_28px_-20px_rgba(23,32,43,.35)]"}`}>
      <div className="flex items-center gap-1.5 border-b border-line px-3.5 py-2.5">
        <span className="flex-1 whitespace-nowrap text-sm font-bold">New agent</span>
        <button className="btn btn-sm btn-ghost" title="Minimize to a draft pill; + Add buttons keep adding context" onClick={() => setMinimized(true)}>Minimize</button>
        <button className="iconbtn" title="Close (the draft is kept)" onClick={close}>
          <Icon name="x" size={14} />
        </button>
      </div>

      <div className="flex min-h-0 flex-1 flex-col gap-3 overflow-y-auto px-3.5 py-3">
        <PromptField
          disabled={launching || readingAttachments}
          onPaste={(e) => { if (launching || attachmentRead.current) return; const files = pastedImages(e); if (files) void addAttachments(files); }}
          value={spec.text}
          onChange={(text) => set((s) => ({ ...s, text }))}
          projects={sorted}
          repo={repo}
          onPickProject={addProjectContext}
          onPickItem={(it) =>
            set((s) =>
              s.github_refs.some((g) => g.url === it.url)
                ? s
                : { ...s, github_refs: [...s.github_refs, { url: it.url, kind: it.kind, repo: it.repo, number: it.number, title: it.title }] },
            )
          }
        />

        <div>
          <input ref={attachmentInput} type="file" multiple hidden onChange={(e) => { void addAttachments(Array.from(e.target.files ?? [])); e.target.value = ""; }} />
          <button className="btn btn-sm" disabled={launching || readingAttachments} onClick={() => attachmentInput.current?.click()} title="Attach files or images; you can also drop files or paste a screenshot"><Icon name="clip" size={14} />Attach files / images</button>
          <div className="mt-1 text-xs text-ink-3">{readingAttachments ? "Reading attachments…" : "Drop files or paste a screenshot into the prompt. Up to 8 files, 5 MB each, 20 MB total."}</div>
          {(spec.attachments ?? []).length > 0 && <div className="mt-2 flex flex-wrap items-start gap-2">{spec.attachments.map((file, i) => <div key={i} className="relative max-w-full rounded-md border border-line bg-chrome p-1.5 pr-6">
            {file.data_url.startsWith("data:image/") && <img src={file.data_url} alt={file.name} className="mb-1 h-16 max-w-28 object-contain" />}
            <div className="max-w-48 truncate text-xs" title={file.name}>{file.name}</div>
            <button className="iconbtn absolute top-0 right-0" title={`Remove ${file.name}`} aria-label={`Remove ${file.name}`} disabled={launching || readingAttachments} onClick={() => set((s) => ({ ...s, attachments: s.attachments.filter((_, j) => j !== i) }))}><Icon name="x" size={12} /></button>
          </div>)}</div>}
          {unsupportedImages && <div className="mt-1 text-xs text-danger">This harness does not support attached images. Remove them or choose another harness.</div>}
        </div>

        <div className="grid grid-cols-2 gap-2.5">
          <Field label="Project">
            <Popover
              width={300}
              trigger={({ toggle }) => (
                <SelectButton onClick={toggle}>
                  <Icon name="folder" size={14} />
                  {spec.target.kind === "temp" ? "New temporary folder" : (target?.name ?? <span className="text-ink-3">Pick a project</span>)}
                </SelectButton>
              )}
            >
              {(closeMenu) => (
                <ProjectPicker
                  projects={sorted}
                  workspaceName={scope.workspace?.name}
                  hasProject={scope.hasProject}
                  selected={target?.id}
                  temporary={spec.target.kind === "temp"}
                  projectsRoot={config?.projects_root ?? "~/projects"}
                  onPick={(p) => {
                    set((s) => p ? ({ ...s, target: { kind: s.target.kind === "predraft" ? "predraft" : "project", project_id: p.id }, context_projects: s.context_projects.filter((c) => c.project_id !== p.id) }) : ({ ...s, target: { kind: "temp" } }));
                    closeMenu();
                  }}
                />
              )}
            </Popover>
          </Field>
          <Field label="Run in">
            {target ? <select className="field-input h-8 w-full" aria-label="Run in" value={worktreeMode ? "worktree" : spec.target.kind} onChange={(e) => { const v = e.target.value; set((s) => ({ ...s, target: { kind: v === "predraft" ? "predraft" : "project", project_id: target.id }, worktree: v === "worktree" ? (s.worktree ?? { base: null, include_changes: false, merge_when_done: false }) : v === "project" ? null : s.worktree, presets: s.presets.filter((id) => id !== "draft-pr" && id !== "worktree") })); }}>
              {spec.target.kind === "session_folder" && <option value="session_folder">Previous agent’s folder</option>}{existing && <option value="existing_predraft">Pre-draft: {existing.title}</option>}{gitTarget && <option value="worktree">New worktree</option>}<option value="project">{gitTarget ? "Project checkout" : "Project folder (no Git)"}</option>{gitTarget && <option value="predraft">Pre-draft PR</option>}
            </select> : <div className="field-input h-8">{spec.target.kind === "session_folder" ? source?.cwd ?? "Source session unavailable" : spec.target.kind === "temp" ? "A new folder" : "—"}</div>}
            {spec.target.kind === "session_folder" && <div className="mt-1 break-all text-xs text-ink-3">{source?.cwd ?? "Source session unavailable"}</div>}
            {target && spec.target.kind === "project" && !worktreeMode && <div className="mt-1 text-xs text-ink-3">{gitTarget ? <TargetCheckout projectId={target.id} /> : "Runs in the project folder"}</div>}
            {worktreeMode && <div className="mt-1 text-xs text-ink-3" title="Launchpad creates a new branch and folder before the agent starts; your checkout is not modified.">Your checkout stays untouched</div>}
          </Field>
        </div>

        {worktreeMode && target && <div className="flex flex-wrap items-center gap-x-3 gap-y-1.5 rounded-lg border border-line-2 bg-chrome px-2.5 py-1.5 text-xs text-ink-2">
          <span className="inline-flex items-center gap-1">From<Branch projectId={target.id} branch={spec.worktree?.base ?? null} currentLabel="current branch" onChange={(base) => set((s) => ({ ...s, worktree: { ...(s.worktree ?? { include_changes: false, merge_when_done: false }), base } }))} /></span>
          <label className="inline-flex items-center gap-1.5" title="Copy the uncommitted changes of your checkout into the new worktree (only when starting from the current branch)">
            <input type="checkbox" className="accent-teal" disabled={!!spec.worktree?.base && spec.worktree.base !== targetBranches.data?.current} checked={!!spec.worktree?.include_changes} onChange={(e) => set((s) => ({ ...s, worktree: { ...s.worktree!, include_changes: e.target.checked } }))} />
            Copy uncommitted changes
          </label>
          <label className="inline-flex items-center gap-1.5 whitespace-nowrap" title="Merge into the base branch: the agent asks Launchpad to merge when it is done; Launchpad waits for a clean checkout, rebases and fast-forwards. Conflicts go back to the agent.">
            When done
            <select className="field-input h-6 py-0 text-xs" aria-label="When done" value={spec.worktree?.merge_when_done ? "merge" : "keep"} onChange={(e) => set((s) => ({ ...s, worktree: { ...s.worktree!, merge_when_done: e.target.value === "merge" } }))}>
              <option value="keep">Keep branch for review</option>
              <option value="merge">Merge into {baseLabel}</option>
            </select>
          </label>
        </div>}

        <div className="flex flex-wrap items-center gap-1.5">
          {(spec.agent_traces ?? []).map((trace) => (
            <span key={trace.session_id} className="inline-flex h-7 max-w-full items-center gap-1.5 rounded-[7px] border border-line bg-chrome px-2 text-xs">
              <Icon name="agents" size={13} />
              <span className="min-w-0 truncate" title={`Stored trace through event #${trace.through_seq} · ${trace.session_id}`}>Trace · {sessions?.find((s) => s.id === trace.session_id)?.title ?? "Missing agent"}</span>
              <button title="Remove agent trace" className="text-ink-3 hover:text-ink" onClick={() => set((s) => ({ ...s, agent_traces: s.agent_traces.filter((t) => t.session_id !== trace.session_id) }))}><Icon name="x" size={12} /></button>
            </span>
          ))}
          {spec.github_refs.map((g) => {
            const ic = itemIcon(g.kind === "pr" ? "pr" : "issue", "open");
            return (
              <span key={g.url} className="inline-flex h-7 max-w-full items-center gap-1.5 rounded-[7px] border border-line bg-chrome px-2 text-xs" title={g.title ?? g.url}>
                <Icon name={g.kind === "comment" || g.kind === "review_comment" ? "comment" : ic.name} size={13} className={ic.color} />
                <span className="font-mono">{g.number === null ? g.kind : `#${g.number}`}</span>
                <span className="min-w-0 truncate text-ink-2">{g.title ?? g.url}</span>
                <button className="text-ink-3 hover:text-ink" title="Remove" onClick={() => set((s) => ({ ...s, github_refs: s.github_refs.filter((x) => x.url !== g.url) }))}><Icon name="x" size={12} /></button>
              </span>
            );
          })}
          {(spec.files ?? []).map((f, i) => (
            <span key={`${f.root}\n${f.rev}\n${f.path}\n${f.start_line}-${f.end_line}`} className="inline-flex h-7 max-w-full items-center gap-1.5 rounded-[7px] border border-line bg-chrome px-2 text-xs" title={`${f.root}/${f.path}${f.rev ? ` at ${f.rev}` : ""}`}>
              <Icon name="file" size={13} />
              <span className="min-w-0 truncate font-mono">
                {f.path}
                {f.start_line !== null && `:${f.start_line}${f.end_line !== null && f.end_line > f.start_line ? `-${f.end_line}` : ""}`}
              </span>
              <button className="text-ink-3 hover:text-ink" title="Remove" onClick={() => set((s) => ({ ...s, files: s.files.filter((_, j) => j !== i) }))}><Icon name="x" size={12} /></button>
            </span>
          ))}
          {spec.context_projects.map((c) => (
            <span key={c.project_id} className="inline-flex h-7 max-w-full items-center gap-1.5 rounded-[7px] border border-line bg-chrome px-2 text-xs">
              <Icon name="folder" size={13} />
              <span className="font-mono">{projects.find((x) => x.id === c.project_id)?.name ?? "?"}</span>
              <Branch projectId={c.project_id} branch={c.branch} onChange={(branch) => set((s) => ({ ...s, context_projects: s.context_projects.map((x) => (x.project_id === c.project_id ? { ...x, branch } : x)) }))} />
              <button
                className={`inline-flex h-5 items-center rounded-[5px] border px-1.5 text-[11px] ${c.editable ? "border-teal bg-teal-soft font-semibold text-teal-ink" : "border-line bg-paper text-ink-2"}`}
                title={c.editable ? (spec.target.kind === "project" && !worktreeMode ? "Editable: the agent may modify this checkout. Click to make it read-only." : "Editable: the agent gets its own worktree of this project. Click to make it read-only.") : "Read-only reference. Click to let the agent modify it."}
                aria-pressed={c.editable}
                onClick={() => set((s) => ({ ...s, context_projects: s.context_projects.map((x) => (x.project_id === c.project_id ? { ...x, editable: !x.editable } : x)) }))}
              >{c.editable ? "editable" : "read-only"}</button>
              <button className="text-ink-3 hover:text-ink" title="Remove" onClick={() => set((s) => ({ ...s, context_projects: s.context_projects.filter((x) => x.project_id !== c.project_id) }))}><Icon name="x" size={12} /></button>
            </span>
          ))}
          <AddContextMenu
            projects={sorted.filter((p) => p.id !== target?.id && !spec.context_projects.some((c) => c.project_id === p.id))}
            recent={recentContexts}
            sessions={recentSessions}
            selectedTraces={spec.agent_traces ?? []}
            onProject={addProjectContext}
            onGitHub={(r) => set((s) => (s.github_refs.some((g) => g.url === r.url) ? s : { ...s, github_refs: [...s.github_refs, r] }))}
            onTrace={(trace) => set((s) => ({ ...s, agent_traces: [...(s.agent_traces ?? []), trace] }))}
          />
        </div>

        {missingVariant && <div className="rounded-lg bg-attn-soft px-3 py-2 text-xs text-attn">The harness variant {spec.variant} no longer exists. Pick a harness or variant in the options.</div>}

        <button
          className={`flex w-full flex-wrap items-center gap-1 rounded-lg border px-2 py-1.5 text-left ${showOptions ? "border-[#c4ccd7] bg-chrome" : "border-line bg-paper hover:border-[#c4ccd7]"}`}
          aria-expanded={showOptions}
          title={showOptions ? "Hide the options" : "Show the options"}
          onClick={() => setShowOptions((v) => !v)}
        >
          <Token warn={!spec.harness || missingVariant}>{spec.harness && <HarnessMark harness={spec.harness} size={13} />}{variant?.name ?? (spec.harness ? harnessName(spec.harness) : "No harness")}</Token>
          <Token>{modelLabel}</Token>
          {!!efforts.length && effortLabel && <Token>{effortLabel}</Token>}
          {runIn && <Token>{runIn}</Token>}
          {worktreeMode && spec.worktree?.merge_when_done && <Token title={`The agent asks Launchpad to merge into ${baseLabel} when it is done`}>merge when done</Token>}
          {presetLabels.map((l) => <Token key={l}>{l}</Token>)}
          <Token warn={scheduled} title={scheduled ? "Starts when all start conditions are met" : undefined}>{scheduled ? `start: ${conditions.length} condition${conditions.length === 1 ? "" : "s"}` : "now"}</Token>
          <Token warn={!!cron} title={cron ? `Cron schedule ${cron.schedule}` : undefined}>{cron ? `cron ${cron.schedule}` : "once"}</Token>
          <Token warn={payg || spec.allow_paid_usage} title={billingText}>{payg ? "pay as you go" : harness && !harness.capabilities.quota ? "provider billing" : spec.allow_paid_usage ? "paid usage on" : "subscription only"}</Token>
          {harness?.capabilities.cost && <Token warn={spec.budget_usd != null || !budgetValid}>{spec.budget_usd != null ? `limit ${usd(spec.budget_usd)}` : "no cost limit"}</Token>}
          <span className="ml-auto flex items-center gap-0.5 pl-1 text-[11.5px] text-ink-3">Options<Icon name="chev" size={12} className={showOptions ? "rotate-180" : ""} /></span>
        </button>

        {showOptions && <div className="grid gap-3 rounded-lg border border-line bg-chrome p-2.5">
          <OptionRow label="Harness">
            <span className="inline-flex flex-wrap gap-0.5 rounded-lg bg-sunk p-0.5">
              {harnessOptions.map((o) => (
                <button
                  key={o.key}
                  title={o.variant ? `${harnessName(o.harness)} variant${o.variant.billing === "pay_as_you_go" ? " · pay as you go" : ""}` : undefined}
                  onClick={() => { setBudgetText(null); set((s) => {
                    const same = s.harness === o.harness && (s.variant ?? null) === (o.variant?.id ?? null);
                    const cost = harnesses.find((h) => h.id === o.harness)?.capabilities.cost;
                    const run = same ? { model: s.model, effort: s.effort } : rememberedRun(o.harness, o.variant?.id ?? null);
                    return { ...s, harness: o.harness, variant: o.variant?.id ?? null, ...run, budget_usd: same ? s.budget_usd : cost ? (o.variant?.budget_usd ?? null) : null };
                  }); }}
                  className={`inline-flex h-[26px] shrink-0 items-center gap-1.5 whitespace-nowrap rounded-md px-2.5 text-[12.5px] ${
                    spec.harness === o.harness && (spec.variant ?? null) === (o.variant?.id ?? null) ? "bg-paper font-semibold text-ink shadow-[0_1px_2px_rgba(23,32,43,.12),inset_0_0_0_1px_var(--color-line)]" : "text-ink-2"
                  }`}
                >
                  <HarnessMark harness={o.harness} size={15} />
                  {o.label}
                </button>
              ))}
            </span>
          </OptionRow>
          <OptionRow label="Model" end={
            <button className="inline-flex size-5 items-center justify-center rounded hover:bg-select hover:text-ink disabled:opacity-50" disabled={!spec.harness || refreshingModels} onClick={refreshModelList} title={refreshingModels ? "Refreshing models…" : "Refresh models"} aria-label={refreshingModels ? "Refreshing models" : "Refresh models"} aria-busy={refreshingModels}>
              <Icon name="refresh" size={12} className={refreshingModels ? "animate-spin" : ""} />
            </button>
          }>
            <Popover
              width={300}
              trigger={({ toggle }) => (
                <SelectButton onClick={toggle}>
                  {spec.model ?? (
                    <>
                      Default <span className="text-ink-3">· {variant?.model ?? defaults?.model ?? "harness default"}</span>
                    </>
                  )}
                </SelectButton>
              )}
            >
              {(closeMenu) => (
                <>
                  <MenuItem icon={<Icon name="refresh" size={13} className={refreshingModels ? "animate-spin" : ""} />} label={refreshingModels ? "Refreshing models…" : "Refresh models"} detail="Query the selected harness again" disabled={!spec.harness || refreshingModels} onClick={refreshModelList} />
                  {!defaults?.models.length && <div className="px-2 py-1 text-xs text-ink-3">No models loaded. Click Refresh models to retry.</div>}
                  <MenuItem label={`Default (${variant?.model ?? defaults?.model ?? "harness default"})`} detail={variant?.model ? `Default of ${variant.name}` : "What the agent picks on its own"} active={!spec.model} onClick={() => (set((s) => ({ ...s, model: null })), closeMenu())} />
                  {(defaults?.models ?? []).map((m) => (
                    <MenuItem key={m.id} label={m.display_name || m.id} detail={`${m.id} · ${m.description}`} active={spec.model === m.id} onClick={() => (set((s) => ({ ...s, model: m.id })), closeMenu())} />
                  ))}
                </>
              )}
            </Popover>
          </OptionRow>
          {!!efforts.length && <OptionRow label="Effort">
            <Popover
              width={220}
              trigger={({ toggle }) => (
                <SelectButton onClick={toggle}>
                  {spec.effort ?? (
                    <>
                      Default <span className="text-ink-3">· {variant?.effort ?? defaults?.effort ?? "harness default"}</span>
                    </>
                  )}
                </SelectButton>
              )}
            >
              {(closeMenu) => (
                <>
                  <MenuItem label={`Default (${variant?.effort ?? defaults?.effort ?? "harness default"})`} active={!spec.effort} onClick={() => (set((s) => ({ ...s, effort: null })), closeMenu())} />
                  {efforts.map((e) => (
                    <MenuItem key={e} label={e} active={spec.effort === e} onClick={() => (set((s) => ({ ...s, effort: e })), closeMenu())} />
                  ))}
                </>
              )}
            </Popover>
          </OptionRow>}
          <OptionRow label="Presets" end={<button className="ml-1 font-normal text-teal-ink hover:underline" onClick={() => void navigate({ to: "/settings", hash: "presets" })}>Edit</button>}>
            <div className="flex flex-wrap gap-1.5">
              {(config?.presets ?? []).filter((p) => p.id !== "draft-pr" && ((spec.target.kind !== "predraft" && spec.target.kind !== "existing_predraft" && !worktreeMode) || p.id !== "worktree")).map((p) => {
                const on = spec.presets.includes(p.id);
                return (
                  <button
                    key={p.id}
                    title={p.text}
                    onClick={() => set((s) => ({ ...s, presets: on ? s.presets.filter((x) => x !== p.id) : [...s.presets, p.id] }))}
                    className={`inline-flex h-7 items-center gap-1.5 rounded-full border px-[11px] text-[12.5px] ${on ? "border-ink bg-ink font-semibold text-white" : "border-line bg-paper text-ink-2 hover:border-[#c4ccd7]"}`}
                  >
                    {on && <Icon name="check" size={12} />}
                    {p.label}
                  </button>
                );
              })}
              {config?.presets.length === 0 && <span className="pt-1.5 text-xs text-ink-3">No preset.</span>}
            </div>
          </OptionRow>
          {harness?.capabilities.cost && <OptionRow label="Cost limit">
            <span className="grid gap-1">
              <input className="field-input w-28 font-mono text-xs" aria-label="Cost limit in USD" placeholder="No limit" value={budgetInput} onChange={(e) => { setBudgetText(e.target.value); const v = parseUsd(e.target.value); if (v !== undefined) set((s) => ({ ...s, budget_usd: v })); }} />
              <span className={`text-[11px] ${budgetValid ? "text-ink-3" : "text-attn"}`}>{budgetValid ? "USD of reported cost. At the limit, the running turn stops and no new turn starts. Empty: no limit." : "Enter a positive amount in USD, or leave empty."}</span>
            </span>
          </OptionRow>}
          {harness?.capabilities.quota && !payg && <OptionRow label="Paid usage">
            <label className="inline-flex items-center gap-[7px] pt-1.5 text-xs text-ink-2" title={billingText}>
              <input type="checkbox" className="accent-teal" checked={spec.allow_paid_usage} onChange={(e) => set((s) => ({ ...s, allow_paid_usage: e.target.checked }))} />
              Allow paid usage after included quota runs out
            </label>
          </OptionRow>}
          <CronSelector cron={cron} harness={spec.harness} onChange={(c) => { setCron(c); if (c) setConditions([]); }} onValidityChange={setCronValid} />
          {!cron && <DelaySelector conditions={conditions} harness={spec.harness} onChange={setConditions} onValidityChange={setDelayValid} />}
        </div>}

        <Popover width={400} trigger={({ toggle }) => <button className="btn btn-sm w-fit" onClick={toggle}><Icon name="eye" size={12} />Preview prompt</button>}>
          {() => <PromptPreview spec={spec} title={`Prompt sent to ${spec.harness ? harnessName(spec.harness) : "the agent"}`} />}
        </Popover>
      </div>

      <div className="flex items-center gap-2 border-t border-line bg-chrome px-3.5 py-2.5">
        <label className="mr-auto inline-flex items-center gap-[7px] text-xs text-ink-2" style={scheduled || cron ? { visibility: "hidden" } : undefined}>
          <input type="checkbox" className="accent-teal" disabled={scheduled || !!cron} checked={openAfterLaunch} onChange={(e) => setOpenAfterLaunch(e.target.checked)} />
          Open after launch
        </label>
        <button className="btn btn-ghost" onClick={() => { setCron(null); discard(); }} title="Clear the composer">
          Discard
        </button>
        {unconfirmedLaunch && <div role="status" className="text-[11.5px] text-ink-3">Delivery not confirmed. Retry unchanged to check the result. Inspect Agents before starting another attempt.<button className="btn btn-sm mt-1" disabled={launching} onClick={() => void abandonOperation("/api/sessions", unconfirmedLaunch).then(() => setUnconfirmedLaunch(null)).catch(toastError)}>I checked Agents — start a new attempt</button></div>}
        {launchButton()}
      </div>
    </aside>
  );
}

function TargetCheckout({ projectId }: { projectId: string }) {
  const q = useQuery({
    queryKey: ["branches", projectId],
    queryFn: () => api.get<{ current: string | null; branches: string[] }>(`/api/projects/${projectId}/branches`),
    staleTime: 30_000,
  });
  return (
    <>
      Checkout{q.data?.current && <span className="font-mono text-xs">· {q.data.current}</span>}
    </>
  );
}
