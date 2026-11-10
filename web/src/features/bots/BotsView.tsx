// Bots: persistent agents with one thread each. The list (Ask Launchpad is one of them, created once per
// workspace), the form that creates one (name, job, profile and folder only), and a bot's page: its chat (the
// whole thread, features/chat/BotChat.tsx) is the page; About (a drawer on the laptop, a page opened from ⓘ on the
// phone) holds what defines the bot: BOT.md, Triggers, Memory, Access and Versions; ⋯ holds Pause all triggers,
// Start a new session, Open in Agents, Edit as file and Delete. Creating saves; the first task is a separate Send.

import { useQuery, useQueryClient } from "@tanstack/react-query";
import { ToolDefaultsEditor } from "@/features/settings/ToolDefaults";
import { defaultOn, useLayers } from "@/lib/toolDefaults";
import { Link, useNavigate, useParams, useRouter, useSearch } from "@tanstack/react-router";
import { memo, useEffect, useLayoutEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { Icon, type IconName } from "@/components/Icon";
import { MenuItem } from "@/components/Popover";
import { Markdown } from "@/components/Markdown";
import { toast, toastError } from "@/components/Toast";
import { ConfirmButton, Empty, ErrorState, IconButton, Loading, MoreMenu, PageHeader, StatusBadge, Tabs, type StatusTone } from "@/components/ui";
import { PatchView } from "@/features/github/DiffReview";
import { BotChat, type NewSessionRequest } from "@/features/chat/BotChat";
import type { ChannelsOverview } from "@/features/settings/ChannelsView";
import { useTelegram } from "@/features/settings/TelegramSettings";
import { BotAvatar } from "@/components/BotAvatar";
import { SplitView } from "@/components/SplitView";
import type { Bot } from "@/gen/Bot";
import type { BotSettings } from "@/gen/BotSettings";
import type { Change } from "@/gen/Change";
import type { PermissionMode } from "@/gen/PermissionMode";
import type { Project } from "@/gen/Project";
import type { Session } from "@/gen/Session";
import type { VaultOverview } from "@/gen/VaultOverview";
import { api } from "@/lib/api";
import { runAction } from "@/lib/actions";
import { ActionButton, ActionMenuItem } from "@/components/ActionButton";
import { askTarget, botJob, botProfile, botState, botThread, currentSession, instructionsUpdated, rotationReason, useBots, useBotTemplates, type BotState } from "@/lib/bots";
import { isQuotaPaused, isRunning } from "@/lib/sessionStatus";
import { daemonUrl } from "@/lib/connection";
import { age, ago, when } from "@/lib/format";
import { usePhone } from "@/lib/phone";
import { useProfiles } from "@/lib/profiles";
import { useRunnerTag } from "@/lib/burst";
import { useCrons, useHarnessDefaults, useProjects, useSessions } from "@/lib/queries";
import { cronTitle, routineState, triggerSummary } from "@/lib/routines";
import { NewRoutine, RoutineDetails, RunStrip } from "@/features/schedules/Routines";
import type { CronAgent } from "@/gen/CronAgent";
import type { ProposalMode } from "@/gen/ProposalMode";
import type { LaunchpadAccess } from "@/gen/LaunchpadAccess";

const CLI_ACCESS: { id: LaunchpadAccess; label: string; detail: string }[] = [
  { id: "read", label: "Read", detail: "reads Launchpad state: folders, agents, Inbox, schedules, quotas" },
  { id: "propose", label: "Propose", detail: "reads, and writes proposals that wait for your click" },
  { id: "admin", label: "Admin", detail: "also changes Launchpad itself: bots, routines, schedules, workspaces, launches and messages; never GitHub, the Outbox, credentials, the machine or paid usage" },
];
import { useScope } from "@/lib/workspace";
import { PhoneBar, PhoneBarEnd, PhoneHeader } from "@/mobile/PhoneHeader";
import { Sheet } from "@/mobile/Sheet";
import { StartPreviewDialog } from "@/features/launch/StartPreview";
import type { StartPreview } from "@/gen/StartPreview";

const PERMISSIONS: { id: PermissionMode; label: string; detail: string }[] = [
  { id: "yolo", label: "YOLO", detail: "Runs commands without asking, inside its jail" },
  { id: "approvals", label: "Ask for approval", detail: "Asks you before commands and edits" },
  { id: "auto", label: "Auto", detail: "The harness's own reviewer decides each request" },
];

const PROPOSAL_MODES: { id: ProposalMode; label: string; detail: string }[] = [
  { id: "manual", label: "Manual", detail: "every proposal waits for your click" },
  { id: "auto", label: "Auto", detail: "one-time launches, messages and schedules start at once; routines, presets, bots, tools, BOT.md changes and ready clicks still wait" },
];

const STATE: Record<BotState, { label: string; tone: StatusTone }> = {
  needs: { label: "Needs you", tone: "needs" },
  running: { label: "Working", tone: "running" },
  paused: { label: "Quota paused", tone: "needs" },
  idle: { label: "Idle", tone: "idle" },
  new: { label: "No task yet", tone: "idle" },
};


function StateBadge({ state }: { state: BotState }) {
  const s = STATE[state];
  return <StatusBadge tone={s.tone} label={s.label} />;
}

const useBotSessions = () => useSessions().data ?? [];
const folderLabel = (projects: Project[], id: string | null | undefined) => id ? projects.find((p) => p.id === id)?.name ?? "a removed folder" : "a temporary folder";

// ---------------------------------------------------------------- the list

export function BotsView() {
  const phone = usePhone();
  const bots = useBots();
  const scope = useScope();
  const sessions = useBotSessions();
  const projects = useProjects().data ?? [];
  const search = useSearch({ strict: false }) as { new?: boolean };
  const navigate = useNavigate();
  const mine = (bots.data ?? []).filter((b) => b.workspace_id === scope.workspace.id);
  const creating = !!search.new || (bots.isSuccess && mine.length === 0);
  const close = () => void navigate({ to: "/bots", search: {} as never });
  const header = phone
    ? <PhoneHeader title="Bots" primary={!creating ? <Link to="/bots" search={{ new: true } as never} className="btn btn-sm"><Icon name="plus" size={12} />New bot</Link> : undefined} />
    : <PageHeader title="Bots" meta={mine.length ? `${mine.length} bot${mine.length === 1 ? "" : "s"}` : undefined} intro="Persistent agents with one thread, memory in the knowledge base and files on Home." actions={!creating && <Link to="/bots" search={{ new: true } as never} className="btn btn-sm btn-primary" data-tour="bots.new"><Icon name="plus" size={12} />New bot</Link>} />;
  return (
    <div className="flex h-full min-h-0 flex-col" data-tour="bots.page">
      {header}
      <div className="min-h-0 flex-1 overflow-y-auto">
        <div className="grid w-full max-w-page grid-cols-[minmax(0,1fr)] gap-4 px-5 py-4">
          {bots.isLoading ? <Loading what="bots" /> : bots.error ? <ErrorState error={bots.error} onRetry={() => void bots.refetch()} /> : <>
            {creating && <NewBot first={mine.length === 0} onCancel={mine.length ? close : undefined} />}
            <div className="grid grid-cols-[minmax(0,1fr)] gap-2">
              {mine.map((b) => <BotRow key={b.id} bot={b} thread={botThread(sessions, b.id)} folder={folderLabel(projects, b.settings.folder)} />)}
            </div>
          </>}
        </div>
      </div>
    </div>
  );
}

const BotRow = memo(function BotRow({ bot, thread, folder }: { bot: Bot; thread: Session[]; folder: string }) {
  const last = thread.at(-1);
  return <Link to="/bots/$id" params={{ id: bot.id }} className="flex min-h-[56px] items-center gap-3 rounded-card border border-line bg-paper px-3 py-2 no-underline hover:bg-sunk">
    <BotAvatar name={bot.settings.name} size={32} />
    <span className="min-w-0 flex-1">
      <span className="flex items-center gap-2 text-[13px] font-semibold text-ink"><span className="truncate">{bot.settings.name}</span><StateBadge state={botState(thread)} /></span>
      <span className="block truncate text-[12px] text-ink-3">{botJob(bot) || "No instructions yet"}</span>
    </span>
    <span className="hidden flex-none text-right text-[12px] text-ink-3 sm:block">
      <span className="block">{folder}</span>
      <span className="block">{last ? ago(last.status_since) : "never ran"}</span>
    </span>
  </Link>;
});

// ---------------------------------------------------------------- creating a bot

/** Tools granted to a workspace (Home's registry): what a bot's sessions reach with Home's tokens. */
function useGrantedTools(workspaceId: string | null) {
  const vault = useQuery({ queryKey: ["vault"], queryFn: () => api.get<VaultOverview>("/api/vault"), staleTime: 30_000, retry: false });
  return (vault.data?.tools ?? []).filter((t) => !!workspaceId && (t.workspaces.includes("*") || t.workspaces.includes(workspaceId)));
}

/** What the bot can reach, in plain words: its folder, the knowledge base, the granted tools and the mode. */
function Access({ folder, workspace, workspaceId, mode, proposals = "manual", launchpad = "propose" }: { folder: Project | null; workspace: string; workspaceId: string | null; mode: PermissionMode; proposals?: ProposalMode; launchpad?: LaunchpadAccess }) {
  const tools = useGrantedTools(workspaceId);
  const permission = PERMISSIONS.find((p) => p.id === mode)!;
  const cli = tools.some((t) => t.id === "launchpad");
  return <ul className="m-0 grid gap-1 pl-4 text-[12px] text-ink-2">
    <li>Runs contained on a runner, like every agent. <b>{permission.label}</b>: {permission.detail.toLowerCase()}.</li>
    <li>{folder ? (folder.plain ? <>Works in the plain folder <b>{folder.name}</b>, synced by Home.</> : <>Works in <b>{folder.name}</b>, in a fresh worktree per session; it prepares branches and Local PRs for your review.</>) : <>Starts each session in a temporary folder, saved on Home and deleted 30 days after the session is archived. It finds the workspace's folders with the Launchpad CLI and its notes, and works on a Git folder in a fresh worktree that it adds when a task needs it.</>}</li>
    <li>Remembers in the knowledge base of <b>{workspace}</b>: every change records it, and you can restore it.</li>
    <li>{tools.length ? <>Uses the tools granted to {workspace} with Home's tokens: {tools.map((t) => t.name).join(", ")}.</> : <>No tool is granted to {workspace} (Settings → Credentials and tools).</>}</li>
    <li>{!cli ? <>Without the Launchpad CLI tool, it cannot read Launchpad state or write proposals.</>
      : launchpad === "read" ? <>Reads Launchpad state with the Launchpad CLI; it cannot propose.</>
      : <>{launchpad === "admin" ? <><b>Admin</b>: changes Launchpad's bots, routines, schedules and workspaces, and starts agents, with the Launchpad CLI; it also </> : "Reads Launchpad state and "}writes proposals. {proposals === "auto" ? "Auto: one-time starts apply at once; the rest waits for your click." : "Every proposal waits for your click."}</>}</li>
  </ul>;
}

function NewBot({ first, onCancel }: { first: boolean; onCancel?: () => void }) {
  const qc = useQueryClient();
  const navigate = useNavigate();
  const scope = useScope();
  const profiles = useProfiles();
  const ws = scope.workspace.id || null;
  const folders = scope.projects.filter((p) => !p.is_temp).sort((a, b) => a.name.localeCompare(b.name));
  const [name, setName] = useState("");
  const [job, setJob] = useState("");
  const [profile, setProfile] = useState<string>(profiles.find((p) => p.billing === "subscription")?.id ?? profiles[0]?.id ?? "");
  const [folder, setFolder] = useState<string>("");
  // A template's starters (the chips over the message box), kept while its name is.
  const [starters, setStarters] = useState<{ name: string; list: string[] } | null>(null);
  const [busy, setBusy] = useState(false);
  useEffect(() => { if (!profile && profiles.length) setProfile(profiles.find((p) => p.billing === "subscription")?.id ?? profiles[0].id); }, [profiles, profile]);
  const repo = folders.find((p) => !p.plain);
  const templates = useBotTemplates().data ?? [];
  const create = async () => {
    if (!ws || !name.trim()) return;
    setBusy(true);
    try {
      const bot = await api.post<Bot>("/api/bots", { workspace_id: ws, name: name.trim(), instructions: job.trim() ? `# ${name.trim()}\n\n${job.trim()}` : "", profile: profile || null, folder: folder || null, starters: starters?.name === name.trim() ? starters.list : [] });
      await Promise.all([qc.invalidateQueries({ queryKey: ["bots"] }), qc.invalidateQueries({ queryKey: ["projects"] })]);
      void navigate({ to: "/bots/$id", params: { id: bot.id } });
    } catch (e) { toastError(e); } finally { setBusy(false); }
  };
  return (
    <section className="grid gap-3 rounded-card border border-line bg-paper p-4" data-tour="bots.form">
      <div>
        <h2 className="m-0 text-[15px] font-semibold">{first ? "Create your first bot" : "New bot"}</h2>
        <p className="m-0 mt-1 text-[13px] text-ink-2">A bot is an agent that stays: one thread you come back to, its own instructions, and notes it keeps in the knowledge base between sessions. Describe its job; you can change everything later.</p>
      </div>
      {repo && !name && !job && <div className="flex flex-wrap items-center gap-1.5 text-[12px] text-ink-3">Start from
        <button className="btn btn-sm" onClick={() => { setName("Maintainer"); setJob(`Maintain ${repo.name}: answer my questions about the code, investigate issues and failing checks, and prepare fixes as Local PRs for my review.`); setFolder(repo.id); }}><Icon name="branch" size={12} />Repo maintainer for {repo.name}</button>
      </div>}
      {templates.length > 0 && !name && !job && <div className="flex flex-wrap items-center gap-1.5 text-[12px] text-ink-3">{repo ? "Or from" : "Start from"}
        {templates.map((t) => <button key={t.id} className="btn btn-sm" onClick={() => { setName(t.name); setStarters({ name: t.name, list: t.starters ?? [] }); setJob(t.instructions.replace(/^#[^\n]*\n+/, "").trim()); setFolder(""); }}><Icon name="spark" size={12} />{t.name}</button>)}
      </div>}
      <label className="grid gap-1 text-[12px] font-semibold text-ink-2">Name
        <input className="field-input font-normal" maxLength={40} value={name} placeholder="Scout" onChange={(e) => setName(e.target.value)} autoFocus />
      </label>
      <label className="grid gap-1 text-[12px] font-semibold text-ink-2">Its job
        <textarea className="field-input min-h-[76px] py-1.5 font-normal leading-5" value={job} placeholder="What it does and how you like it done. For example: “Watch the Xet changelog and tell me only about breaking changes.”" onChange={(e) => setJob(e.target.value)} />
        <span className="font-normal text-ink-3">Saved as BOT.md, the bot's instructions.</span>
      </label>
      <div className="grid gap-3 sm:grid-cols-2">
        <label className="grid gap-1 text-[12px] font-semibold text-ink-2">Profile
          {profiles.length ? <select className="field-input font-normal" value={profile} onChange={(e) => setProfile(e.target.value)}>{profiles.map((p) => <option key={p.id} value={p.id}>{p.name} · {p.billing === "subscription" ? "subscription" : "pay as you go"}</option>)}</select>
            : <Link to="/settings/profiles" className="btn btn-sm w-fit">Add a profile first</Link>}
        </label>
        <label className="grid gap-1 text-[12px] font-semibold text-ink-2">Folder
          <select className="field-input font-normal" value={folder} onChange={(e) => setFolder(e.target.value)}>
            <option value="">None: a temporary folder per session</option>
            {folders.map((p) => <option key={p.id} value={p.id}>{p.name}{p.plain ? " · plain folder" : ""}</option>)}
          </select>
        </label>
      </div>
      <div className="rounded-control bg-sunk px-3 py-2">
        <div className="seclabel pb-1">What it can reach</div>
        <Access folder={folders.find((p) => p.id === folder) ?? null} workspace={scope.workspace.name} workspaceId={ws} mode="yolo" />
      </div>
      <div className="flex flex-wrap items-center justify-end gap-2">
        <span className="mr-auto text-[12px] text-ink-3">Creating saves the bot. Nothing runs until you send it a task.</span>
        {onCancel && <button className="btn btn-sm btn-ghost" onClick={onCancel}>Cancel</button>}
        <button className="btn btn-sm btn-primary" disabled={busy || !name.trim() || !ws || !profiles.length} onClick={() => void create()}><Icon name="plus" size={12} />Create bot</button>
      </div>
    </section>
  );
}

// ---------------------------------------------------------------- a bot's page

/** The bot page's About drawer is open for this bot on this device (browser storage; the demo keeps nothing). */
const ABOUT_KEY = (id: string) => `launchpad.bot-about.${id}`;
const aboutRemembered = (id: string) => !window.__lpDemo && localStorage.getItem(ABOUT_KEY(id)) === "1";
const rememberAbout = (id: string, open: boolean) => {
  if (window.__lpDemo) return;
  if (open) localStorage.setItem(ABOUT_KEY(id), "1"); else localStorage.removeItem(ABOUT_KEY(id));
};

/** A bot's page: the chat (the whole thread) is the page; About (a drawer on the laptop, a page on the phone) holds
 * what defines the bot: BOT.md, Triggers, Memory, Access and Versions. The trace of each session stays in Agents. */
export function BotPage() {
  const { id } = useParams({ strict: false }) as { id: string };
  // One page per bot: nothing of the previous bot's page stays when another bot opens.
  return <BotPageOf key={id} id={id} />;
}

function BotPageOf({ id }: { id: string }) {
  const phone = usePhone();
  const bots = useBots();
  const bot = bots.data?.find((b) => b.id === id);
  const workspaceId = useScope().workspace.id;
  const sessions = useBotSessions();
  const thread = useMemo(() => botThread(sessions, id), [sessions, id]);
  const current = currentSession(thread);
  const search = useSearch({ strict: false }) as { panel?: string };
  const navigate = useNavigate();
  const router = useRouter();
  // The URL's ?panel= is the only source of the open About (older links name a panel: they open About too). The
  // laptop opens it again when it was open the last time on this device; the phone shows the chat first, and opening
  // About from the chat adds one history entry, so Back (and ‹) returns to the chat.
  const about = !!search.panel;
  const pushed = useRef(false);
  // Android back (or the browser's) closed About: its entry is gone, so ‹ must not go back once more.
  useEffect(() => { if (!about) pushed.current = false; }, [about]);
  const setAbout = (open: boolean) => {
    if (!phone) rememberAbout(id, open);
    if (phone && !open && pushed.current) { pushed.current = false; router.history.back(); return; }
    const push = phone && open && !about;
    if (push) pushed.current = true;
    void navigate({ to: "/bots/$id", params: { id }, search: (open ? { panel: "about" } : {}) as never, replace: !push });
  };
  useEffect(() => {
    if (!phone && !search.panel && aboutRemembered(id)) void navigate({ to: "/bots/$id", params: { id }, search: { panel: "about" } as never, replace: true });
  }, []); // eslint-disable-line react-hooks/exhaustive-deps
  const routines = (useCrons().data ?? []).filter((c) => c.bot_id === id);
  const profiles = useProfiles();
  const where = useRunnerTag(current?.runner_id);
  const [newSession, setNewSession] = useState<NewSessionRequest | null>(null);
  const [asking, setAsking] = useState(false);
  const [file, setFile] = useState<string | null>(null);
  const [step, setStep] = useState<string | null>(null);
  if (bots.isLoading) return <Loading what="the bot" />;
  if (bots.error) return <ErrorState error={bots.error} onRetry={() => void bots.refetch()} />;
  if (!bot) return <div className="flex h-full flex-col"><PageHeader title="Bot" /><Empty fill icon="spark" action={<Link to="/bots" className="btn btn-sm">All bots</Link>}>This bot no longer exists.</Empty></div>;
  const profile = botProfile(bot, profiles);
  const rotation = rotationReason(bot, current, profile);
  const status = statusOf(bot, thread, current, routines, step);
  const statusLine = <span className={`min-w-0 truncate ${status.tone}`} title={where && where.tone !== "quiet" ? where.title : undefined} data-tour="bots.status">{status.text}{where && where.tone !== "quiet" ? ` · ${where.label}` : ""}</span>;
  const menu = (close: () => void) => <BotMenu bot={bot} thread={thread} current={current} rotation={rotation} close={close}
    onNewSession={() => setAsking(true)} onEditFile={() => setFile("bot.toml")} />;
  const chat = <BotChat bot={bot} thread={thread} rotation={rotation} needsProfile={!profile} openSettings={() => setAbout(true)} newSession={newSession} onNewSession={setNewSession} onStep={setStep}
    intro={<p className="m-0 text-[13px] text-ink-2">Send {bot.settings.name} its first task.</p>} />;
  const sections = <AboutSections bot={bot} current={current} routines={routines} onEditFile={setFile} />;
  const dialogs = <>
    {asking && <NewSessionDialog bot={bot} onClose={() => setAsking(false)} onStart={(r) => { setNewSession(r); setAsking(false); if (phone && about) setAbout(false); }} />}
    {file && <FilesDialog bot={bot} routines={routines} path={file} onPath={setFile} onClose={() => setFile(null)} />}
  </>;
  const more = <MoreMenu label={`More actions for ${bot.settings.name}`} width={300}>{menu}</MoreMenu>;
  if (phone) {
    // The Ask tab shows the workspace's default bot as a tab of its own: no way back to All bots from its chat.
    const tabRoot = askTarget(bots.data ?? [], workspaceId)?.id === bot.id;
    const info = <IconButton icon="info" label={`About ${bot.settings.name}`} data-tour="bots.about" onClick={() => setAbout(true)} />;
    return <div className="flex h-full min-h-0 flex-col" data-tour="bots.bot">
      {about
        ? <PhoneBar title={`About ${bot.settings.name}`} onBack={() => setAbout(false)} backLabel="Back to the chat" end={more} />
        : tabRoot
          ? <><PhoneBarEnd>{info}</PhoneBarEnd><header className="phone-page-header flex min-w-0 shrink-0 items-center gap-2 border-b border-line px-3 py-1.5 text-[12px]"><BotAvatar name={bot.settings.name} size={20} /><b className="text-[13px] text-ink">{bot.settings.name}</b>{statusLine}</header></>
          : <PhoneBar title={<span className="grid"><span className="truncate">{bot.settings.name}</span><span className="grid text-[12px] font-normal">{statusLine}</span></span>} onBack={() => void navigate({ to: "/bots" })} backLabel="All bots" end={info} />}
      {about ? <div className="min-h-0 flex-1 overflow-x-hidden overflow-y-auto" data-tour="bots.about-panel">{sections}</div> : chat}
      {dialogs}
    </div>;
  }
  const drawer = <div className="flex h-full min-h-0 flex-col" data-tour="bots.about-panel">
    <div className="flex min-h-11 flex-none items-center gap-2 border-b border-line-2 px-3"><b className="min-w-0 flex-1 truncate text-[13px]">About {bot.settings.name}</b><IconButton icon="x" label="Close About" onClick={() => setAbout(false)} /></div>
    <div className="min-h-0 flex-1 overflow-x-hidden overflow-y-auto">{sections}</div>
  </div>;
  return (
    <div className="flex h-full min-h-0 flex-col" data-tour="bots.bot">
      <PageHeader title={<span className="flex min-w-0 items-center gap-2"><BotAvatar name={bot.settings.name} size={24} /><span className="truncate">{bot.settings.name}</span></span>}
        meta={statusLine}
        actions={<><button className={`btn btn-sm ${about ? "border-teal/40 bg-teal-soft text-teal-ink" : ""}`} aria-pressed={about} data-tour="bots.about" title={`What defines ${bot.settings.name}: BOT.md, triggers, memory, access and versions`} onClick={() => setAbout(!about)}><Icon name="info" size={12} />About</button>{more}</>} />
      {/* Always the split view, so the chat (and a draft in its message box) stays mounted when About opens or closes. */}
      <SplitView storageKey="launchpad.bot-about-width" sideLabel="About" initial={380} min={320} maxRatio={0.5} main={chat} side={about ? drawer : null} />
      {dialogs}
    </div>
  );
}

/** The header's one status line, in words: what the bot does now, or when it runs next. */
function statusOf(bot: Bot, thread: Session[], current: Session | null, routines: CronAgent[], step: string | null): { text: string; tone: string } {
  if (current?.open_question) return { text: "Waiting for you · 1 question", tone: "text-attn" };
  if (current && isRunning(current)) return { text: `Working${step ? ` · ${step}` : ""} (${age(current.status_since)})`, tone: "text-work" };
  if (current && isQuotaPaused(current)) return { text: "Quota paused · continues when the quota allows", tone: "text-attn" };
  if (bot.paused) return { text: "Paused · no trigger starts it; your messages still go through", tone: "text-ink-2" };
  if (current?.status === "error") return { text: "The last turn failed", tone: "text-gh-closed" };
  if (current?.status === "interrupted") return { text: "The last turn was interrupted", tone: "text-gh-closed" };
  if (!thread.length) return { text: "No task yet", tone: "text-ink-3" };
  const next = routines.filter((r) => r.next_run_at != null && !r.paused).sort((a, b) => a.next_run_at! - b.next_run_at!)[0];
  return { text: next ? `Idle · next: ${next.title} ${when(next.next_run_at!)}` : "Idle", tone: "text-ink-3" };
}

/** ⋯ of the bot page (header on the laptop, About on the phone). */
function BotMenu({ bot, thread, current, rotation, close, onNewSession, onEditFile }: { bot: Bot; thread: Session[]; current: Session | null; rotation: string | null; close: () => void; onNewSession: () => void; onEditFile: () => void }) {
  const qc = useQueryClient();
  const navigate = useNavigate();
  const running = thread.some(isRunning);
  // A new session starts only between turns; with no open session (or a changed profile, folder, model…) the next
  // message starts one anyway.
  const newDetail = !current ? "Your next message starts one anyway" : rotation ? `${rotation}: your next message starts one anyway` : isRunning(current) ? "Wait for the turn to end, or Stop it" : "The next message starts it, with your reason";
  const refresh = () => Promise.all([qc.invalidateQueries({ queryKey: ["bots"] }), qc.invalidateQueries({ queryKey: ["crons"] })]);
  return <>
    <ActionMenuItem name="bot.pause" params={{ id: bot.id }} body={{ paused: !bot.paused }} close={close} icon={bot.paused ? "play" : "stop"}
      label={bot.paused ? "Resume all triggers" : "Pause all triggers"} detail={bot.paused ? "Routines and hooks start runs again" : "Routines and hooks start nothing; your messages still go through"}
      onDone={() => refresh().then(() => toast(bot.paused ? `${bot.settings.name} resumed` : `${bot.settings.name} paused: no trigger starts it`))} />
    <MenuItem icon={<Icon name="plus" size={14} />} label="Start a new session…" detail={newDetail} disabled={!current || !!rotation || isRunning(current)} onClick={() => { close(); onNewSession(); }} />
    <MenuItem icon={<Icon name="agents" size={14} />} label="Open current session in Agents" detail={current ? "Its trace, terminal and files" : "No open session"} disabled={!current} onClick={() => { close(); if (current) void navigate({ to: "/agents", search: { session: current.id } as never }); }} />
    <MenuItem icon={<Icon name="file" size={14} />} label="Edit as file" detail="bot.toml and its routine files" onClick={() => { close(); onEditFile(); }} />
    {running
      ? <MenuItem icon={<Icon name="trash" size={14} />} label="Delete bot" detail="Stop its session first" disabled onClick={() => {}} />
      : <ActionMenuItem name="bot.delete" params={{ id: bot.id }} close={close} onDone={async () => { await qc.invalidateQueries({ queryKey: ["bots"] }); void navigate({ to: "/bots" }); }} />}
  </>;
}

/** A small dialog: centered on the laptop, a sheet on the phone. */
function Dialog({ title, onClose, wide, children }: { title: string; onClose: () => void; wide?: boolean; children: ReactNode }) {
  const phone = usePhone();
  if (phone) return <Sheet title={title} full={wide} onClose={onClose}><div className="grid gap-3 p-3">{children}</div></Sheet>;
  return <div className="fixed inset-0 z-50 grid place-items-center bg-black/35 p-6" onClick={onClose} onKeyDown={(e) => { if (e.key === "Escape") onClose(); }}>
    <div role="dialog" aria-modal="true" aria-label={title} className={`flex max-h-[calc(86vh/var(--lp-zoom,1))] ${wide ? "w-[min(900px,calc(92vw/var(--lp-zoom,1)))]" : "w-[min(480px,calc(92vw/var(--lp-zoom,1)))]"} flex-col overflow-hidden rounded-card border border-line bg-paper shadow-[0_20px_60px_-20px_rgba(23,32,43,.6)]`} onClick={(e) => e.stopPropagation()}>
      <div className="flex items-center gap-2 border-b border-line-2 px-4 py-2.5">
        <b className="min-w-0 flex-1 truncate text-[15px]">{title}</b>
        <IconButton icon="x" label="Close" onClick={onClose} />
      </div>
      <div className="grid min-h-0 flex-1 gap-3 overflow-y-auto p-4">{children}</div>
    </div>
  </div>;
}

/** ⋯ → Start a new session: a short reason; the next message starts the session and its divider shows the reason. */
function NewSessionDialog({ bot, onClose, onStart }: { bot: Bot; onClose: () => void; onStart: (r: NewSessionRequest) => void }) {
  const [reason, setReason] = useState("");
  return <Dialog title="Start a new session" onClose={onClose}>
    <p className="m-0 text-[13px] text-ink-2">Your next message starts a new session of {bot.settings.name}'s thread. It reads the previous session's trace only when it needs it.</p>
    <label className="grid gap-1 text-[12px] font-semibold text-ink-2">Why (optional, shown in the chat)
      <input className="field-input font-normal" maxLength={120} value={reason} placeholder="Fresh context for the release" autoFocus onChange={(e) => setReason(e.target.value)} onKeyDown={(e) => { if (e.key === "Enter") onStart({ reason: reason.trim() }); }} />
    </label>
    <div className="flex justify-end gap-2">
      <button className="btn btn-sm btn-ghost" onClick={onClose}>Cancel</button>
      <button className="btn btn-sm btn-primary" onClick={() => onStart({ reason: reason.trim() })}>Start with my next message</button>
    </div>
  </Dialog>;
}

/** ⋯ → Edit as file: bot.toml or one of its routine files. Saving checks it; the bot and its routines follow. */
function FilesDialog({ bot, routines, path, onPath, onClose }: { bot: Bot; routines: CronAgent[]; path: string; onPath: (p: string) => void; onClose: () => void }) {
  const qc = useQueryClient();
  const paths = ["bot.toml", ...Object.keys(bot.routines).sort()];
  const routine = routines.find((r) => r.file === path);
  const value = path === "bot.toml" ? bot.settings_text : bot.routines[path] ?? "";
  const error = path === "bot.toml" ? bot.settings_error : routine?.file_error ?? null;
  const save = async (t: string) => {
    try {
      await runAction<Bot>("bot.update", { id: bot.id }, path === "bot.toml" ? { settings_text: t } : { files: { [path]: t } });
      await Promise.all([qc.invalidateQueries({ queryKey: ["bots"] }), qc.invalidateQueries({ queryKey: ["crons"] })]);
      toast(path === "bot.toml" ? "Saved: a new profile, folder, model, effort or permission mode starts a new session" : "Saved: the routine follows its file");
      return true;
    } catch (e) { toastError(e); return false; }
  };
  return <Dialog title={`Edit as file · ${bot.settings.name}`} wide onClose={onClose}>
    <label className="grid gap-1 text-[12px] font-semibold text-ink-2">File
      <select className="field-input font-mono text-[12px] font-normal" value={path} onChange={(e) => onPath(e.target.value)}>{paths.map((p) => <option key={p} value={p}>{p}</option>)}</select>
    </label>
    {error && <p className="m-0 rounded-control bg-attn-soft px-2 py-1 text-[12px] text-attn">{path} cannot be used: {error}</p>}
    <FileEditor key={`${path}-${value}`} value={value} mono rows={14} onSave={save} />
  </Dialog>;
}

export function Section({ title, children, action, tour }: { title: ReactNode; children: ReactNode; action?: ReactNode; tour?: string }) {
  return <section className="grid min-w-0 grid-cols-[minmax(0,1fr)] gap-2 border-b border-line-2 p-3" data-tour={tour}>
    <div className="flex min-h-7 items-center gap-2"><h3 className="seclabel m-0 min-w-0 flex-1">{title}</h3>{action}</div>
    {children}
  </section>;
}

// ---------------------------------------------------------------- About

/** What defines the bot, top to bottom: BOT.md, Triggers, Memory, Access, Versions. */
function AboutSections({ bot, current, routines, onEditFile }: { bot: Bot; current: Session | null; routines: CronAgent[]; onEditFile: (path: string) => void }) {
  const save = useSaveFiles(bot);
  return <>
    {/* Keyed on the file, not the store version: the bot writing its memory must not reset a draft. */}
    <InstructionsEditor key={`i-${bot.instructions}`} bot={bot} older={instructionsUpdated(bot, current)} onSave={(t) => save({ instructions: t })} />
    <TriggersSection bot={bot} routines={routines} onEditFile={onEditFile} />
    <MemorySection bot={bot} />
    <AccessSection bot={bot} />
    <VersionsSection bot={bot} />
  </>;
}

const useSaveFiles = (bot: Bot) => {
  const qc = useQueryClient();
  return async (patch: Record<string, string>) => {
    try { await runAction<Bot>("bot.update", { id: bot.id }, patch); await qc.invalidateQueries({ queryKey: ["bots"] }); toast(patch.instructions !== undefined ? "Saved: it applies to your next message" : "Saved"); return true; } catch (e) { toastError(e); return false; }
  };
};

/** BOT.md: the start of the rendered file; Edit BOT.md opens a large editor with a live preview. A draft survives
 * closing the editor until it is saved or discarded. `older`: the current session runs the previous version. */
function InstructionsEditor({ bot, older, onSave }: { bot: Bot; older: boolean; onSave: (text: string) => Promise<boolean> }) {
  const phone = usePhone();
  const [text, setText] = useState(bot.instructions);
  const [open, setOpen] = useState(false);
  const [mode, setMode] = useState<"read" | "edit">("read");
  const [busy, setBusy] = useState(false);
  const dirty = text !== bot.instructions;
  const save = async () => { setBusy(true); const ok = await onSave(text); setBusy(false); if (ok) setOpen(false); };
  const buttons = dirty && <>
    <button className="btn btn-sm btn-ghost" disabled={busy} onClick={() => setText(bot.instructions)}>Discard</button>
    <button className="btn btn-sm btn-primary" disabled={busy} onClick={() => void save()}>Save</button>
  </>;
  const close = () => setOpen(false);
  // The fade shows only when the preview is cut.
  const preview = useRef<HTMLDivElement>(null);
  const [cut, setCut] = useState(false);
  useLayoutEffect(() => { const el = preview.current; if (el) setCut(el.scrollHeight > el.clientHeight + 1); }, [text]);
  const editor = <textarea className="field-input h-auto min-h-0 flex-1 resize-none py-2 text-[13px] leading-5" value={text} onChange={(e) => setText(e.target.value)} aria-label="BOT.md" autoFocus />;
  return <Section title={<span className="normal-case">BOT.md</span>} tour="bots.files" action={<button className="btn btn-sm" title="Read and edit BOT.md in a large editor" onClick={() => setOpen(true)}><Icon name="pencil" size={12} />Edit BOT.md</button>}>
    <div ref={preview} className="relative max-h-[120px] cursor-pointer overflow-hidden rounded-control bg-sunk px-3 py-2" title="Open BOT.md" onClick={() => setOpen(true)}>
      {text.trim() ? <Markdown text={text} className="text-[12px]" /> : <span className="text-[12px] text-ink-3">No instructions yet. Edit BOT.md to write what {bot.settings.name} does.</span>}
      {cut && <span className="pointer-events-none absolute inset-x-0 bottom-0 h-10 bg-linear-to-t from-sunk to-transparent" />}
    </div>
    {older && !dirty && <p className="m-0 flex items-center gap-1.5 text-[12px] text-ink-3"><Icon name="file" size={12} className="flex-none" />The current session runs the previous version; your next message updates it.</p>}
    {dirty && <div className="flex items-center justify-end gap-1.5"><span className="mr-auto text-[12px] text-attn">Unsaved changes</span>{buttons}</div>}
    {open && (phone
      ? <Sheet title="BOT.md" full onClose={close}>
        {/* Read first (no keyboard on open); Save and Discard stay above the keyboard. */}
        <div className="flex h-full min-h-0 flex-col gap-2 p-3">
          <div className="flex flex-none items-center gap-1.5">
            <Tabs label="BOT.md view" items={[{ id: "read", label: "Read" }, { id: "edit", label: "Edit" }]} value={mode} onChange={setMode} className="mr-auto" />
            {buttons}
          </div>
          {mode === "edit" ? editor : <div className="min-h-0 flex-1 overflow-x-hidden overflow-y-auto [overflow-wrap:anywhere]">{text.trim() ? <Markdown text={text} className="text-[15px]" /> : <span className="text-[13px] text-ink-3">No instructions yet. Tap Edit to write what {bot.settings.name} does.</span>}</div>}
        </div>
      </Sheet>
      : <div className="fixed inset-0 z-50 grid place-items-center bg-black/35 p-6" onClick={close} onKeyDown={(e) => { if (e.key === "Escape") close(); }}>
        <div role="dialog" aria-modal="true" aria-label={`BOT.md of ${bot.settings.name}`} className="flex h-[calc(86vh/var(--lp-zoom,1))] w-[min(1200px,calc(92vw/var(--lp-zoom,1)))] flex-col overflow-hidden rounded-card border border-line bg-paper shadow-[0_20px_60px_-20px_rgba(23,32,43,.6)]" onClick={(e) => e.stopPropagation()}>
          <div className="flex items-center gap-2 border-b border-line-2 px-4 py-2.5">
            <b className="min-w-0 flex-1 truncate text-[15px]">BOT.md · {bot.settings.name}</b>
            <span className="text-[12px] text-ink-3">Applies to your next message</span>
            <button className="iconbtn" aria-label="Close" title="Close (your draft stays until you discard it)" onClick={close}><Icon name="x" size={14} /></button>
          </div>
          <div className="grid min-h-0 flex-1 grid-cols-2">
            <div className="flex min-h-0 min-w-0 flex-col border-r border-line-2 p-3"><div className="seclabel pb-1.5">Edit</div>{editor}</div>
            <div className="flex min-h-0 min-w-0 flex-col p-3"><div className="seclabel pb-1.5">Preview</div><div className="min-h-0 flex-1 overflow-x-hidden overflow-y-auto [overflow-wrap:anywhere]">{text.trim() ? <Markdown text={text} className="text-[13px]" /> : <span className="text-[12px] text-ink-3">Nothing to preview.</span>}</div></div>
          </div>
          <div className="flex min-h-[48px] items-center justify-end gap-1.5 border-t border-line-2 px-4 py-2">{dirty ? buttons : <span className="text-[12px] text-ink-3">No change</span>}</div>
        </div>
      </div>)}
  </Section>;
}

// ---------------------------------------------------------------- triggers

const TRIGGER_ICON: Record<CronAgent["trigger"]["kind"], IconName> = { schedule: "clock", github: "pr", session_done: "agents", failure: "alert" };

/** Every way the bot starts on its own, in one list: its routines (with their last runs), incoming hooks that message
 * it and its Telegram command, with one Pause for all of them. */
function TriggersSection({ bot, routines, onEditFile }: { bot: Bot; routines: CronAgent[]; onEditFile: (path: string) => void }) {
  const qc = useQueryClient();
  const phone = usePhone();
  const projects = useProjects().data ?? [];
  const profiles = useProfiles();
  const navigate = useNavigate();
  const channels = useQuery({ queryKey: ["channels"], queryFn: () => api.get<ChannelsOverview>("/api/channels"), retry: false }).data;
  const hooks = (channels?.hooks ?? []).filter((h) => h.target.kind === "bot" && h.target.id === bot.id);
  const telegram = !!useTelegram(bot.workspace_id).data?.bound;
  const askBot = askTarget(useBots().data ?? [], bot.workspace_id)?.id === bot.id;
  const [adding, setAdding] = useState(false);
  const [open, setOpen] = useState<string | null>(null);
  const folder = projects.find((p) => p.id === bot.settings.folder);
  const repo = folder && !folder.plain ? folder.gh_repo ?? folder.gh_upstream ?? null : null;
  const harness = profiles.find((p) => p.id === bot.settings.profile)?.harness ?? null;
  const folderName = (fid: string) => projects.find((p) => p.id === fid)?.name ?? "a removed folder";
  const pause = <ActionButton name="bot.pause" params={{ id: bot.id }} body={{ paused: !bot.paused }} icon={bot.paused ? "play" : "stop"}
    title={bot.paused ? "Routines and incoming hooks start runs again" : "Routines and incoming hooks start nothing until you resume. Your own messages still go through."}
    onDone={() => Promise.all([qc.invalidateQueries({ queryKey: ["bots"] }), qc.invalidateQueries({ queryKey: ["crons"] })]).then(() => toast(bot.paused ? `${bot.settings.name} resumed` : `${bot.settings.name} paused: no trigger starts it`))}>
    <span data-tour="bots.pause">{bot.paused ? "Resume all" : "Pause all"}</span>
  </ActionButton>;
  const row = "flex min-h-9 w-full min-w-0 items-center gap-2 rounded-control px-1.5 py-1 text-left text-[13px] text-ink no-underline hover:bg-sunk";
  return <Section title="Triggers" tour="bots.triggers" action={pause}>
    {/* The laptop header's status line says it already; the phone's About page has no status line. */}
    {bot.paused && phone && <p className="m-0 text-[12px] text-ink-2">Paused: no trigger starts {bot.settings.name} until you resume it. Your own messages still go through.</p>}
    <div className="grid min-w-0 grid-cols-[minmax(0,1fr)] gap-0.5" data-tour="bots.routines">
      {routines.map((r) => <div key={r.id} className="grid min-w-0 grid-cols-[minmax(0,1fr)] gap-1">
        <button className={row} aria-expanded={open === r.id} title={cronTitle(r)} onClick={() => setOpen(open === r.id ? null : r.id)}>
          <Icon name={TRIGGER_ICON[r.trigger.kind]} size={12} className="flex-none text-ink-3" />
          <span className="min-w-0 flex-1"><span className="block truncate">{r.title}</span><span className="block truncate text-[12px] text-ink-3">{r.file_error ? <span className="text-attn">File error · </span> : null}{triggerSummary(r, folderName)} · {routineState(r, bot.settings.name)}</span></span>
          <RunStrip history={r.history} count={5} />
        </button>
        {open === r.id && <RoutineDetails cron={r} bot={bot} onOpen={(sid) => void navigate({ to: "/agents", search: { session: sid } as never })} onDeleted={() => setOpen(null)} onEditFile={r.file ? () => onEditFile(r.file!) : undefined} />}
      </div>)}
      {hooks.map((h) => <Link key={h.id} to="/settings/channels" className={row} title="Incoming hook: edit or delete it in Settings → Channels">
        <Icon name="down" size={12} className="flex-none text-ink-3" />
        <span className="min-w-0 flex-1"><span className="block truncate">Hook “{h.name}”</span><span className="block truncate text-[12px] text-ink-3">{h.last_at ? `Last call ${ago(h.last_at)}` : "Never called"} · {h.count} message{h.count === 1 ? "" : "s"}</span></span>
      </Link>)}
      {telegram && <Link to="/settings/workspace" className={row} title="The workspace's Telegram chat: /command followed by a task messages this bot">
        <Icon name="send" size={12} className="flex-none text-ink-3" />
        <span className="min-w-0 flex-1"><span className="block truncate">Telegram /{bot.slug.replace(/-/g, "_")}</span><span className="block truncate text-[12px] text-ink-3">From the workspace's chat{askBot ? " · plain text too" : ""}</span></span>
      </Link>}
      {!routines.length && !hooks.length && !telegram && !adding && <p className="m-0 text-[12px] text-ink-3">No trigger yet: {bot.settings.name} runs only when you message it.</p>}
    </div>
    {adding ? <NewRoutine bot={bot} repo={repo} harness={harness} onDone={() => setAdding(false)} />
      : <button className="btn btn-sm w-fit" data-tour="bots.routine-new" title="A routine: on a schedule, on GitHub activity, when another agent is done or on a Launchpad failure. Incoming hooks are created in Settings → Channels." onClick={() => setAdding(true)}><Icon name="plus" size={12} />New trigger</button>}
  </Section>;
}

// ---------------------------------------------------------------- memory

/** The lines of its notes a new session gets in its instructions (`bot_memory::LOADED_LINES`). */
const MEMORY_LINES = 100;
const fileSize = (n: number) => n < 1024 ? `${n} B` : `${Math.round(n / 1024)} KB`;

function MemorySection({ bot }: { bot: Bot }) {
  const [open, setOpen] = useState<string | null>(null);
  const loaded = bot.memory_loaded;
  return <Section tour="bots.memory" title={<>Memory{loaded.total > 0 && <span className="normal-case" data-tour="bots.memory-loaded" title={`A new session gets up to ${MEMORY_LINES} lines of its notes in its instructions, and reads the rest itself`}> · {loaded.loaded < loaded.total ? `loaded at start: ${loaded.loaded} of ${loaded.total} lines` : "loaded at start"}</span>}</>}>
    {bot.memory.length === 0
      ? <p className="m-0 text-[12px] text-ink-3">No note yet. {bot.settings.name} writes here what it needs to remember between sessions.</p>
      : bot.memory.map((f) => <div key={f.path} className="min-w-0">
        <button className="flex min-h-7 w-full min-w-0 items-center gap-1.5 text-left text-[13px] hover:underline" aria-expanded={open === f.path} onClick={() => setOpen(open === f.path ? null : f.path)}>
          <Icon name="file" size={12} className="flex-none text-ink-3" /><span className="min-w-0 flex-1 truncate">{f.path.replace(/^memory\//, "")}</span><span className="flex-none text-[12px] text-ink-3">{fileSize(f.size)}</span>
        </button>
        {open === f.path && <NoteText bot={bot.id} path={f.path} sha={f.sha256} />}
      </div>)}
  </Section>;
}

function NoteText({ bot, path, sha }: { bot: string; path: string; sha: string }) {
  const q = useBotFile(bot, path, sha);
  return q.isLoading ? <Loading what="the note" /> : q.error ? <ErrorState error={q.error} /> : <div className="mb-2 rounded-control bg-sunk px-3 py-2"><Markdown text={q.data ?? ""} className="text-[13px]" /></div>;
}

/** One of a bot's files as text (`GET /api/bots/<id>/file`). */
function useBotFile(bot: string, path: string, version: string) {
  return useQuery({ queryKey: ["bot-file", bot, path, version], staleTime: Infinity, queryFn: async () => { const r = await fetch(daemonUrl(`/api/bots/${bot}/file?path=${encodeURIComponent(path)}`), { credentials: "include" }); if (!r.ok) throw new Error(await r.text()); return r.text(); } });
}

// ---------------------------------------------------------------- access

/** Access as one sentence (profile, model, folder, permission mode, Launchpad access, tools), Edit for the compact
 * form, and What it sees at start. */
function AccessSection({ bot }: { bot: Bot }) {
  const profiles = useProfiles();
  const projects = useProjects().data ?? [];
  const [editing, setEditing] = useState(false);
  const [preview, setPreview] = useState(false);
  const s = bot.settings;
  const profile = botProfile(bot, profiles);
  const layers = useLayers(bot.workspace_id, s.folder ?? null, s.tools);
  const tools = useGrantedTools(bot.workspace_id).filter((t) => t.id !== "launchpad" && defaultOn(layers, t.id));
  const access = CLI_ACCESS.find((a) => a.id === (s.launchpad ?? "propose"))!;
  const parts = [
    profile ? profile.name : "no profile",
    s.model ?? profile?.model ?? "default model",
    folderLabel(projects, s.folder),
    PERMISSIONS.find((p) => p.id === s.permission_mode)?.label,
    `${access.label.toLowerCase()}${(s.proposals ?? "manual") === "auto" ? " (auto proposals)" : ""}`,
    s.inbox_done && access.id !== "read" ? "marks Inbox threads done" : null,
    tools.length ? tools.map((t) => t.name).join(", ") : "no tools",
  ].filter(Boolean);
  return <Section title="Access" tour="bots.settings" action={!editing && <button className="btn btn-sm" onClick={() => setEditing(true)}><Icon name="pencil" size={12} />Edit</button>}>
    {editing ? <SettingsForm bot={bot} onClose={() => setEditing(false)} /> : <>
      <p className="m-0 text-[13px] text-ink-2" title="Profile · model · folder · permission mode · Launchpad CLI access · tools">{parts.join(" · ")}</p>
      <button className="btn btn-sm w-fit" data-tour="bots.start-preview" title="The instructions and first prompt of its next new session, as Launchpad builds them" onClick={() => setPreview(true)}><Icon name="eye" size={12} />What it sees at start</button>
    </>}
    {preview && <StartPreviewDialog queryKey={["bot", bot.id, bot.version]} load={() => api.get<StartPreview>(`/api/bots/${bot.id}/start-preview`)} subject={`${bot.settings.name}'s next session`} onClose={() => setPreview(false)} />}
  </Section>;
}

/** The compact settings form: bot.toml's fields as selects. */
function SettingsForm({ bot, onClose }: { bot: Bot; onClose: () => void }) {
  const qc = useQueryClient();
  const profiles = useProfiles();
  const projects = useProjects().data ?? [];
  const [s, setS] = useState<BotSettings>(bot.settings);
  const profile = botProfile({ ...bot, settings: s }, profiles);
  const fallback = profiles.find((p) => p.billing === "subscription") ?? profiles[0];
  const models = useHarnessDefaults(profile && !profile.models_url ? profile.harness : null).data?.models ?? [];
  const folders = projects.filter((p) => p.workspace_id === bot.workspace_id && !p.is_temp);
  const dirty = JSON.stringify(s) !== JSON.stringify(bot.settings);
  const toolLayers = useLayers(bot.workspace_id, s.folder ?? null);
  const access = s.launchpad ?? "propose";
  const save = async () => {
    try {
      await runAction<Bot>("bot.update", { id: bot.id }, { settings: s });
      await qc.invalidateQueries({ queryKey: ["bots"] });
      toast(s.profile !== bot.settings.profile || (s.folder ?? null) !== (bot.settings.folder ?? null) || s.model !== bot.settings.model || s.effort !== bot.settings.effort || s.permission_mode !== bot.settings.permission_mode ? "Saved: the next message starts a new session" : "Saved");
      onClose();
    } catch (e) { toastError(e); }
  };
  const field = "grid gap-1 text-[12px] font-semibold text-ink-2";
  return <div className="grid gap-2.5">
    <label className={field}>Name<input className="field-input font-normal" maxLength={40} value={s.name} onChange={(e) => setS({ ...s, name: e.target.value })} /></label>
    <label className={field}>Profile
      <select className="field-input font-normal" value={s.profile ?? ""} onChange={(e) => setS({ ...s, profile: e.target.value || undefined, model: undefined, effort: undefined })}>
        <option value="">Default{fallback ? `: ${fallback.name}` : ""}</option>
        {s.profile && !profiles.some((p) => p.id === s.profile) && <option value={s.profile}>A removed profile</option>}
        {profiles.map((p) => <option key={p.id} value={p.id}>{p.name}</option>)}
      </select>
    </label>
    <label className={field}>Model
      <select className="field-input font-normal" value={s.model ?? ""} onChange={(e) => setS({ ...s, model: e.target.value || undefined })}>
        <option value="">Default of the profile{profile?.model ? ` (${profile.model})` : ""}</option>
        {models.map((m) => <option key={m.id} value={m.id}>{m.display_name || m.id}</option>)}
        {s.model && !models.some((m) => m.id === s.model) && <option value={s.model}>{s.model}</option>}
      </select>
    </label>
    <label className={field}>Folder
      <select className="field-input font-normal" value={s.folder ?? ""} onChange={(e) => setS({ ...s, folder: e.target.value || undefined })}>
        <option value="">None: a temporary folder per session</option>
        {s.folder && !folders.some((p) => p.id === s.folder) && <option value={s.folder}>A removed folder</option>}
        {folders.map((p) => <option key={p.id} value={p.id}>{p.name}{p.plain ? " · plain folder" : ""}</option>)}
      </select>
    </label>
    <label className={field}>Permissions
      <select className="field-input font-normal" value={s.permission_mode} title={PERMISSIONS.find((p) => p.id === s.permission_mode)?.detail} onChange={(e) => setS({ ...s, permission_mode: e.target.value as PermissionMode })}>
        {PERMISSIONS.map((p) => <option key={p.id} value={p.id}>{p.label} · {p.detail.toLowerCase()}</option>)}
      </select>
    </label>
    <label className={field}>Launchpad CLI
      <select className="field-input font-normal" value={access} title={CLI_ACCESS.find((m) => m.id === access)?.detail} onChange={(e) => setS({ ...s, launchpad: e.target.value as LaunchpadAccess })}>
        {CLI_ACCESS.map((m) => <option key={m.id} value={m.id}>{m.label}</option>)}
      </select>
      <span className="font-normal text-ink-3">{CLI_ACCESS.find((m) => m.id === access)?.detail}</span>
    </label>
    <label className={field}>Proposals
      <select className="field-input font-normal" value={s.proposals ?? "manual"} title={PROPOSAL_MODES.find((m) => m.id === (s.proposals ?? "manual"))?.detail} onChange={(e) => setS({ ...s, proposals: e.target.value as ProposalMode })}>
        {PROPOSAL_MODES.map((m) => <option key={m.id} value={m.id}>{m.label}</option>)}
      </select>
      <span className="font-normal text-ink-3">{PROPOSAL_MODES.find((m) => m.id === (s.proposals ?? "manual"))?.detail}</span>
    </label>
    <label className="flex min-h-7 items-center gap-2 text-[13px]" title="Marks your Inbox threads done or unsubscribes from them itself, here and on GitHub, without your click (needs Propose or Admin)">
      <input type="checkbox" className="accent-teal" disabled={access === "read"} checked={!!s.inbox_done && access !== "read"} onChange={(e) => setS({ ...s, inbox_done: e.target.checked || undefined })} />Marks Inbox threads done
    </label>
    <div className="grid gap-1">
      <span className="text-[12px] font-semibold text-ink-2">Default tools</span>
      <ToolDefaultsEditor workspaceId={bot.workspace_id} above={toolLayers.slice(0, 2)} value={s.tools ?? {}} onChange={(tools) => setS({ ...s, tools })} />
    </div>
    <div className="flex justify-end gap-1.5"><button className="btn btn-sm btn-ghost" onClick={onClose}>Cancel</button><button className="btn btn-sm btn-primary" disabled={!dirty || !s.name.trim()} onClick={() => void save()}>Save</button></div>
  </div>;
}

// ---------------------------------------------------------------- versions

/** Every version of its files (BOT.md, bot.toml, routines, memory) for 30 days, with Restore. */
function VersionsSection({ bot }: { bot: Bot }) {
  const history = useQuery({ queryKey: ["bot-history", bot.id, bot.version], queryFn: () => api.get<Change[]>(`/api/bots/${bot.id}/history`) });
  return <Section title="Versions · last 30 days" tour="bots.versions">
    {history.isLoading ? <Loading what="versions" /> : (history.data ?? []).length === 0 ? <p className="m-0 text-[12px] text-ink-3">No change yet.</p>
      : (history.data ?? []).map((c) => <Version key={c.id} base={`/api/bots/${bot.id}`} change={c} />)}
  </Section>;
}

export function FileEditor({ value, onSave, rows, mono, tall }: { value: string; onSave: (text: string) => Promise<boolean>; rows: number; mono?: boolean; tall?: boolean }) {
  const [text, setText] = useState(value);
  const [busy, setBusy] = useState(false);
  const ref = useRef<HTMLTextAreaElement>(null);
  // Shows the whole file up to 36 lines, then scrolls.
  useLayoutEffect(() => { const el = ref.current; if (!el) return; el.style.height = "auto"; el.style.height = `${Math.min(el.scrollHeight + 2, 740)}px`; }, [text]);
  const dirty = text !== value;
  return <div className="grid gap-1.5">
    <textarea ref={ref} className={`field-input ${tall ? "min-h-[320px]" : "min-h-[120px]"} resize-none py-1.5 text-[12px] leading-5 ${mono ? "font-mono" : ""}`} rows={rows} value={text} onChange={(e) => setText(e.target.value)} spellCheck={!mono} />
    {dirty && <div className="flex justify-end gap-1.5"><button className="btn btn-sm btn-ghost" onClick={() => setText(value)}>Discard</button><button className="btn btn-sm btn-primary" disabled={busy} onClick={async () => { setBusy(true); await onSave(text); setBusy(false); }}>Save</button></div>}
  </div>;
}

/** One change of a bot's files (`base`: `/api/bots/<id>`): its diff and Restore previous. */
export function Version({ base, change }: { base: string; change: Change }) {
  const qc = useQueryClient();
  const [open, setOpen] = useState(false);
  const diff = useQuery({ queryKey: ["bot-diff", base, change.id], queryFn: () => api.get<{ patch: string }>(`${base}/history/${change.id}`), enabled: open, staleTime: Infinity });
  return <div className="grid min-w-0 grid-cols-[minmax(0,1fr)] gap-1">
    <div className="flex min-w-0 items-center gap-2 text-[12px]">
      <button className="min-w-0 flex-1 truncate text-left hover:underline" aria-expanded={open} onClick={() => setOpen(!open)}>{change.path} · {change.author?.name ?? "You"} · {ago(change.at)}{change.added != null && <span className="text-ink-3"> · +{change.added} −{change.removed ?? 0}</span>}</button>
      {/* A routine file or a memory note that this change created is removed by Restore (the routine goes with it). */}
      {(change.sha256 || /^(routines|memory)\//.test(change.path)) && <span className="flex-none"><ConfirmButton icon="refresh" tone="publish" consequence={change.sha256 ? `Puts back ${change.path} as it was before this change. The restore is itself a version.` : `This change created ${change.path}: Restore removes it${change.path.startsWith("routines/") ? ", and its routine with its runs" : ""}. The restore is itself a version.`} confirmLabel="Restore" onConfirm={async () => { try { await api.post(`${base}/restore/${change.id}`); await Promise.all([qc.invalidateQueries({ queryKey: ["bots"] }), qc.invalidateQueries({ queryKey: ["crons"] })]); toast("Restored"); } catch (e) { toastError(e); } }}>Restore previous</ConfirmButton></span>}
    </div>
    {open && (diff.isLoading ? <Loading what="the diff" /> : diff.data?.patch ? <PatchView patch={diff.data.patch} path={change.path} language={change.path.endsWith(".toml") ? "toml" : "markdown"} split={false} wrap /> : <p className="m-0 text-[12px] text-ink-3">No text difference.</p>)}
  </div>;
}
