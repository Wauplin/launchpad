import { OutdatedRunnerNotice, RunnerChoice, runnerName, ToolsChoice, useLaunchRunners, useLaunchTools } from "./LaunchExtras";
import { isPhone } from "@/lib/phone";
import { useBackClose } from "@/mobile/back";
import { Sheet } from "@/mobile/Sheet";
// Composer drawer (sketch S2): stays on the right while you navigate, so context can be added from
// anywhere. It minimizes to a draft pill, and covers the content on narrow windows. Prompt first, then
// project, context chips and one summary line of options; "Preview prompt" shows the exact prompt.

import { abandonOperation } from "@/lib/operations";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Link, useNavigate } from "@tanstack/react-router";
import { useEffect, useRef, useState } from "react";
import type { HarnessDefaults } from "@/gen/HarnessDefaults";
import type { HarnessKind } from "@/gen/HarnessKind";
import type { Profile } from "@/gen/Profile";
import { isImageAttachment, pastedImages, readAttachments } from "@/lib/attachments";
import { ATTACH_LIMITS, AttachmentList } from "@/features/agents/MessageAttachments";
import type { LaunchSpec } from "@/gen/LaunchSpec";
import type { Preview } from "@/gen/Preview";
import type { StartPreview } from "@/gen/StartPreview";
import { StartPreviewDialog } from "./StartPreview";
import type { Predraft } from "@/gen/Predraft";
import type { Project } from "@/gen/Project";
import type { Session } from "@/gen/Session";
import type { WorktreeInfo } from "@/gen/WorktreeInfo";
import type { WhenDone } from "@/gen/WhenDone";
import { reusableWorktrees } from "./worktreeChoices";
import { pullRequestOf } from "./pullRequest";
import { HarnessMark, useHarnessName, useHarnesses, useHarnessInfo } from "@/components/HarnessMark";
import { Icon } from "@/components/Icon";
import { VoiceButton } from "@/components/VoiceButton";
import { VoiceCaption } from "@/components/VoiceCaption";
import { useDictation } from "@/lib/voice/useDictation";
import { MenuItem, Popover } from "@/components/Popover";
import { toast, toastError } from "@/components/Toast";
import { Choice, Choices } from "@/components/ui";
import { api, ApiError } from "@/lib/api";
import { useConfig, useHarnessDefaults, useProjects, useSchedules, useSessions } from "@/lib/queries";
import { costUnavailableMessage, supportsModelCost, parseUsd, usd, usesModelReference, useProfileModels, useProfiles } from "@/lib/profiles";
import { ModelSearch } from "@/components/ModelSearch";
import { findModel, pickModel, ProviderPicker } from "@/components/ProviderPicker";
import { browserZone, describeCron } from "@/lib/routines";
import { useScope } from "@/lib/workspace";
import { useZoomScale } from "@/lib/zoom";
import { rememberProjectCompletion, rememberedRun, rememberSuccessfulSession, useComposer } from "@/stores/composer";
import { useLayout } from "@/stores/layout";
import { ProjectPicker } from "./ProjectPicker";
import { SkillChips } from "./SkillChips";
import { AddContextMenu } from "./AddContextMenu";
import { PresetChips, usePresets } from "@/features/agents/MessagePresets";
import { DelaySelector, conditionsError, conditionsSummary } from "./DelaySelector";
import { when } from "@/lib/format";
import type { CronAgent } from "@/gen/CronAgent";
import type { ScheduledLaunch } from "@/gen/ScheduledLaunch";
import { CronSelector, DEFAULT_CRON, type CronDraft } from "./CronSelector";
import { itemIcon, PromptField } from "./PromptField";

function Field({ label, hint, end, children, tour }: { label: string; hint?: string; end?: React.ReactNode; children: React.ReactNode; tour?: string }) {
  return (
    <div className="grid min-w-0 grid-cols-1 content-start gap-1.5" data-tour={tour}>
      <div className="flex items-center gap-2 text-[12px] font-[650] text-ink-2">
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
    <button type="button" onClick={onClick} className="field-input flex min-w-0 items-center gap-2 text-left">
      <span className="flex min-w-0 flex-1 items-center gap-2 truncate">{children}</span>
      <Icon name="chev" size={14} className="flex-none text-ink-3" />
    </button>
  );
}

/** Branch menu: a full-width field in the options (`field`), else a small inline chip. */
function Branch({ projectId, branch, onChange, currentLabel = "checkout", field = false }: { projectId: string; branch: string | null; onChange: (b: string | null) => void; currentLabel?: string; field?: boolean }) {
  const q = useQuery({
    queryKey: ["branches", projectId],
    queryFn: () => api.get<{ current: string | null; branches: string[] }>(`/api/projects/${projectId}/branches`),
    staleTime: 30_000,
  });
  return (
    <Popover
      width={260}
      trigger={({ toggle }) => field ? (
        <SelectButton onClick={toggle}><Icon name="branch" size={12} className="flex-none text-ink-3" /><span className="truncate">{branch ?? `${currentLabel}${q.data?.current ? ` (${q.data.current})` : ""}`}</span></SelectButton>
      ) : (
        <button onClick={toggle} className="inline-flex h-5 items-center gap-1 rounded-control border border-line bg-paper px-1.5 font-mono text-[12px] whitespace-nowrap">
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
            <MenuItem key={b} icon={<Icon name="branch" size={12} />} label={<span className="font-mono text-[12px]">{b}</span>} active={b === branch} onClick={() => (onChange(b), close())} />
          ))}
          {q.isLoading && <div className="px-2 py-1 text-[12px] text-ink-3">Loading branches…</div>}
        </>
      )}
    </Popover>
  );
}

/** One menu for where a Git folder's agent works: a new worktree from a branch, an existing worktree or an existing Local PR. */
function RunIn({ projectId, label, base, worktrees, selectedWorktree, localPrs, selectedLocalPr, onBranch, onWorktree, onLocalPr }: {
  projectId: string;
  label: string;
  /** The base of a new worktree; undefined when the agent works elsewhere. */
  base: string | null | undefined;
  worktrees: ReturnType<typeof reusableWorktrees>;
  selectedWorktree?: string;
  localPrs: Predraft[];
  selectedLocalPr?: string;
  onBranch: (base: string | null) => void;
  onWorktree: (w: ReturnType<typeof reusableWorktrees>[number]) => void;
  onLocalPr: (p: Predraft) => void;
}) {
  const q = useQuery({
    queryKey: ["branches", projectId],
    queryFn: () => api.get<{ current: string | null; branches: string[] }>(`/api/projects/${projectId}/branches`),
    staleTime: 30_000,
  });
  // A branch that already has a worktree or a Local PR shows once, in its own section.
  const taken = new Set([q.data?.current, ...worktrees.map((w) => w.branch), ...localPrs.map((p) => p.branch)]);
  const branches = (q.data?.branches ?? []).filter((b) => !taken.has(b));
  const heading = (text: string) => <div className="px-[9px] pt-1.5 pb-0.5 text-[11px] font-[650] text-ink-3">{text}</div>;
  return (
    <Popover
      width="100%"
      trigger={({ toggle }) => <SelectButton onClick={toggle}><Icon name="branch" size={12} className="flex-none text-ink-3" /><span className="truncate">{label}</span></SelectButton>}
    >
      {(close) => (
        <>
          {heading("New worktree from")}
          <MenuItem icon={<Icon name="branch" size={12} />} label="Default branch" detail={q.data?.current ?? undefined} active={base === null} onClick={() => (onBranch(null), close())} />
          {branches.map((b) => <MenuItem key={b} icon={<Icon name="branch" size={12} />} label={<span className="font-mono text-[12px]">{b}</span>} active={base === b} onClick={() => (onBranch(b), close())} />)}
          {q.isLoading && <div className="px-2 py-1 text-[12px] text-ink-3">Loading branches…</div>}
          {!!worktrees.length && heading("Existing worktrees")}
          {worktrees.map((w) => <MenuItem key={w.id} label={<span className="font-mono text-[12px]">{w.branch}</span>} detail={w.busy ? "Agent active" : undefined} disabled={w.busy} active={w.id === selectedWorktree} onClick={() => (onWorktree(w), close())} />)}
          {!!localPrs.length && heading("Existing Local PRs")}
          {localPrs.map((p) => <MenuItem key={p.id} label={p.title} detail={p.published_number ? `PR #${p.published_number}` : <span className="font-mono">{p.branch}</span>} active={p.id === selectedLocalPr} onClick={() => (onLocalPr(p), close())} />)}
        </>
      )}
    </Popover>
  );
}

function PromptPreview({ spec, title }: { spec: LaunchSpec; title: string }) {
  const debounced = useDebounced(spec, 250);
  const preview = useQuery({ queryKey: ["preview", debounced], queryFn: () => api.post<Preview>("/api/launch/preview", debounced), placeholderData: (prev) => prev });
  return (
    <div className="grid gap-1 p-1">
      <div className="flex items-center text-[12px] font-[650] text-ink-2">
        {title}
        <span className="ml-auto font-normal text-ink-3">{preview.isFetching ? "Updating…" : `${preview.data?.prompt.length ?? 0} characters`}</span>
      </div>
      <pre className="m-0 max-h-[50vh] overflow-y-auto rounded-control bg-chrome px-2.5 py-2 font-mono text-[12px] leading-normal whitespace-pre-wrap text-ink-2">
        {preview.isError ? <span className="text-danger-ink">{preview.error.message}</span> : preview.data?.prompt || <span className="text-ink-3">{preview.isLoading ? "Rendering…" : "The prompt is empty."}</span>}
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
function useOverlay(scale: number) {
  const [width, setWidth] = useState(window.innerWidth);
  useEffect(() => {
    const onResize = () => setWidth(window.innerWidth);
    window.addEventListener("resize", onResize);
    return () => window.removeEventListener("resize", onResize);
  }, []);
  // The root `zoom` scales CSS pixels; `innerWidth` is not zoomed.
  return width / scale < OVERLAY_BELOW;
}

/** Makes `p` the target of a draft, and drops it from the context folders. */
function withFolder(s: LaunchSpec, p: Project): LaunchSpec {
  // Git folders of Home always run in a fresh worktree.
  const worktree = p.home_git ? (s.worktree ?? { base: null, when_done: p.when_done }) : s.worktree;
  return { ...s, worktree, target: { kind: "project", project_id: p.id }, context_projects: s.context_projects.filter((c) => c.project_id !== p.id) };
}

function Token({ warn, title, children }: { warn?: boolean; title?: string; children: React.ReactNode }) {
  return <span title={title} className={`inline-flex h-[22px] max-w-full items-center gap-1 truncate rounded-control px-1.5 text-[12px] ${warn ? "bg-attn-soft font-semibold text-attn shadow-[inset_0_0_0_1px_var(--color-attn-line)]" : "bg-sunk text-ink-2"}`}>{children}</span>;
}

/** The options panel: inline on desktop, a bottom sheet on the phone. */
function OptionsFrame({ onClose, children }: { onClose: () => void; children: React.ReactNode }) {
  if (isPhone) return <Sheet title="Launch options" onClose={onClose}><div className="grid gap-3 p-3">{children}</div></Sheet>;
  return <div className="grid gap-3 rounded-card border border-line bg-chrome p-2.5">{children}</div>;
}

function OptionRow({ label, end, children }: { label: string; end?: React.ReactNode; children: React.ReactNode }) {
  return (
    <div className="grid grid-cols-[76px_minmax(0,1fr)] items-start gap-2">
      <span className="flex items-center gap-1 pt-1.5 text-[12px] font-[650] text-ink-2">{label}{end}</span>
      <div className="min-w-0">{children}</div>
    </div>
  );
}

export function Composer({ embedded = false }: { embedded?: boolean }) {
  const harnessName = useHarnessName();
  const harnesses = useHarnesses();
  const { open, minimized, setMinimized, spec, update, close, discard, openAfterLaunch, setOpenAfterLaunch, conditions, setConditions, conflict, loadNewer, keepMine, autoLaunchUntil, editing, stopEditing } = useComposer();
  // Phone: the full-screen composer is an overlay; Android back closes it and keeps the draft.
  useBackClose(isPhone && !embedded && open && !minimized, close);
  // Editing a scheduled agent: it must still be queued (or failed) and not started.
  const schedules = useSchedules();
  const edited = editing ? schedules.data?.find((l) => l.id === editing && !l.resume_session_id) : undefined;
  const editGone = !!editing && !!schedules.data && (!edited || edited.status === "starting");
  const browserOnly = spec.target.kind === "browser";
  const projects = useProjects().data ?? [];
  const completionDefault = useMutation({
    mutationFn: ({ projectId, whenDone }: { projectId: string; whenDone: WhenDone }) => api.patch<Project>(`/api/projects/${projectId}`, { when_done: whenDone }),
    onSuccess: (project) => {
      rememberProjectCompletion(project.id, project.when_done);
      queryClient.setQueryData<Project[]>(["projects"], (projects) => projects?.map((p) => p.id === project.id ? project : p));
    },
    onError: toastError,
  });
  const scope = useScope();
  const sessions = useSessions().data;
  useEffect(() => {
    // A queued launch may have started while the browser was disconnected.
    for (const session of sessions ?? []) rememberSuccessfulSession(session);
  }, [sessions]);
  const overlay = useOverlay(useZoomScale());
  const agentPresets = usePresets("agent");
  const defaultsQuery = useHarnessDefaults(spec.harness);
  const defaults = defaultsQuery.data;
  const projectsLoaded = useProjects().isSuccess;
  const configLoaded = useConfig().isSuccess;
  const harness = useHarnessInfo(spec.harness);
  const profiles = useProfiles();
  const profile = profiles.find((v) => v.id === spec.profile) ?? null;
  const payg = profile?.billing === "pay_as_you_go";
  // Switching profiles restores the latest run settings of that profile; the cost limit follows its default.
  const withProfile = (s: LaunchSpec, o: Profile): LaunchSpec => {
    const same = s.harness === o.harness && s.profile === o.id;
    const cost = supportsModelCost(harnesses.find((h) => h.id === o.harness), o.model);
    const run = same ? { model: s.model, effort: s.effort, permission_mode: s.permission_mode } : rememberedRun(o.harness, o.id);
    return { ...s, harness: o.harness, profile: o.id, ...run, budget_usd: same ? s.budget_usd : cost ? (o.budget_usd ?? null) : null };
  };
  // Profiles pointing at another provider list that provider's models instead of the harness ones.
  const providerModels = useProfileModels(profile?.models_url);
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
  const [later, setLater] = useState(false);
  // Schedules' New routine and Schedule a launch open the composer on that timing.
  const requestedTiming = useComposer((s) => s.requestedTiming);
  useEffect(() => {
    if (!open || !requestedTiming) return;
    useComposer.setState({ requestedTiming: null });
    if (requestedTiming === "later") { setCron(null); setLater(true); }
    else { setLater(false); setCron((c) => c ?? { ...DEFAULT_CRON, conditions: DEFAULT_CRON.conditions.filter((x) => harness?.capabilities.quota || x.kind !== "quota_left") }); }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open, requestedTiming]);
  const [cronValid, setCronValid] = useState(true);
  const [startPreview, setStartPreview] = useState<LaunchSpec | null>(null);
  // A scheduled agent opens in "Start later", so removing its last condition does not switch to Start now.
  useEffect(() => { if (editing) { setLater(true); setCron(null); } }, [editing]);
  const [launching, setLaunching] = useState(false);
  // "repo#number" of a detected pull request the owner chose not to push to.
  const [prOptOut, setPrOptOut] = useState<string | null>(null);
  const [unconfirmedLaunch, setUnconfirmedLaunch] = useState<LaunchSpec | null>(null);
  const [readingAttachments, setReadingAttachments] = useState(false);
  const attachmentRead = useRef(false);
  const attachmentInput = useRef<HTMLInputElement>(null);
  // Launch while dictating waits for the last words, then launches.
  const promptInput = useRef<HTMLTextAreaElement>(null);
  const dictation = useDictation({ inputRef: promptInput, onSend: () => void launch(), keep: { read: () => useComposer.getState().spec.text, write: (text) => useComposer.getState().update((s) => ({ ...s, text })) } });
  const addAttachments = async (input: File[] | Promise<File[]>) => {
    if (launching || attachmentRead.current) return;
    attachmentRead.current = true;
    setReadingAttachments(true);
    const draft = useComposer.getState().spec;
    try {
      const files = await input;
      if (!files.length) return;
      const added = await readAttachments(files);
      if (added.some((f) => isImageAttachment(f.data_url)) && harness && !harness.capabilities.images) throw new Error("This harness does not support images.");
      // Do not add a completed read to a different workspace or a discarded draft.
      if (useComposer.getState().spec.attachments === draft.attachments) update((s) => ({ ...s, attachments: [...(s.attachments ?? []), ...added] }));
    } catch (e) { toastError(e); }
    finally { attachmentRead.current = false; setReadingAttachments(false); }
  };
  const [showOptions, setShowOptions] = useState(false);
  const items = (spec.attachments ?? []).length + spec.github_refs.length + (spec.files ?? []).length + spec.context_projects.length + (spec.agent_traces ?? []).length;

  const localPrs = useQuery({ queryKey: ["predrafts"], queryFn: () => api.get<import("@/gen/Predraft").Predraft[]>("/api/predrafts"), enabled: open });
  const taskWorktrees = useQuery({ queryKey: ["worktrees"], queryFn: () => api.get<WorktreeInfo[]>("/api/worktrees"), enabled: open && !browserOnly });
  const existing = spec.target.kind === "existing_predraft" ? localPrs.data?.find((p) => p.id === (spec.target as { predraft_id: string }).predraft_id) : undefined;
  const source = spec.target.kind === "session_folder" ? sessions?.find((s) => s.id === (spec.target as { session_id: string }).session_id) : undefined;
  const targetId = spec.target.kind === "project" || spec.target.kind === "predraft" ? spec.target.project_id : existing?.project_id ?? source?.project_id ?? null;
  const targetBranches = useQuery({
    queryKey: ["branches", targetId],
    queryFn: () => api.get<{ current: string | null; branches: string[] }>(`/api/projects/${targetId}/branches`),
    enabled: open && !!targetId,
    staleTime: 30_000,
    retry: false,
  });
  // Worktrees need a Git repository with a commit; other folders run in place.
  const gitTarget = !!targetId && !targetBranches.isError;
  const worktreeChoices = reusableWorktrees(taskWorktrees.data ?? [], sessions ?? [], targetId);
  const selectedWorktree = worktreeChoices.find((w) => w.path === source?.cwd);
  // Published Local PRs stay available: their agents push to the published branch.
  const existingPrChoices = (localPrs.data ?? []).filter((p) => p.project_id === targetId && !p.archived && !p.merged && (!p.pushed_head || !!p.published_url));
  const recentSessions = (sessions ?? []).filter(scope.hasSession);
  // Recent folders of this workspace, newest first, as one-tap target chips.
  const recentFolders: Project[] = [];
  for (const session of [...recentSessions].sort((a, b) => b.started_at - a.started_at)) {
    const p = projects.find((x) => x.id === session.project_id && !x.is_temp);
    if (p && !recentFolders.includes(p)) recentFolders.push(p);
    if (recentFolders.length >= 4) break;
  }
  // A draft without a folder starts in the workspace's only folder, else the last folder used here.
  const preselected = useRef(false);
  useEffect(() => {
    if (!open) { preselected.current = false; return; }
    if (preselected.current || spec.target.kind !== "none" || !projects.length) return;
    preselected.current = true;
    const mine = projects.filter((p) => !p.is_temp && scope.hasProject(p.id));
    const pick = mine.length === 1 ? mine[0] : recentFolders[0];
    if (pick) update((s) => (s.target.kind === "none" ? withFolder(s, pick) : s));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open, spec.target.kind, projects.length]);
  // Keep every hook above this early return: hook order must not change when the draft opens or closes.
  // A draft without a profile (older drafts, the first launch) takes the first profile of its harness, else the first one.
  useEffect(() => {
    if (!open || spec.profile || !profiles.length) return;
    const pick = profiles.find((p) => p.harness === spec.harness) ?? profiles[0];
    update((s) => (s.profile ? s : withProfile(s, pick)));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open, spec.profile, profiles]);
  // Drafts saved with the former "New Local PR" target run in a new worktree that opens a Local PR when done.
  useEffect(() => {
    if (open && spec.target.kind === "predraft") update((s) => (s.target.kind === "predraft" ? { ...s, target: { kind: "project", project_id: s.target.project_id }, worktree: { base: s.worktree?.base ?? null, when_done: "local_pr" } } : s));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open, spec.target.kind]);
  const launchRunners = useLaunchRunners();
  // The lease takes the workspace of the session's folder, else the launch's.
  const toolsWorkspace = projects.find((p) => p.id === targetId)?.workspace_id ?? spec.workspace_id;
  const launchTools = useLaunchTools(spec, toolsWorkspace, targetId);
  const toolsOff = launchTools.items.filter((t) => !t.on);
  if (!open) return null;

  const refreshingModels = refreshModels.isPending || providerModels.isFetching;
  const refreshModelList = () => {
    if (profile?.models_url) void providerModels.refetch().then((r) => (r.isError ? toastError(r.error) : toast(`${profile.name} models refreshed`)));
    else if (spec.harness && !refreshingModels) refreshModels.mutate(spec.harness);
  };
  const set = (f: (s: LaunchSpec) => LaunchSpec) => update(f);
  const target = (spec.target.kind === "project" || spec.target.kind === "predraft") ? projects.find((p) => p.id === (spec.target as { project_id: string }).project_id) : existing ? projects.find((p) => p.id === existing.project_id) : source ? projects.find((p) => p.id === source.project_id) : null;
  const repo = target?.gh_repo ?? target?.gh_upstream ?? null;
  // A worktree launch that names one pull request of this folder continues it: start from its head and push there.
  const prOf = (s: LaunchSpec) => s.target.kind === "project" && !!s.worktree && gitTarget && target ? pullRequestOf(s.text, s.github_refs, [target.gh_repo, target.gh_upstream]) : null;
  const detectedPr = prOf(spec);
  const pushToPr = detectedPr && prOptOut !== `${detectedPr.repo}#${detectedPr.number}` ? detectedPr : null;
  // Only the folders of this window's workspace: other workspaces show counts, never their content.
  const sorted = scope.projects.filter((p) => !p.is_temp).sort((a, b) => Number(b.pinned) - Number(a.pinned) || a.name.localeCompare(b.name));
  const ownModels = !!profile?.models_url;
  // The model used when none is chosen for this launch; a provider profile has only its own default.
  const fallbackModel = profile?.model ?? (ownModels ? null : defaults?.model) ?? null;
  const fallbackModelLabel = fallbackModel ?? (ownModels ? "none, choose one" : "harness default");
  const model = ownModels ? findModel(providerModels.data, spec.model ?? fallbackModel) : defaults?.models.find((m) => m.id === (spec.model ?? fallbackModel));
  const efforts = ownModels ? [] : model?.efforts.length ? model.efforts : (defaults?.efforts ?? []);
  const modelMissing = !spec.model && !fallbackModel && !!profile && usesModelReference(Object.entries(profile.env));
  const scheduled = conditions.length > 0;
  // Everything a blocker depends on has loaded: what still blocks a top-bar launch needs the owner.
  const settled = projectsLoaded && configLoaded && !!scope.workspace.id && !defaultsQuery.isPending && !(profile?.models_url && providerModels.isPending);
  // Launch ▾ chooses when: now, later (start conditions) or repeatedly (a cron agent).
  const timing: "now" | "later" | "repeat" = cron ? "repeat" : later || scheduled ? "later" : "now";
  const setTiming = (t: "now" | "later" | "repeat") => {
    if (t !== "repeat") setCron(null);
    if (t !== "later") { setLater(false); setConditions([]); }
    if (t === "later") setLater(true);
    if (t === "repeat" && !cron) setCron({ ...DEFAULT_CRON, conditions: DEFAULT_CRON.conditions.filter((c) => harness?.capabilities.quota || c.kind !== "quota_left") });
  };
  const supportsCost = supportsModelCost(harness, spec.model ?? fallbackModel);
  const budgetInput = budgetText ?? (spec.budget_usd != null ? String(spec.budget_usd) : "");
  const budgetValid = !supportsCost || parseUsd(budgetInput) !== undefined;
  const missingProfile = !!spec.profile && !profile;
  // The daemon renders and checks the prompt at launch; Launch does not wait for a preview.
  const hasPrompt = !!spec.text.trim() || items > 0 || agentPresets.some((p) => spec.presets.includes(p.id) && p.text.trim());
  const blocker = editGone ? "This scheduled agent already started or was cancelled" : editing && timing === "later" && !scheduled ? "Add a start condition, or choose Start now" : selectedWorktree?.busy ? "Stop the active agent before using this worktree" : !profiles.length ? "Add a profile in Settings → Profiles" : !spec.harness || !spec.profile ? "Pick a profile in the options" : missingProfile ? "The profile no longer exists" : modelMissing ? `Choose a model for ${profile!.name}` : browserOnly && spec.target.kind === "browser" && !spec.target.grants.length ? "Add a page explicitly in the Chrome extension" : spec.target.kind === "none" ? "Pick a folder" : !scope.workspace.id ? "Waiting for the workspace to load" : !hasPrompt ? "Write a prompt" : !budgetValid ? "Fix the cost limit" : (cron ? !cronValid : !delayValid || !!conditionsError(conditions)) ? "Fix the start or repeat options" : null;
  const unsupportedImages = !!harness && !harness.capabilities.images && (spec.attachments ?? []).some((f) => isImageAttachment(f.data_url));
  const canLaunch = !blocker && !readingAttachments && !unsupportedImages;

  // The first @folder of a draft without a target becomes its target; later ones are context.
  const addProjectContext = (p: Project) =>
    set((s) => s.target.kind === "none" ? withFolder(s, p) : (s.context_projects.some((c) => c.project_id === p.id) || ((s.target.kind === "project" || s.target.kind === "predraft") && s.target.project_id === p.id) ? s : { ...s, context_projects: [...s.context_projects, { project_id: p.id, branch: null, editable: false }] }));


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
    // A temporary folder joins this window's workspace; other folders keep their own.
    const draft = useComposer.getState().spec;
    const pr = prOf(draft);
    const pullRequest = pr && prOptOut !== `${pr.repo}#${pr.number}` ? pr : undefined;
    const spec = { ...draft, workspace_id: scope.workspace.id || null, worktree: draft.target.kind === "project" && !gitTarget ? null : draft.worktree && { ...draft.worktree, pull_request: pullRequest } };
    try {
      if (editing) {
        if (cron) {
          // A routine replaces the scheduled agent.
          const r = await api.post<CronAgent>("/api/launch/crons", { spec, schedule: cron.schedule, conditions: cron.conditions, time_zone: browserZone() });
          await api.delete(`/api/launch/schedules/${editing}`).catch(toastError);
          setCron(null);
          discard();
          void navigate({ to: "/schedules", search: { tab: "routines", routine: r.id } });
          toast("Routine created and scheduled launch cancelled.");
          return;
        }
        // Save keeps the scheduled id, so agents that wait for it keep waiting; Start now saves, then starts it.
        await api.patch(`/api/launch/schedules/${editing}`, timing === "later" ? { spec, conditions } : { spec });
        void queryClient.invalidateQueries({ queryKey: ["schedules"] });
        setLater(false);
        if (timing === "later") {
          discard();
          toast("Scheduled agent saved.");
          return;
        }
        const s = await api.post<Session>(`/api/launch/schedules/${editing}/start`);
        discard();
        showSession(s.id);
        void navigate({ to: "/agents", search: { session: s.id } });
        toast("Delivered: agent launched.");
        return;
      }
      if (cron) {
        const r = await api.post<CronAgent>("/api/launch/crons", { spec, schedule: cron.schedule, conditions: cron.conditions, time_zone: browserZone() });
        setCron(null);
        discard();
        void queryClient.invalidateQueries({ queryKey: ["crons"] });
        void navigate({ to: "/schedules", search: { tab: "routines", routine: r.id } });
        toast(r.next_run_at ? `Routine created: first run ${when(r.next_run_at)}` : "Routine created");
        return;
      }
      if (scheduled) {
        const l = await api.post<ScheduledLaunch>("/api/launch/schedules", { spec, conditions });
        discard();
        void queryClient.invalidateQueries({ queryKey: ["schedules"] });
        void navigate({ to: "/schedules", search: { tab: "launches", launch: l.id } });
        const starts = conditionsSummary(l, sessions ?? []);
        toast(starts ? `Scheduled: starts ${starts.charAt(0).toLowerCase()}${starts.slice(1)}` : "Scheduled");
        return;
      }
      const s = await api.post<Session>("/api/sessions", spec);
      setUnconfirmedLaunch(null);
      if (s.status !== "error") useComposer.getState().rememberLaunch(spec, s.started_at);
      discard();
      const where = s.launch_spec.target.kind === "browser" ? "a browser session" : projects.find((p) => p.id === s.project_id)?.name ?? s.cwd.split("/").pop();
      const openIt = () => {
        showSession(s.id);
        void navigate({ to: "/agents", search: { session: s.id } });
      };
      // On the phone, the new agent always opens.
      if (openAfterLaunch || isPhone) { openIt(); toast("Delivered: agent launched."); }
      else toast(`Delivered: ${harnessName(s.harness)} started in ${where}`, { label: "Open", run: openIt });
    } catch (e) {
      if (!editing && !cron && !scheduled && !(e instanceof ApiError && e.operationCompleted)) setUnconfirmedLaunch(spec);
      toastError(e);
    } finally {
      setLaunching(false);
    }
  };
  // Discard keeps the draft for 10 seconds behind Undo (a long prompt is one click away from being lost).
  const discardDraft = () => {
    setCron(null);
    setLater(false);
    const undo = useComposer.getState().discardUndoable();
    toast(editing ? "Changes discarded" : "Draft discarded", { label: "Undo", run: undo }, 10_000);
  };
  const launchLabel = launching ? (cron ? "Creating…" : editing && scheduled ? "Saving…" : scheduled ? "Scheduling…" : "Sending…") : cron ? "Create routine" : editing ? (timing === "later" ? "Save" : "Start now") : timing === "later" ? "Schedule" : "Launch";
  const launchButton = () => (
    <span className="inline-flex" data-tour="composer.launch">
      <button className={`btn btn-primary ${browserOnly ? "" : "rounded-r-none"} ${isPhone ? "min-w-[120px] justify-center" : ""}`} disabled={!canLaunch || launching} title={dictation.active ? "Launch waits until dictation has finished" : unsupportedImages ? "This harness does not support attached images" : blocker ?? undefined} onClick={() => (dictation.active ? dictation.stopAndSend() : void launch())}>
        <Icon name={timing === "repeat" ? "refresh" : timing === "later" ? (editing ? "check" : "clock") : "play"} size={14} />
        {launchLabel}
      </button>
      {!browserOnly && <Popover up align="right" width={280} trigger={({ open: menu, toggle }) => <button className="btn btn-primary rounded-l-none border-l-white/30 px-1.5!" aria-label="When to start" aria-expanded={menu} disabled={launching} onClick={toggle}><Icon name="chev" size={12} /></button>}>
        {(closeMenu) => (
          <>
            <MenuItem icon={<Icon name="play" size={12} />} label={editing ? "Start now" : "Launch now"} active={timing === "now"} onClick={() => { setTiming("now"); closeMenu(); }} />
            <MenuItem icon={<Icon name="clock" size={12} />} label="Start later…" detail="After a delay, at a time, after agents are Done, when quota resets" active={timing === "later"} onClick={() => { setTiming("later"); closeMenu(); }} />
            <MenuItem icon={<Icon name="refresh" size={12} />} label="Repeat on a schedule…" detail={editing ? "Creates a routine and cancels this scheduled launch" : "Creates a routine"} active={timing === "repeat"} onClick={() => { setTiming("repeat"); closeMenu(); }} />
          </>
        )}
      </Popover>}
    </span>
  );

  // A minimized draft is the "draft" badge of the top-bar Launch button (LaunchSplitButton). Launch from the top-bar
  // field: the hidden draft launches once it can start now, or opens at once when what blocks it will not change.
  if (minimized)
    return autoLaunchUntil ? <AutoLaunch ready={canLaunch && !launching && timing === "now" && !conflict && !editing} settled={settled} blocker={blocker} launch={launch} /> : null;

  const modelLabel = spec.model ?? fallbackModel ?? "default model";
  const effortLabel = spec.effort ?? profile?.effort ?? defaults?.effort ?? null;
  const worktreeMode = spec.target.kind === "project" && !!spec.worktree && gitTarget;
  // Only values that differ from the defaults show in the option line; a new worktree is the default for Git folders.
  const runIn = spec.target.kind === "session_folder" ? (selectedWorktree ? `worktree · ${selectedWorktree.branch}` : "previous agent’s folder") : spec.target.kind === "existing_predraft" ? "existing Local PR" : worktreeMode ? (pushToPr ? `PR #${pushToPr.number}` : null) : null;
  const baseLabel = spec.worktree?.base ?? targetBranches.data?.current ?? "current branch";
  const billingText = !supportsCost && harness?.capabilities.cost ? costUnavailableMessage : payg ? `Pay as you go: Launchpad does not monitor subscription quotas for ${profile!.name}. ${spec.budget_usd != null ? `New turns stop once the reported cost reaches ${usd(spec.budget_usd)}.` : "No cost limit is set."}` : harness && !harness.capabilities.quota ? `This harness uses its native provider billing settings. Launchpad does not monitor its subscription quotas.${harness.capabilities.cost ? " A cost limit stops new turns at a reported cost." : ""}` : spec.allow_paid_usage ? "Additional credits may be used after included quota runs out." : "Pause at included quota limits and continue after reset. Best effort; some paid usage may occur before interruption.";
  // Where it runs and what happens when done, first on the options line ("New worktree from main · keep branch").
  const whenDoneText = spec.worktree?.when_done === "merge" ? `merge into ${baseLabel}` : spec.worktree?.when_done === "local_pr" ? "Local PR when done" : spec.worktree?.when_done === "read_only" ? "read only" : "keep branch";
  const whereLine = browserOnly ? null : worktreeMode ? (pushToPr ? `PR #${pushToPr.number} · pushes to the PR` : `New worktree from ${baseLabel} · ${whenDoneText}`) : spec.target.kind === "temp" ? "Temporary folder" : runIn ? runIn.charAt(0).toUpperCase() + runIn.slice(1) : null;
  const hasChips = (spec.agent_traces ?? []).length > 0 || spec.github_refs.length > 0 || (spec.skills ?? []).length > 0 || (spec.files ?? []).length > 0 || spec.context_projects.length > 0;

  return (
    <aside data-tour="composer" onDragOver={(e) => { if (e.dataTransfer.types.includes("Files")) { e.preventDefault(); e.dataTransfer.dropEffect = "copy"; } }} onDrop={(e) => { if (e.dataTransfer.types.includes("Files")) { e.preventDefault(); void addAttachments(Array.from(e.dataTransfer.files)); } }} className={`flex w-[440px] max-w-full flex-none flex-col border-l border-line bg-paper ${!embedded && isPhone ? "fixed! inset-0 z-50 w-full! pt-[env(safe-area-inset-top)] pb-[env(safe-area-inset-bottom)]" : ""} ${embedded ? "absolute inset-0 z-40 w-full!" : overlay ? "absolute top-0 right-0 bottom-0 z-40 shadow-[-18px_0_40px_-12px_rgba(23,32,43,.45)]" : "relative shadow-[-14px_0_28px_-20px_rgba(23,32,43,.35)]"}`}>
      <div className="flex items-center gap-1.5 border-b border-line px-3.5 py-2.5">
        <span className="flex-1 truncate whitespace-nowrap text-[13px] font-bold" title={edited?.title}>{editing ? "Edit scheduled launch" : "Launch an agent"}</span>
        {!embedded && !isPhone && <button className="btn btn-sm btn-ghost" title="Keep the draft as a badge on Launch; + Add buttons keep adding context" onClick={() => setMinimized(true)}>Minimize</button>}
        {isPhone && !embedded && <button className="btn btn-sm btn-ghost" onClick={discardDraft} title={editing ? "Close without saving; the scheduled agent stays as it is" : "Clear the composer"}>{editing ? "Discard changes" : "Discard"}</button>}
        <button className="iconbtn" title="Close (the draft is kept)" onClick={close}>
          <Icon name="x" size={14} />
        </button>
      </div>

      {conflict && (
        <div role="alert" className="flex flex-col gap-2 border-b border-attn-line bg-attn-soft px-3.5 py-2.5 text-[13px] text-ink">
          <span><b className="text-attn">Another window or device saved a newer draft.</b> Changes made here are not saved until you choose.</span>
          <span className="flex gap-2">
            <button className="btn btn-sm btn-primary" onClick={() => void loadNewer()}>Load the newer draft</button>
            <button className="btn btn-sm" title="Overwrite the newer draft with this one" onClick={keepMine}>Keep mine</button>
          </span>
        </div>
      )}

      {editing && <div role="status" className={`flex flex-col gap-2 border-b px-3.5 py-2.5 text-[13px] ${editGone ? "border-attn-line bg-attn-soft" : "border-line bg-chrome text-ink-2"}`}>
        {editGone
          ? <span><b className="text-attn">This scheduled agent already started or was cancelled.</b> Your changes are kept in the draft.</span>
          : <span>Changes apply to <b className="text-ink">{edited?.title ?? "the scheduled agent"}</b> when you click Save. Agents that wait for it keep waiting.</span>}
        <span className="flex gap-2">
          <button className="btn btn-sm" title="Keep this text and options as a new agent draft; the scheduled agent stays as it is" onClick={stopEditing}>Use as a new agent</button>
        </span>
      </div>}

      <div className="flex min-h-0 flex-1 flex-col gap-3 overflow-y-auto px-3.5 py-3">
        <div className="composer-prompt" data-tour="composer.prompt">
          <VoiceCaption dictation={dictation} className="px-2.5 pt-2" />
          <PromptField
            inputRef={promptInput}
            disabled={launching || readingAttachments}
            onPaste={(e) => { if (launching || attachmentRead.current) return; const files = pastedImages(e); if (files) void addAttachments(files); }}
            document={{ text: spec.text, mentions: spec.mentions, skills: spec.skills, context_projects: spec.context_projects, github_refs: spec.github_refs, agent_traces: spec.agent_traces }}
            onDocumentChange={(doc) => set((s) => ({ ...s, ...doc }))}
            root={source?.cwd ?? target?.path ?? null}
            harness={spec.harness}
            profile={spec.profile}
            repo={repo}
            projectId={target?.id}
            relativePaths={!spec.worktree}
            noFolders={browserOnly}

          />
          <input ref={attachmentInput} type="file" multiple hidden onChange={(e) => { void addAttachments(Array.from(e.target.files ?? [])); e.target.value = ""; }} />
          {/* The attach row sits under the text, so no line passes under the button. */}
          <div className="flex items-center justify-between gap-1 border-t border-line-2 px-1 py-1">
            <AddContextMenu
              compact
              disabled={launching}
              projects={(browserOnly ? [] : sorted).filter((p) => p.id !== target?.id && !spec.context_projects.some((c) => c.project_id === p.id))}
              recent={recentContexts}
              sessions={recentSessions}
              selectedTraces={spec.agent_traces ?? []}
              onProject={addProjectContext}
              onGitHub={(r) => set((s) => (s.github_refs.some((g) => g.url === r.url) ? s : { ...s, github_refs: [...s.github_refs, r] }))}
              onToggleTrace={(trace) => set((s) => {
                const traces = s.agent_traces ?? [];
                return { ...s, agent_traces: traces.some((t) => t.session_id === trace.session_id) ? traces.filter((t) => t.session_id !== trace.session_id) : [...traces, trace] };
              })}
              presets={{ presets: agentPresets, selected: spec.presets, onChange: (ids) => set((s) => ({ ...s, presets: ids })) }}
            />
            <button className={`${isPhone ? "btn btn-ghost text-[13px]" : "btn btn-sm btn-ghost"} ml-auto`} disabled={launching || readingAttachments} aria-label="Attach files or images" title={`Attach files or images${isPhone ? "" : "; you can also drop files or paste a screenshot"}. ${ATTACH_LIMITS}`} onClick={() => attachmentInput.current?.click()}><Icon name="clip" size={14} />{isPhone ? "Attach files or images" : "Attach"}</button>
            <VoiceButton dictation={dictation} />
          </div>
        </div>
        {(readingAttachments || (spec.attachments ?? []).length > 0 || unsupportedImages) && <div>
          {readingAttachments && <div className="text-[12px] text-ink-3">Reading attachments…</div>}
          <AttachmentList files={spec.attachments ?? []} disabled={launching || readingAttachments} onRemove={(i) => set((s) => ({ ...s, attachments: s.attachments.filter((_, j) => j !== i) }))} />
          {unsupportedImages && <div className="mt-1 text-[12px] text-danger-ink">This harness does not support attached images. Remove them or choose another harness.</div>}
        </div>}

        {browserOnly ? <div className="rounded-card border border-line bg-chrome px-3 py-2 text-[12px]">Browser · explicit page grants · own folder, no worktree<div className="mt-1 text-ink-3">{harness?.browser_isolation}</div></div> : <div className="grid gap-2.5">
          <Field label="Folder" tour="composer.folder">
            <Popover
              width={300}
              trigger={({ toggle }) => (
                <SelectButton onClick={toggle}>
                  <Icon name="folder" size={14} />
                  {spec.target.kind === "temp" ? "Temporary folder" : (target?.name ?? <span className="text-ink-3">Pick a folder</span>)}
                </SelectButton>
              )}
            >
              {(closeMenu) => (
                <ProjectPicker
                  projects={sorted}
                  selected={target?.id}
                  temporary={spec.target.kind === "temp"}
                  allowTemporary
                  onPick={(p) => {
                    set((s) => p ? withFolder(s, p) : ({ ...s, target: { kind: "temp" } }));
                    closeMenu();
                  }}
                />
              )}
            </Popover>
            {recentFolders.filter((p) => p.id !== target?.id).length > 0 && <div className="mt-1.5 flex flex-wrap gap-1">{recentFolders.filter((p) => p.id !== target?.id).slice(0, 3).map((p) => <button key={p.id} className="lp-chip h-[22px]! px-2! text-[11px]!" title={`Run in ${p.name}`} onClick={() => set((s) => withFolder(s, p))}>{p.name}</button>)}</div>}
          </Field>
        </div>}


        {hasChips && <div className="flex flex-wrap items-center gap-1.5">
          {(spec.agent_traces ?? []).map((trace) => (
            <span key={`${trace.session_id}:${trace.through_seq}`} className="inline-flex h-7 max-w-full items-center gap-1.5 rounded-control border border-line bg-chrome px-2 text-[12px]">
              <Icon name="agents" size={12} />
              <span className="min-w-0 truncate" title={`Stored trace through event #${trace.through_seq} · ${trace.session_id}`}>Trace · {sessions?.find((s) => s.id === trace.session_id)?.title ?? "Missing agent"}</span>
              <button title="Remove agent trace" className="text-ink-3 hover:text-ink" onClick={() => set((s) => ({ ...s, agent_traces: s.agent_traces.filter((t) => t.session_id !== trace.session_id || t.through_seq !== trace.through_seq) }))}><Icon name="x" size={12} /></button>
            </span>
          ))}
          {spec.github_refs.map((g) => {
            const ic = itemIcon(g.kind === "pr" ? "pr" : "issue", "open");
            return (
              <span key={g.url} className="inline-flex h-7 max-w-full items-center gap-1.5 rounded-control border border-line bg-chrome px-2 text-[12px]" title={g.title ?? g.url}>
                <Icon name={g.kind === "comment" || g.kind === "review_comment" ? "comment" : ic.name} size={12} className={ic.color} />
                <span className="font-mono">{g.number === null ? g.kind : `#${g.number}`}</span>
                <span className="min-w-0 truncate text-ink-2">{g.title ?? g.url}</span>
                <button className="text-ink-3 hover:text-ink" title="Remove" onClick={() => set((s) => ({ ...s, github_refs: s.github_refs.filter((x) => x.url !== g.url) }))}><Icon name="x" size={12} /></button>
              </span>
            );
          })}
          <SkillChips skills={spec.skills} disabled={launching} onRemove={(path) => set((s) => ({ ...s, skills: (s.skills ?? []).filter((v) => v.path !== path) }))} />
          {(spec.files ?? []).map((f, i) => (
            <span key={`${f.root}\n${f.rev}\n${f.path}\n${f.start_line}-${f.end_line}`} className="inline-flex h-7 max-w-full items-center gap-1.5 rounded-control border border-line bg-chrome px-2 text-[12px]" title={`${f.root}/${f.path}${f.rev ? ` at ${f.rev}` : ""}`}>
              <Icon name="file" size={12} />
              <span className="min-w-0 truncate font-mono">
                {f.path}
                {f.start_line !== null && `:${f.start_line}${f.end_line !== null && f.end_line > f.start_line ? `-${f.end_line}` : ""}`}
              </span>
              <button className="text-ink-3 hover:text-ink" title="Remove" onClick={() => set((s) => ({ ...s, files: s.files.filter((_, j) => j !== i) }))}><Icon name="x" size={12} /></button>
            </span>
          ))}
          {spec.context_projects.map((c) => (
            <span key={c.project_id} className="inline-flex h-7 max-w-full items-center gap-1.5 rounded-control border border-line bg-chrome px-2 text-[12px]">
              <Icon name="folder" size={12} />
              <span className="font-mono">{projects.find((x) => x.id === c.project_id)?.name ?? "?"}</span>
              <Branch projectId={c.project_id} branch={c.branch} onChange={(branch) => set((s) => ({ ...s, context_projects: s.context_projects.map((x) => (x.project_id === c.project_id ? { ...x, branch } : x)) }))} />
              <button
                className={`inline-flex h-5 items-center rounded-control border px-1.5 text-[11px] ${c.editable ? "border-teal bg-teal-soft font-semibold text-teal-ink" : "border-line bg-paper text-ink-2"}`}
                title={c.editable ? (spec.target.kind === "project" && !worktreeMode ? "Editable: the agent may modify this folder. Click to make it read-only." : "Editable: the agent gets its own worktree of this project. Click to make it read-only.") : "Read-only reference. Click to let the agent modify it."}
                aria-pressed={c.editable}
                onClick={() => set((s) => ({ ...s, context_projects: s.context_projects.map((x) => (x.project_id === c.project_id ? { ...x, editable: !x.editable } : x)) }))}
              >{c.editable ? "editable" : "read-only"}</button>
              <button className="text-ink-3 hover:text-ink" title="Remove" onClick={() => set((s) => ({ ...s, context_projects: s.context_projects.filter((x) => x.project_id !== c.project_id) }))}><Icon name="x" size={12} /></button>
            </span>
          ))}
        </div>}

        <PresetChips selected={spec.presets} onChange={(ids) => set((s) => ({ ...s, presets: ids }))} disabled={launching} />

        {missingProfile && <div className="rounded-card bg-attn-soft px-3 py-2 text-[12px] text-attn">The profile {spec.profile} no longer exists. Pick another profile in the options.</div>}
        <OutdatedRunnerNotice spec={spec} />
        {!profiles.length && <div className="rounded-card bg-attn-soft px-3 py-2 text-[12px] text-attn">No profile yet. A profile says which harness runs the agent and what pays for it. <Link to="/settings/profiles" className="font-semibold underline">Add one in Settings → Profiles</Link>.</div>}

        <button
          data-tour="composer.options"
          className={`flex w-full flex-wrap items-center gap-1 rounded-card border px-2 py-1.5 text-left ${showOptions ? "border-line bg-chrome" : "border-line bg-paper hover:border-line"}`}
          aria-expanded={showOptions}
          title={showOptions ? "Hide the options" : "Show the options"}
          onClick={() => setShowOptions((v) => !v)}
        >
          {whereLine && <span className="min-w-0 truncate text-[12px] text-ink-2" data-tour="composer.runin">{whereLine}</span>}
          <Token warn={!profile}>{spec.harness && <HarnessMark harness={spec.harness} size={13} />}{profile?.name ?? "No profile"}</Token>
          {spec.model && spec.model !== fallbackModel && <Token>{modelLabel}</Token>}
          {!!efforts.length && spec.effort && spec.effort !== (profile?.effort ?? defaults?.effort) && <Token>{effortLabel}</Token>}
          {harness?.capabilities.permission_modes && spec.permission_mode && spec.permission_mode !== "yolo" && <Token>{spec.permission_mode === "auto" ? "Auto approval" : "Ask for approval"}</Token>}
          {spec.runner_id && <Token warn={launchRunners.some((r) => r.runner.id === spec.runner_id && r.compat === "outdated")} title="The runner this launch goes to">on {runnerName(launchRunners, spec.runner_id)}</Token>}
          {toolsOff.length > 0 && <Token title={`Off for this launch: ${toolsOff.map((t) => t.name).join(", ")}`}>{toolsOff.length} tool{toolsOff.length === 1 ? "" : "s"} off</Token>}
          {scheduled && <Token warn title="Starts when all start conditions are met">start: {conditions.length} condition{conditions.length === 1 ? "" : "s"}</Token>}
          {cron && <Token warn title={`Cron: ${cron.schedule}`}>routine · {describeCron(cron.schedule)}</Token>}
          {(payg || spec.allow_paid_usage || (harness && !harness.capabilities.quota)) && <Token warn={payg || spec.allow_paid_usage} title={billingText}>{payg ? "pay as you go" : spec.allow_paid_usage ? "paid usage on" : "provider billing"}</Token>}
          {!supportsCost && harness?.capabilities.cost && <Token warn title={costUnavailableMessage}>cost unavailable</Token>}
          {supportsCost && (spec.budget_usd != null || !budgetValid) && <Token warn>{spec.budget_usd != null ? `limit ${usd(spec.budget_usd)}` : "invalid cost limit"}</Token>}
          <span className="ml-auto flex items-center gap-0.5 pl-1 text-[12px] text-ink-3">Options<Icon name="chev" size={12} className={showOptions ? "rotate-180" : ""} /></span>
        </button>

        {showOptions && <OptionsFrame onClose={() => setShowOptions(false)}>
          {!browserOnly && <OptionRow label="Run in">
            <div className="grid min-w-0 grid-cols-1 gap-1.5">
              {target && gitTarget ? <RunIn
                projectId={target.id}
                label={pushToPr ? `PR #${pushToPr.number}` : selectedWorktree ? selectedWorktree.branch : existing ? `Local PR: ${existing.title}` : spec.target.kind === "session_folder" ? "Previous agent’s folder" : `New worktree from ${spec.worktree?.base ?? `default branch${targetBranches.data?.current ? ` (${targetBranches.data.current})` : ""}`}`}
                base={worktreeMode ? (spec.worktree?.base ?? null) : undefined}
                worktrees={worktreeChoices}
                selectedWorktree={selectedWorktree?.id}
                localPrs={existingPrChoices}
                selectedLocalPr={existing?.id}
                onBranch={(base) => set((s) => ({ ...s, target: { kind: "project", project_id: target.id }, worktree: { ...(s.worktree ?? { when_done: target.when_done }), base } }))}
                onWorktree={(w) => set((s) => ({ ...s, target: { kind: "session_folder", session_id: w.source.id }, worktree: null }))}
                onLocalPr={(p) => set((s) => ({ ...s, target: { kind: "existing_predraft", predraft_id: p.id }, worktree: null }))}
              /> : target ? <select className="field-input w-full" aria-label="Run in" value={spec.target.kind} onChange={(e) => {
                if (e.target.value === "project") set((s) => ({ ...s, target: { kind: "project", project_id: target.id }, worktree: null }));
              }}>
                {spec.target.kind === "session_folder" && <option value="session_folder">Previous agent’s folder</option>}
                <option value="project">{target.plain ? "Folder (no Git)" : "Temporary folder"}</option>
              </select> : <div className="field-input">{spec.target.kind === "session_folder" ? source?.cwd ?? "Source session unavailable" : spec.target.kind === "temp" ? "Deleted at Archive" : "—"}</div>}
              {taskWorktrees.isFetching && !taskWorktrees.data && target && <div className="mt-1 text-[12px] text-ink-3">Loading existing worktrees…</div>}
              {taskWorktrees.isError && target && <div className="mt-1 text-[12px] text-danger-ink">Could not load existing worktrees. <button className="underline" onClick={() => void taskWorktrees.refetch()}>Retry</button></div>}
              {existing && <div className="mt-1 break-all text-[12px] text-ink-3">Branch {existing.branch} on Home</div>}
              {spec.target.kind === "session_folder" && <div className="mt-1 break-all text-[12px] text-ink-3">{source?.cwd ?? "Source session unavailable"}</div>}
              {target && spec.target.kind === "project" && !worktreeMode && <div className="mt-1 text-[12px] text-ink-3">{gitTarget ? <TargetCheckout projectId={target.id} /> : "Runs in the folder"}</div>}
              {worktreeMode && <div className="mt-1 text-[12px] text-ink-3" title="Before the agent starts, the runner clones the folder from Home Git into a fresh worktree on a new branch. The branch goes back to Home at the end of every turn.">{pushToPr ? `A fresh clone of PR #${pushToPr.number} from Home Git` : "A fresh clone from Home Git"}</div>}
              {worktreeMode && detectedPr && <label className="inline-flex items-center gap-1.5 text-[12px] text-ink-2" title="The worktree starts from the pull request's head and Home pushes the agent's commits to the PR branch after each turn (never force). Uncheck to keep the work on Home.">
                <input type="checkbox" className="accent-teal" checked={!!pushToPr} onChange={(e) => setPrOptOut(e.target.checked ? null : `${detectedPr.repo}#${detectedPr.number}`)} />
                Push to PR #{detectedPr.number}
              </label>}
            </div>
          </OptionRow>}
          {worktreeMode && target && !pushToPr && <OptionRow label="When done">
            <span className="grid min-w-0 grid-cols-1" title="Merge: the agent asks Launchpad to merge when it is done; Home fast-forwards the base branch, and a base that moved goes back to the agent for a rebase. Open a Local PR: the agent opens a Local PR of its branch on Home, with a title and description for you to review and publish. Read only: for questions and audits; the agent changes nothing, and Archive deletes the unchanged worktree and branch.">
              <select className="field-input min-w-0 w-full" aria-label="When done" value={spec.worktree?.when_done ?? "keep"} disabled={completionDefault.isPending} title="Remembered for new agents in this project, except Read only" onChange={(e) => {
                const whenDone = e.target.value as WhenDone;
                set((s) => ({ ...s, worktree: { ...s.worktree!, when_done: whenDone } }));
                if (whenDone !== "read_only") completionDefault.mutate({ projectId: target.id, whenDone });
              }}>
                <option value="keep">Keep branch for review</option>
                <option value="local_pr">Open a Local PR</option>
                <option value="merge">Merge into {baseLabel}</option>
                <option value="read_only">Read only (questions, audits)</option>
              </select>
            </span>
          </OptionRow>}
          <OptionRow label="Profile">
            <span className="grid gap-1">
              <Choices label="Profile">
                {profiles.map((o) => (
                  <Choice key={o.id} active={spec.profile === o.id} title={`${harnessName(o.harness)} · ${o.billing === "pay_as_you_go" ? "pay as you go" : "subscription"}`} onClick={() => { setBudgetText(null); set((s) => withProfile(s, o)); }}>
                    <HarnessMark harness={o.harness} size={15} />
                    {o.name}
                  </Choice>
                ))}
              </Choices>
              <Link to="/settings/profiles" className="w-fit text-[12px] text-teal-ink hover:underline">Manage profiles</Link>
            </span>
          </OptionRow>
          <OptionRow label="Runner"><RunnerChoice spec={spec} set={set} /></OptionRow>
          {launchTools.items.length > 0 && <OptionRow label="Tools"><ToolsChoice spec={spec} set={set} workspaceId={toolsWorkspace} folderId={targetId} /></OptionRow>}
          {harness?.capabilities.permission_modes && <OptionRow label="Permissions">
            <div className="grid gap-1.5">
              <select className="field-input w-auto" aria-label="Permission mode" value={spec.permission_mode ?? "yolo"} onChange={(e) => set((s) => ({ ...s, permission_mode: e.target.value as LaunchSpec["permission_mode"] }))}>
                <option value="yolo">YOLO</option>
                <option value="approvals">Ask for approval</option>
                <option value="auto">Auto approval</option>
              </select>
              <span className="text-[12px] text-ink-3">{spec.permission_mode === "auto" ? "A separate AI reviews actions that need permission. Human-only requests still appear in the session. Requires native CLI, model and account support." : spec.permission_mode === "approvals" ? "Uses native permission rules. Requested commands and changes wait for your approval in the session." : "Commands and file changes run without permission prompts."}</span>
            </div>
          </OptionRow>}
          <OptionRow label="Model" end={
            <button className="iconbtn disabled:opacity-50" disabled={!spec.harness || refreshingModels} onClick={refreshModelList} title={refreshingModels ? "Refreshing models…" : "Refresh models"} aria-label={refreshingModels ? "Refreshing models" : "Refresh models"} aria-busy={refreshingModels}>
              <Icon name="refresh" size={12} className={refreshingModels ? "animate-spin" : ""} />
            </button>
          }>
            <Popover
              width={300}
              trigger={({ toggle }) => (
                <SelectButton onClick={toggle}>
                  {spec.model ?? (
                    <>
                      Default <span className="text-ink-3">· {fallbackModelLabel}</span>
                    </>
                  )}
                </SelectButton>
              )}
            >
              {(closeMenu) => (
                <>
                  <MenuItem icon={<Icon name="refresh" size={12} className={refreshingModels ? "animate-spin" : ""} />} label={refreshingModels ? "Refreshing models…" : "Refresh models"} detail="Query the selected harness again" disabled={!spec.harness || refreshingModels} onClick={refreshModelList} />
                  {(!ownModels || fallbackModel) && <MenuItem label={`Default (${fallbackModelLabel})`} detail={profile?.model ? `Default of ${profile.name}` : "What the agent picks on its own"} active={!spec.model} onClick={() => (set((s) => ({ ...s, model: null })), closeMenu())} />}
                  {ownModels ? <ModelSearch models={providerModels.data ?? []} loading={providerModels.isLoading} error={providerModels.error ? String(providerModels.error.message ?? providerModels.error) : null} active={spec.model} onPick={(id) => (set((s) => ({ ...s, model: pickModel(providerModels.data, s.model ?? fallbackModel, id) })), closeMenu())} /> : <>
                  {!defaults?.models.length && <div className="px-2 py-1 text-[12px] text-ink-3">No models loaded. Click Refresh models to retry.</div>}
                  {(defaults?.models ?? []).map((m) => (
                    <MenuItem key={m.id} label={m.display_name || m.id} detail={`${m.id} · ${m.description}`} active={spec.model === m.id} onClick={() => (set((s) => ({ ...s, model: m.id })), closeMenu())} />
                  ))}
                  </>}
                </>
              )}
            </Popover>
          </OptionRow>
          {ownModels && !!findModel(providerModels.data, spec.model ?? fallbackModel)?.providers.length && <OptionRow label="Provider">
            <ProviderPicker compact models={providerModels.data} model={spec.model ?? fallbackModel} onChange={(id) => set((s) => ({ ...s, model: id }))} />
          </OptionRow>}
          {!!efforts.length && <OptionRow label="Effort">
            <Popover
              width={220}
              trigger={({ toggle }) => (
                <SelectButton onClick={toggle}>
                  {spec.effort ?? (
                    <>
                      Default <span className="text-ink-3">· {profile?.effort ?? defaults?.effort ?? "harness default"}</span>
                    </>
                  )}
                </SelectButton>
              )}
            >
              {(closeMenu) => (
                <>
                  <MenuItem label={`Default (${profile?.effort ?? defaults?.effort ?? "harness default"})`} active={!spec.effort} onClick={() => (set((s) => ({ ...s, effort: null })), closeMenu())} />
                  {efforts.map((e) => (
                    <MenuItem key={e} label={e} active={spec.effort === e} onClick={() => (set((s) => ({ ...s, effort: e })), closeMenu())} />
                  ))}
                </>
              )}
            </Popover>
          </OptionRow>}
          {!supportsCost && harness?.capabilities.cost && <OptionRow label="Cost management"><span className="text-[12px] text-attn">{costUnavailableMessage}</span></OptionRow>}
          {supportsCost && <OptionRow label="Cost limit">
            <span className="grid gap-1">
              <input className="field-input w-28" aria-label="Cost limit in USD" placeholder="No limit" value={budgetInput} onChange={(e) => { setBudgetText(e.target.value); const v = parseUsd(e.target.value); if (v !== undefined) set((s) => ({ ...s, budget_usd: v })); }} />
              <span className={`text-[11px] ${budgetValid ? "text-ink-3" : "text-attn"}`}>{budgetValid ? "USD of reported cost. At the limit, the running turn stops and no new turn starts. Empty: no limit." : "Enter a positive amount in USD, or leave empty."}</span>
            </span>
          </OptionRow>}
          {harness?.capabilities.quota && !payg && <OptionRow label="Paid usage">
            <label className="inline-flex items-center gap-2 pt-1.5 text-[12px] text-ink-2" title={billingText}>
              <input type="checkbox" className="accent-teal" checked={spec.allow_paid_usage} onChange={(e) => set((s) => ({ ...s, allow_paid_usage: e.target.checked }))} />
              Allow paid usage after included quota runs out
            </label>
          </OptionRow>}
        </OptionsFrame>}

        {timing !== "now" && !browserOnly && <div className="grid gap-2 rounded-card border border-attn-line bg-attn-soft/40 p-2.5">
          <div className="flex items-center gap-2 text-[12px] font-semibold text-attn"><Icon name={timing === "repeat" ? "refresh" : "clock"} size={12} />{timing === "repeat" ? "Repeat on a schedule: creates a routine" : "Start later"}<button className="btn btn-sm btn-ghost ml-auto" onClick={() => setTiming("now")}>{editing ? "Start now instead" : "Launch now instead"}</button></div>
          {timing === "repeat" ? <CronSelector cron={cron} harness={spec.harness} onChange={(c) => { setCron(c); if (c) setConditions([]); else setTiming("now"); }} onValidityChange={setCronValid} /> : <DelaySelector conditions={conditions} harness={spec.harness} profile={spec.profile} exclude={editing} onChange={setConditions} onValidityChange={setDelayValid} />}
        </div>}

        <div className="flex flex-wrap items-center gap-2">
          <Popover width={400} trigger={({ toggle }) => <button className="btn btn-sm w-fit" onClick={toggle}><Icon name="eye" size={12} />Preview prompt</button>}>
            {() => <PromptPreview spec={spec} title={`Prompt sent to ${spec.harness ? harnessName(spec.harness) : "the agent"}`} />}
          </Popover>
          <button className="btn btn-sm btn-ghost w-fit" data-tour="composer.start-preview" title="Its instructions (knowledge base, Launchpad CLI) and first prompt, as Launchpad builds them at start" disabled={!spec.harness} onClick={() => setStartPreview(spec)}>What it sees at start</button>
        </div>
        {startPreview && <StartPreviewDialog queryKey={["launch", startPreview]} load={() => api.post<StartPreview>("/api/launch/start-preview", startPreview)} subject="this agent" onClose={() => setStartPreview(null)} />}
      </div>

      {/* Phone: one footer row, Launch at full width (Discard sits next to ✕, the reason Launch waits in one line). */}
      {isPhone && !embedded ? <div className="composer-phone-footer grid gap-1.5 border-t border-line bg-chrome px-3 py-2">
        {unconfirmedLaunch && <div role="status" className="text-[12px] text-ink-3">Delivery not confirmed. Retry unchanged to check the result. Inspect Agents before starting another attempt.<button className="btn btn-sm mt-1" disabled={launching} onClick={() => void abandonOperation("/api/sessions", unconfirmedLaunch).then(() => setUnconfirmedLaunch(null)).catch(toastError)}>I checked Agents — start a new attempt</button></div>}
        {(blocker || unsupportedImages) && !launching && <span role="status" className="truncate text-[12px] text-attn">{unsupportedImages ? "Remove the images or pick another harness" : blocker}</span>}
        <div className="flex [&>span]:flex-1 [&>span>button:first-child]:flex-1">{launchButton()}</div>
      </div> : <div className="flex flex-wrap items-center gap-2 border-t border-line bg-chrome px-3.5 py-2.5">
        <label className="mr-auto inline-flex items-center gap-2 text-[12px] text-ink-2" style={scheduled || cron || isPhone ? { visibility: "hidden" } : undefined}>
          <input type="checkbox" className="accent-teal" disabled={scheduled || !!cron} checked={openAfterLaunch} onChange={(e) => setOpenAfterLaunch(e.target.checked)} />
          Open after launch
        </label>
        <button className="btn btn-ghost" onClick={discardDraft} title={editing ? "Close without saving; the scheduled agent stays as it is" : "Clear the composer"}>
          {editing ? "Discard changes" : "Discard"}
        </button>
        {unconfirmedLaunch && <div role="status" className="text-[12px] text-ink-3">Delivery not confirmed. Retry unchanged to check the result. Inspect Agents before starting another attempt.<button className="btn btn-sm mt-1" disabled={launching} onClick={() => void abandonOperation("/api/sessions", unconfirmedLaunch).then(() => setUnconfirmedLaunch(null)).catch(toastError)}>I checked Agents — start a new attempt</button></div>}
        <span className="text-[12px] text-ink-3" title="Where this launch goes: the chosen runner, or Automatic">Runs on {spec.runner_id ? runnerName(launchRunners, spec.runner_id) : "a runner, else Burst"}</span>
        {(blocker || unsupportedImages) && !launching && <span role="status" className="basis-full text-right text-[12px] text-attn sm:basis-auto">{unsupportedImages ? "Remove the images or pick another harness" : blocker}</span>}
        {launchButton()}
      </div>}
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
      Checkout{q.data?.current && <span className="font-mono text-[12px]">· {q.data.current}</span>}
    </>
  );
}

/** The top-bar Launch of a hidden draft: launches as soon as it can, or opens the composer with what is missing once
 * everything it depends on has loaded (a short grace lets the folder and profile defaults apply first). The field's
 * 4-second fallback still opens it if neither happens. */
function AutoLaunch({ ready, settled, blocker, launch }: { ready: boolean; settled: boolean; blocker: string | null; launch: () => Promise<void> }) {
  const run = useRef(launch);
  run.current = launch;
  useEffect(() => {
    if (ready) {
      if (useComposer.getState().autoLaunchUntil <= Date.now()) return;
      useComposer.setState({ autoLaunchUntil: 0 });
      void run.current();
      return;
    }
    if (!settled) return;
    const t = setTimeout(() => {
      if (!useComposer.getState().autoLaunchUntil) return;
      useComposer.setState({ autoLaunchUntil: 0 });
      useComposer.getState().setMinimized(false);
      toast(blocker ? `Finish the launch options: ${blocker}` : "Finish the launch options, then Launch.");
    }, 400);
    return () => clearTimeout(t);
  }, [ready, settled, blocker]);
  return null;
}
