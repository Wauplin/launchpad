// Bots: persistent agents with several threads each (parallel conversations), shown in Chats (`features/chats`).
// This file holds a chat with a bot (`BotPage`: one thread at a time, `?thread=`, features/chat/BotChat.tsx), whose
// header names the chat (its automatic title), its bots in the order they joined (a conversation of several bots),
// its state, Archive and × (back to Chats, archiving nothing); a bot's settings (`BotSettingsPage`, a page of the
// Chats frame opened from the gear of its tile or of its name in a chat's header: BOT.md, Triggers, Profile and model,
// Access, Memory, Versions, Edit as file and Delete); and the New bot form (name, job, profile and folder only).
// Starting a chat is the bot's tile in Chats; archived chats are under Archived in the Chats list.

import { useQuery, useQueryClient } from "@tanstack/react-query";
import { ToolDefaultsEditor } from "@/features/settings/ToolDefaults";
import { defaultOn, slackKey, useLayers } from "@/lib/toolDefaults";
import { useSlackProfiles } from "@/lib/slack";
import { copyRefLink } from "@/lib/refActions";
import { Link, useNavigate, useParams, useSearch } from "@tanstack/react-router";
import { Fragment, useEffect, useLayoutEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { Icon, type IconName } from "@/components/Icon";
import { MenuItem } from "@/components/Popover";
import { Markdown } from "@/components/Markdown";
import { toast, toastError } from "@/components/Toast";
import { ConfirmButton, Empty, ErrorState, IconButton, Loading, MoreMenu, PageHeader, RoutineMark, Tabs } from "@/components/ui";
import { PatchView } from "@/components/diff/DiffReview";
import { BotChat, BotModelChoice } from "@/features/chat/BotChat";
import { ArchiveThreadButton, NEW_THREAD, useShownThread, useThreadRows } from "@/features/chat/BotThreads";
import { useChannelName } from "@/lib/channels";
import type { ConnectionsOverview } from "@/features/settings/ConnectionsView";
import { useTelegram } from "@/features/settings/TelegramSettings";
import { BotAvatar } from "@/components/BotAvatar";
import { useHarnessName } from "@/components/HarnessMark";
import { BotIdentity } from "./BotIdentity";
import type { Bot } from "@/gen/Bot";
import type { BotSettings } from "@/gen/BotSettings";
import type { Change } from "@/gen/Change";
import type { PermissionMode } from "@/gen/PermissionMode";
import type { Project } from "@/gen/Project";
import type { Session } from "@/gen/Session";
import type { VaultOverview } from "@/gen/VaultOverview";
import { api } from "@/lib/api";
import { runAction } from "@/lib/actions";
import { ActionButton } from "@/components/ActionButton";
import { askTarget, botProfile, botThread, currentSession, findThread, instructionsUpdated, landingThread, rememberThread, rotationReason, useBots, useBotThreads, useBotTemplates } from "@/lib/bots";
import { chatOf } from "@/lib/chatList";
import { conversationStatus } from "@/lib/conversations";
import { isQuotaPaused, isRunning, isSleeping, sleepUntil } from "@/lib/sessionStatus";
import { daemonUrl } from "@/lib/connection";
import { age, ago, when } from "@/lib/format";
import { usePhone } from "@/lib/phone";
import { useProfiles } from "@/lib/profiles";
import { useRunnerTag } from "@/lib/burst";
import { useCrons, useHarnessDefaults, useProjects, useSessions } from "@/lib/queries";
import { cronTitle, routineState, triggerSummary } from "@/lib/routines";
import { NewRoutine, RoutineDetails, RunStrip } from "@/features/schedules/Routines";
import { useChats } from "@/features/chats/data";
import { Dialog } from "@/features/chats/Dialog";
import { ArchiveAutomationDialog } from "@/features/agents/ArchiveAutomation";
import type { CronAgent } from "@/gen/CronAgent";
import type { ProposalMode } from "@/gen/ProposalMode";
import type { LaunchpadAccess } from "@/gen/LaunchpadAccess";

const CLI_ACCESS: { id: LaunchpadAccess; label: string; detail: string }[] = [
  { id: "read", label: "Read", detail: "reads Launchpad state: folders, agents, schedules, quotas" },
  { id: "propose", label: "Propose", detail: "reads, and writes proposals that wait for your click" },
  { id: "admin", label: "Admin", detail: "also changes Launchpad itself: bots, routines, schedules, workspaces, launches and messages; never GitHub, the Outbox, credentials, the machine or paid usage" },
];
import { useScope } from "@/lib/workspace";
import { PhoneBar } from "@/mobile/PhoneHeader";
import { ChatTitle, PinStar, TitleForm, useTitleEditor } from "@/features/chats/ChatTitle";
import { phoneBack, replaceUnderOverlays } from "@/mobile/back";
import { useKeptDraft, versionOf } from "@/lib/keptDraft";
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

const useBotSessions = () => useSessions().data ?? [];
const folderLabel = (projects: Project[], id: string | null | undefined) => id ? projects.find((p) => p.id === id)?.name ?? "a removed folder" : "a temporary folder";

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
    <li>{folder && !folder.plain ? <>Works in <b>{folder.name}</b>, in a fresh worktree per session; it prepares branches and Local PRs for your review.</> : <>Starts each session in a temporary folder, saved on Home and deleted 30 days after the session is archived. It finds the workspace's folders with the Launchpad CLI and its notes, and works on a Git folder in a fresh worktree that it adds when a task needs it.</>}</li>
    <li>Remembers in the knowledge base of <b>{workspace}</b>: every change records it, and you can restore it.</li>
    <li>{tools.length ? <>Uses the tools granted to {workspace} with Home's tokens: {tools.map((t) => t.name).join(", ")}.</> : <>No tool is granted to {workspace} (Settings → Credentials and tools).</>}</li>
    <li>{!cli ? <>Without the Launchpad CLI tool, it cannot read Launchpad state or write proposals.</>
      : launchpad === "read" ? <>Reads Launchpad state with the Launchpad CLI; it cannot propose.</>
      : <>{launchpad === "admin" ? <><b>Admin</b>: changes Launchpad's bots, routines, schedules and workspaces, and starts agents, with the Launchpad CLI; it also </> : "Reads Launchpad state and "}writes proposals. {proposals === "auto" ? "Auto: one-time starts apply at once; the rest waits for your click." : "Every proposal waits for your click."}</>}</li>
  </ul>;
}

export function NewBot({ first, onCancel }: { first: boolean; onCancel?: () => void }) {
  const qc = useQueryClient();
  const navigate = useNavigate();
  const scope = useScope();
  const profiles = useProfiles();
  const ws = scope.workspace.id || null;
  // Bots work in a Git folder, or in a temporary folder per session: never in a plain folder.
  const folders = scope.projects.filter((p) => !p.is_temp && !p.plain).sort((a, b) => a.name.localeCompare(b.name));
  const [name, setName] = useState("");
  const [job, setJob] = useState("");
  const [profile, setProfile] = useState<string>(profiles.find((p) => p.billing === "subscription")?.id ?? profiles[0]?.id ?? "");
  const [folder, setFolder] = useState<string>("");
  // A template's starters (the chips over the message box), kept while its name is.
  const [starters, setStarters] = useState<{ name: string; list: string[] } | null>(null);
  const [busy, setBusy] = useState(false);
  useEffect(() => { if (!profile && profiles.length) setProfile(profiles.find((p) => p.billing === "subscription")?.id ?? profiles[0].id); }, [profiles, profile]);
  const repo = folders[0];
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
        <p className="m-0 mt-1 text-[13px] text-ink-2">A bot is an agent that stays: chats you come back to, its own instructions, and notes it keeps in the knowledge base between sessions. Describe its job; you can change everything later.</p>
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
            {folders.map((p) => <option key={p.id} value={p.id}>{p.name}</option>)}
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

// ---------------------------------------------------------------- a chat with a bot

/** A chat with a bot: the thread's messages are the page. The header names the chat and its bots; the trace of each
 * session stays in Agents (⋯ of the chat's row in the list). */
export function BotPage() {
  const { id } = useParams({ strict: false }) as { id: string };
  // One page per bot: nothing of the previous bot's page stays when another bot opens.
  return <BotPageOf key={id} id={id} />;
}

/** The bots of a chat in the order they joined, each opening its settings. */
function Participants({ bots, size = 16 }: { bots: Bot[]; size?: number }) {
  return <span className="flex min-w-0 flex-wrap items-center gap-x-1 gap-y-0.5" data-tour="chat.participants" aria-label={`Bots in this chat: ${bots.map((b) => b.settings.name).join(", ")}`}>
    {bots.map((b, i) => <Fragment key={b.id}>
      {i > 0 && <span className="text-ink-3" aria-hidden>·</span>}
      <Link to="/bots/$id/settings" params={{ id: b.id }} className="inline-flex min-h-6 max-w-full min-w-0 items-center gap-1 rounded-control px-0.5 text-ink-2 no-underline hover:bg-sunk hover:text-ink" title={`Settings of ${b.settings.name}`}>
        <BotAvatar name={b.settings.name} id={b.id} size={size} /><span className="truncate">{b.settings.name}</span>
      </Link>
    </Fragment>)}
  </span>;
}

function BotPageOf({ id }: { id: string }) {
  const phone = usePhone();
  const bots = useBots();
  const bot = bots.data?.find((b) => b.id === id);
  const sessions = useBotSessions();
  const search = useSearch({ strict: false }) as { panel?: string; thread?: string; msg?: string };
  const navigate = useNavigate();
  // Older links opened About (`?panel=`): what it held is the bot's settings page now.
  useEffect(() => { if (search.panel) void navigate({ to: "/bots/$id/settings", params: { id }, replace: true }); }, [search.panel, id, navigate]);
  // The thread shown: `?thread=` (a thread ID, or a routing key such as `channel:<id>` from a channel's link), `new`
  // for a new chat, else the thread the owner looked at last (`landingThread`), else a new chat. A key without an
  // open thread lands the same way; an ID not listed (archived, or just created) still shows its session.
  const threads = useBotThreads(id);
  const named = findThread(threads.data, search.thread);
  const landing = useMemo(() => (threads.data ? landingThread(id, threads.data, sessions) : null), [id, threads.data, sessions]);
  const threadId = search.thread === NEW_THREAD ? NEW_THREAD : named?.id ?? (search.thread && !search.thread.includes(":") ? search.thread : landing ?? NEW_THREAD);
  // The thread shown is the one this device opens next time.
  useEffect(() => { if (threadId !== NEW_THREAD && (search.thread || threads.data)) rememberThread(id, threadId); }, [id, threadId]); // eslint-disable-line react-hooks/exhaustive-deps
  const thread = useMemo(() => (threadId === NEW_THREAD ? [] : botThread(sessions, id, threadId)), [sessions, id, threadId]);
  const current = currentSession(thread);
  const rows = useThreadRows(bot ?? ({ id } as Bot), sessions);
  const shownThread = rows.find((r) => r.t.id === threadId)?.t ?? null;
  // The chat in the list's terms: its title and its bots in the order they joined.
  const { chats } = useChats();
  const chat = threadId === NEW_THREAD ? undefined : chatOf(chats, { bot: id, thread: threadId });
  const conversation = shownThread?.conversation ?? (chat?.kind === "thread" ? chat.id : null);
  // The thread shown, so an Archive from the list knows to leave the page.
  useEffect(() => {
    useShownThread.setState({ shown: threadId === NEW_THREAD ? null : { bot: id, thread: threadId, conversation } });
    return () => useShownThread.setState({ shown: null });
  }, [id, threadId, conversation]);
  // Your message went to another thread (a new chat's first message, a chat that continues in a new thread): the page
  // follows it, replacing the history entry.
  const selectThread = (t: string) => {
    const to = `/bots/${id}?thread=${encodeURIComponent(t)}`;
    if (phone && replaceUnderOverlays(to)) return;
    void navigate({ to: "/bots/$id", params: { id }, search: { thread: t } as never, replace: true });
  };
  const routines = (useCrons().data ?? []).filter((c) => c.bot_id === id);
  const profiles = useProfiles();
  const where = useRunnerTag(current?.runner_id);
  const [step, setStep] = useState<string | null>(null);
  const editor = useTitleEditor(id, threadId);
  const run = useRunLabel(bot ?? null, current);
  if (bots.isLoading) return <Loading what="the bot" />;
  if (bots.error) return <ErrorState error={bots.error} onRetry={() => void bots.refetch()} />;
  if (!bot) return <div className="flex h-full flex-col"><PageHeader title="Chat" /><Empty fill icon="spark" action={<Link to="/chats" className="btn btn-sm">All chats</Link>}>This bot no longer exists.</Empty></div>;
  const profile = botProfile(bot, profiles);
  // The thread's session as it is (archived included): its next message may continue in a new thread.
  const rotation = rotationReason(bot, thread.at(-1) ?? null, profile);
  const participants = chat?.kind === "thread" ? chat.participants : [bot];
  const others = chat?.kind === "thread" ? chat.participants.filter((b) => b.id !== bot.id) : [];
  const chatThreads = new Set(chat?.kind === "thread" ? chat.threads : []);
  const otherSessions = others.map((b) => ({ name: b.settings.name, session: currentSession(sessions.filter((s) => s.launch_spec.bot?.id === b.id && chatThreads.has(`${b.id}\n${s.launch_spec.bot?.thread || "main"}`)).sort((a, z) => a.started_at - z.started_at)) }));
  const status = statusOf(bot, thread, current, routines, step, otherSessions);
  const title = threadId === NEW_THREAD ? "New chat" : chat?.title ?? shownThread?.title ?? "Chat";
  const routine = chat?.kind === "thread" ? chat.routine : null;
  const statusLine = <span className={`min-w-0 truncate ${status.tone}`} title={where && where.tone !== "quiet" ? where.title : status.text} data-tour="bots.status">{status.text}{where && where.tone !== "quiet" ? ` · ${where.label}` : ""}</span>;
  // Leave the chat without archiving: the Chats landing (the phone's ‹ goes back to the list).
  const closeChat = () => void navigate({ to: "/chats", search: {} as never });
  const archiveThread = shownThread && threadId !== NEW_THREAD && shownThread.archived_at == null && current ? <ArchiveThreadButton bot={bot} thread={shownThread} sessions={thread} /> : null;
  // A chat with a thread: its title is editable in place, and it can be pinned and opened in Agents from the header.
  const editable = !!shownThread && threadId !== NEW_THREAD;
  const pinTarget = chat?.kind === "thread" ? chat.pin : null;
  const openInAgents = current ? () => void navigate({ to: "/agents", search: { session: current.id } as never }) : null;
  const chatView = <BotChat key={threadId} bot={bot} threadId={threadId} thread={thread} rotation={rotation} needsProfile={!profile} conversationBar={false} target={search.msg}
    openSettings={() => void navigate({ to: "/bots/$id/settings", params: { id } })} onStep={setStep}
    onThread={(t) => { if (t !== threadId) { void threads.refetch(); selectThread(t); } }}
    intro={<p className="m-0 text-[13px] text-ink-2">{threadId !== NEW_THREAD ? "This chat has no message yet." : rows.length ? `Send ${bot.settings.name} the first message of a new chat: it runs next to its other chats.` : `Send ${bot.settings.name} its first task.`}</p>} />;
  if (phone) {
    return <div className="flex h-full min-h-0 flex-col" data-tour="bots.bot">
      <PhoneBar title={<span className="grid min-w-0"><span className="truncate">{title}</span><span className="truncate text-[12px] font-normal text-ink-3">{participants.map((b) => b.settings.name).join(", ")} · {statusLine}{run && <span title={run}> · {run}</span>}</span></span>} onBack={() => phoneBack("/chats")} backLabel="All chats"
        end={<>{editable && pinTarget && <PinStar pin={pinTarget} />}{archiveThread}<IconButton icon="gear" label={`Settings of ${bot.settings.name}`} data-tour="chat.settings" onClick={() => void navigate({ to: "/bots/$id/settings", params: { id } })} />
          {editable && <MoreMenu label="More actions for this chat">{(close) => <>
            <MenuItem icon={<Icon name="pencil" size={14} />} label="Rename" detail={title} onClick={() => { close(); editor.setEditing(true); }} />
            {openInAgents && <MenuItem icon={<Icon name="agents" size={14} />} label="Open in Agents" detail="Its session's trace, terminal and files" onClick={() => { close(); openInAgents(); }} />}
            {editable && <MenuItem icon={<Icon name="link" size={14} />} label="Copy link" detail="Paste it into any message box, or Go to" onClick={() => { close(); void copyRefLink({ kind: "chat", id, thread: threadId, title }); }} />}
          </>}</MoreMenu>}</>} />
      {editable && editor.editing && <div className="flex border-b border-line-2 bg-paper px-3 py-2"><TitleForm bot={id} thread={shownThread!.id} title={title} onDone={() => editor.setEditing(false)} /></div>}
      {chatView}
    </div>;
  }
  return (
    <div className="flex h-full min-h-0 flex-col" data-tour="bots.bot">
      <PageHeader title={editable
          ? <ChatTitle bot={id} thread={shownThread!.id} pin={pinTarget} title={title} mark={routine && <RoutineMark routine={routine} />} editor={editor} />
          : <span className="flex min-w-0 items-center gap-1.5">{routine && <RoutineMark routine={routine} />}<span className="truncate" title={title}>{title}</span></span>}
        meta={<><Participants bots={participants} /><span aria-hidden>·</span>{statusLine}{run && <><span aria-hidden>·</span><span className="min-w-0 truncate" title="Harness, model and effort of this chat's session">{run}</span></>}</>}
        actions={<span className="flex items-center gap-1" data-tour="bots.threads">
          {editable && openInAgents && <button type="button" className="btn btn-sm flex-none" title="Its session's trace, terminal and files" onClick={openInAgents}><Icon name="agents" size={12} />Open in Agents</button>}
          {archiveThread}<IconButton icon="x" label="Close the chat" onClick={closeChat} /></span>} />
      <div className="flex min-h-0 flex-1 flex-col">{chatView}</div>
    </div>
  );
}

/** The harness, model and effort the chat runs on ("Claude · opus · high"): its session's values, else the bot's
 * settings, then its profile's defaults, then the harness's defaults (as Summary → Run shows them). */
function useRunLabel(bot: Bot | null, current: Session | null): string | null {
  const profiles = useProfiles();
  const profile = (current ? profiles.find((p) => p.id === current.launch_spec.profile) : null) ?? (bot ? botProfile(bot, profiles) : null);
  const harness = current?.harness ?? profile?.harness ?? null;
  const defaults = useHarnessDefaults(profile?.models_url ? null : harness).data;
  const harnessName = useHarnessName();
  if (!harness) return null;
  const model = current ? current.model ?? profile?.model ?? defaults?.model : bot?.settings.model ?? profile?.model ?? defaults?.model;
  const effort = profile?.models_url ? null : current?.effort ?? profile?.effort ?? defaults?.effort;
  return [harnessName(harness), model, effort].filter(Boolean).join(" · ");
}

/** The header's status line, in words: what the chat does now (its bots in a conversation of several, most urgent
 * first), else when the bot runs next. A new chat names nothing to do yet. */
function statusOf(bot: Bot, thread: Session[], current: Session | null, routines: CronAgent[], step: string | null, others: { name: string; session: Session | null }[]): { text: string; tone: string } {
  if (others.length) {
    const text = conversationStatus([{ name: bot.settings.name, session: current }, ...others]);
    const all = [current, ...others.map((o) => o.session)];
    return { text, tone: all.some((s) => s?.open_question || (s && isQuotaPaused(s))) ? "text-attn" : all.some((s) => s && isRunning(s)) ? "text-work" : "text-ink-3" };
  }
  if (current?.open_question) return { text: "Waiting for you · 1 question", tone: "text-attn" };
  if (current && isRunning(current)) return { text: `Working${step ? ` · ${step}` : ""} (${age(current.status_since)})`, tone: "text-work" };
  if (current && isQuotaPaused(current)) return { text: "Quota paused · continues when the quota allows", tone: "text-attn" };
  if (current && isSleeping(current)) return { text: `Sleeping · ${sleepUntil(current.sleep!)}`, tone: "text-ink-2" };
  if (current?.status === "error") return { text: current.failure ? `The last turn failed: ${current.failure}` : "The last turn failed · no reason recorded (launchpad logs --session on its runner)", tone: "text-gh-closed" };
  if (current?.status === "interrupted") return { text: "The last turn was interrupted", tone: "text-gh-closed" };
  if (!thread.length) return { text: bot.paused ? "Triggers paused · your messages still go through" : "Your first message starts it", tone: "text-ink-3" };
  if (!current) return { text: "Archived · your next message continues in a new chat", tone: "text-ink-3" };
  const next = routines.filter((r) => r.next_run_at != null && !r.paused).sort((a, b) => a.next_run_at! - b.next_run_at!)[0];
  return { text: next ? `Idle · next: ${next.title} ${when(next.next_run_at!)}` : `Idle · ${ago(current.status_since)}`, tone: "text-ink-3" };
}

// ---------------------------------------------------------------- a bot's settings

/** A bot's settings: everything that defines it, on a page of its own (BOT.md, Triggers, Profile and model, Access,
 * Memory, Versions), plus Edit as file and Delete. */
export function BotSettingsPage() {
  const { id } = useParams({ strict: false }) as { id: string };
  return <BotSettingsOf key={id} id={id} />;
}

function BotSettingsOf({ id }: { id: string }) {
  const phone = usePhone();
  const qc = useQueryClient();
  const navigate = useNavigate();
  const bots = useBots();
  const bot = bots.data?.find((b) => b.id === id);
  const sessions = useBotSessions();
  const all = useMemo(() => botThread(sessions, id), [sessions, id]);
  const current = currentSession(all);
  const routines = (useCrons().data ?? []).filter((c) => c.bot_id === id);
  const rows = useThreadRows(bot ?? ({ id } as Bot), sessions);
  const [file, setFile] = useState<string | null>(null);
  const [archiveAutomation, setArchiveAutomation] = useState(false);
  if (bots.isLoading) return <Loading what="the bot" />;
  if (bots.error) return <ErrorState error={bots.error} onRetry={() => void bots.refetch()} />;
  if (!bot) return <div className="flex h-full flex-col"><PageHeader title="Settings" /><Empty fill icon="spark" action={<Link to="/chats" className="btn btn-sm">All chats</Link>}>This bot no longer exists.</Empty></div>;
  const running = all.some(isRunning);
  const openThread = (key: string) => { const t = rows.find((r) => r.t.key === key); if (t) void navigate({ to: "/bots/$id", params: { id }, search: { thread: t.t.id } as never }); else toast("No chat yet: it starts with the first message"); };
  const newChat = <Link to="/bots/$id" params={{ id }} search={{ thread: NEW_THREAD } as never} className={`btn btn-sm ${phone ? "" : "btn-primary"}`} title={`Start a new chat with ${bot.settings.name}`}><Icon name="comment" size={12} />New chat</Link>;
  const editFile = <button className="btn btn-sm" title="bot.toml and its routine files" onClick={() => setFile("bot.toml")}><Icon name="file" size={12} />Edit as file</button>;
  const meta = [bot.paused ? "Triggers paused" : null, `${Object.keys(bot.routines).length} routine${Object.keys(bot.routines).length === 1 ? "" : "s"}`, `${rows.length} open chat${rows.length === 1 ? "" : "s"}`].filter(Boolean).join(" · ");
  return <div className="flex h-full min-h-0 flex-col" data-tour="bots.settings-page">
    {phone
      ? <PhoneBar title={`${bot.settings.name} · Settings`} onBack={() => phoneBack("/chats")} backLabel="Back" />
      : <PageHeader title={<span className="flex min-w-0 items-center gap-2"><BotAvatar name={bot.settings.name} id={bot.id} size={24} /><span className="truncate">{bot.settings.name}</span></span>} meta={meta}
        actions={<>{editFile}{newChat}<IconButton icon="x" label="Close the settings" onClick={() => void navigate({ to: "/chats", search: {} as never })} /></>} />}
    <div className="min-h-0 flex-1 overflow-x-hidden overflow-y-auto" data-tour="bots.about-panel">
      <div className={`@container grid w-full max-w-page grid-cols-[minmax(0,1fr)] gap-4 py-4 ${phone ? "px-3" : "px-5"}`}>
        {phone && <div className="flex flex-wrap items-center gap-2"><span className="mr-auto text-[12px] text-ink-3">{meta}</span>{editFile}{newChat}</div>}
        {bot.paused && <p className="m-0 rounded-control bg-sunk px-3 py-2 text-[13px] text-ink-2">Paused: no trigger starts {bot.settings.name} until you resume it under Triggers. Your own messages still go through.</p>}
        <div className="grid grid-cols-[minmax(0,1fr)] gap-4 @5xl:grid-cols-2 @5xl:items-start">
          <div className="grid min-w-0 grid-cols-[minmax(0,1fr)] gap-4">
            <AboutSections bot={bot} current={current} routines={routines} onEditFile={setFile} openThread={openThread} threadKeys={new Set(rows.map((r) => r.t.key).filter(Boolean) as string[])} part="main" />
          </div>
          <div className="grid min-w-0 grid-cols-[minmax(0,1fr)] gap-4">
            <Section title="Avatar and color" tour="bots.identity"><BotIdentity key={`${bot.settings.avatar_seed ?? ""}-${bot.settings.color ?? ""}`} bot={bot} /></Section>
            <Section title="Profile and model" tour="bots.model"><BotModelChoice bot={bot} current={current} /></Section>
            <AboutSections bot={bot} current={current} routines={routines} onEditFile={setFile} openThread={openThread} threadKeys={new Set()} part="side" />
            <Section title="Automation threads" tour="bots.archive-automation">
              <p className="m-0 text-[12px] text-ink-3">Finished routine runs of {bot.settings.name} and the threads that channels, DMs, hooks or other agents started, with the chats they handed off to. Preview them, then archive them in one click.</p>
              <button className="btn btn-sm w-fit" onClick={() => setArchiveAutomation(true)}><Icon name="archive" size={12} />Archive automation threads…</button>
            </Section>
            <Section title="Delete" tour="bots.delete">
              <p className="m-0 text-[12px] text-ink-3">Deletes {bot.settings.name}, its memory and its routines. Its sessions stay in Agents.</p>
              {running ? <button className="btn btn-sm w-fit" disabled title="Stop its running chats first"><Icon name="trash" size={12} />Delete bot</button>
                : <span className="w-fit"><ActionButton name="bot.delete" params={{ id }} onDone={async () => { await qc.invalidateQueries({ queryKey: ["bots"] }); void navigate({ to: "/chats", search: {} as never }); }} /></span>}
            </Section>
          </div>
        </div>
      </div>
    </div>
    {file && <FilesDialog bot={bot} routines={routines} path={file} onPath={setFile} onClose={() => setFile(null)} />}
    {archiveAutomation && <ArchiveAutomationDialog workspace={bot.workspace_id} bot={{ id: bot.id, name: bot.settings.name }} onClose={() => setArchiveAutomation(false)} />}
  </div>;
}

/** Edit as file: bot.toml or one of its routine files. Saving checks it; the bot and its routines follow. */
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
      toast(path === "bot.toml" ? "Saved: a new profile, folder, model, effort or permission mode applies to new chats" : "Saved: the routine follows its file");
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

/** A card of the settings page: its title, an action on the right, then its content. */
export function Section({ title, children, action, tour }: { title: ReactNode; children: ReactNode; action?: ReactNode; tour?: string }) {
  return <section className="grid min-w-0 grid-cols-[minmax(0,1fr)] gap-2 rounded-card border border-line bg-paper p-4" data-tour={tour}>
    <div className="flex min-h-7 items-center gap-2"><h2 className="seclabel m-0 min-w-0 flex-1">{title}</h2>{action}</div>
    {children}
  </section>;
}

// ---------------------------------------------------------------- settings sections

/** What defines the bot. `main`: BOT.md and Triggers (the wide column); `side`: Access, Memory and Versions. */
function AboutSections({ bot, current, routines, onEditFile, openThread, threadKeys, part }: { bot: Bot; current: Session | null; routines: CronAgent[]; onEditFile: (path: string) => void; openThread: (key: string) => void; threadKeys: Set<string>; part: "main" | "side" }) {
  const save = useSaveFiles(bot);
  if (part === "side") return <>
    <AccessSection bot={bot} />
    <MemorySection bot={bot} />
    <VersionsSection bot={bot} />
  </>;
  return <>
    {/* Keyed on the file, not the store version: the bot writing its memory must not reset a draft. */}
    <InstructionsEditor key={`i-${bot.instructions}`} bot={bot} older={instructionsUpdated(bot, current)} onSave={(t) => save({ instructions: t })} />
    <TriggersSection bot={bot} routines={routines} onEditFile={onEditFile} openThread={openThread} threadKeys={threadKeys} />
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
  // The draft survives back and leaving the page (kept per bot and BOT.md version) until Save or Discard.
  const draft = useKeptDraft(`bot.${bot.id}.${versionOf(bot.instructions)}`, bot.instructions);
  const { text, setText } = draft;
  const [open, setOpen] = useState(false);
  const [mode, setMode] = useState<"read" | "edit">("read");
  const [busy, setBusy] = useState(false);
  const dirty = text !== bot.instructions;
  const save = async () => { setBusy(true); const ok = await onSave(text); setBusy(false); if (ok) { draft.clear(); setOpen(false); } };
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
    <div ref={preview} className="relative max-h-[240px] cursor-pointer overflow-hidden rounded-control bg-sunk px-3 py-2" title="Open BOT.md" onClick={() => setOpen(true)}>
      {text.trim() ? <Markdown text={text} className="text-[12px]" /> : <span className="text-[12px] text-ink-3">No instructions yet. Edit BOT.md to write what {bot.settings.name} does.</span>}
      {cut && <span className="pointer-events-none absolute inset-x-0 bottom-0 h-10 bg-linear-to-t from-sunk to-transparent" />}
    </div>
    {older && !dirty && <p className="m-0 flex items-center gap-1.5 text-[12px] text-ink-3"><Icon name="file" size={12} className="flex-none" />The current session runs the previous version; your next message updates it.</p>}
    {dirty && <div className="flex items-center justify-end gap-1.5"><span className="mr-auto text-[12px] text-attn">{draft.kept ? "Draft kept: unsaved changes from before" : "Unsaved changes"}</span>{buttons}</div>}
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

const TRIGGER_ICON: Record<CronAgent["trigger"]["kind"], IconName> = { schedule: "clock", session_done: "agents", failure: "alert", channel: "comment" };

/** Every way the bot starts on its own, in one list: its routines (with their last runs), incoming hooks that message
 * it and its Telegram command, with one Pause for all of them. */
function TriggersSection({ bot, routines, onEditFile, openThread, threadKeys }: { bot: Bot; routines: CronAgent[]; onEditFile: (path: string) => void; openThread: (key: string) => void; threadKeys: Set<string> }) {
  const qc = useQueryClient();
  const projects = useProjects().data ?? [];
  const profiles = useProfiles();
  const navigate = useNavigate();
  const connections = useQuery({ queryKey: ["connections"], queryFn: () => api.get<ConnectionsOverview>("/api/connections"), retry: false }).data;
  const hooks = (connections?.hooks ?? []).filter((h) => h.target.kind === "bot" && h.target.id === bot.id);
  const telegram = !!useTelegram(bot.workspace_id).data?.bound;
  const askBot = askTarget(useBots().data ?? [], bot.workspace_id)?.id === bot.id;
  const [adding, setAdding] = useState(false);
  const [open, setOpen] = useState<string | null>(null);
  const harness = profiles.find((p) => p.id === bot.settings.profile)?.harness ?? null;
  const folderName = (fid: string) => projects.find((p) => p.id === fid)?.name ?? "a removed folder";
  const channelName = useChannelName();
  const pause = <ActionButton name="bot.pause" params={{ id: bot.id }} body={{ paused: !bot.paused }} icon={bot.paused ? "play" : "stop"}
    title={bot.paused ? "Routines and incoming hooks start runs again" : "Routines and incoming hooks start nothing until you resume. Your own messages still go through."}
    onDone={() => Promise.all([qc.invalidateQueries({ queryKey: ["bots"] }), qc.invalidateQueries({ queryKey: ["crons"] })]).then(() => toast(bot.paused ? `${bot.settings.name} resumed` : `${bot.settings.name} paused: no trigger starts it`))}>
    <span data-tour="bots.pause">{bot.paused ? "Resume all" : "Pause all"}</span>
  </ActionButton>;
  const row = "flex min-h-9 w-full min-w-0 items-center gap-2 rounded-control px-1.5 py-1 text-left text-[13px] text-ink no-underline hover:bg-sunk";
  return <Section title="Triggers" tour="bots.triggers" action={pause}>
    <div className="grid min-w-0 grid-cols-[minmax(0,1fr)] gap-0.5" data-tour="bots.routines">
      {routines.map((r) => <div key={r.id} className="grid min-w-0 grid-cols-[minmax(0,1fr)] gap-1">
        <button className={row} aria-expanded={open === r.id} title={cronTitle(r)} onClick={() => setOpen(open === r.id ? null : r.id)}>
          <Icon name={TRIGGER_ICON[r.trigger.kind]} size={12} className="flex-none text-ink-3" />
          <span className="min-w-0 flex-1"><span className="block truncate">{r.title}</span><span className="block truncate text-[12px] text-ink-3">{r.file_error ? <span className="text-attn">File error · </span> : null}{triggerSummary(r, folderName, channelName, bot.settings.name)} · {routineState(r, bot.settings.name)}</span></span>
          <RunStrip history={r.history} count={5} />
        </button>
        {open === r.id && <RoutineDetails cron={r} bot={bot} onOpen={(sid) => void navigate({ to: "/agents", search: { session: sid } as never })} onDeleted={() => setOpen(null)} onEditFile={r.file ? () => onEditFile(r.file!) : undefined} />}
        {open === r.id && r.file && threadKeys.has(`routine:${r.file}`) && <button className="btn btn-sm w-fit" title="Each run starts a chat of its own: the newest one" onClick={() => openThread(`routine:${r.file}`)}><Icon name="comment" size={12} />Open its last chat</button>}
      </div>)}
      {hooks.map((h) => <div key={h.id} className="flex min-w-0 items-center gap-1">
        <Link to="/settings/connections" className={`${row} flex-1`} title="Incoming hook: edit or delete it in Settings → Connections">
          <Icon name="down" size={12} className="flex-none text-ink-3" />
          <span className="min-w-0 flex-1"><span className="block truncate">Hook “{h.name}”</span><span className="block truncate text-[12px] text-ink-3">{h.last_at ? `Last call ${ago(h.last_at)}` : "Never called"} · {h.count} message{h.count === 1 ? "" : "s"}</span></span>
        </Link>
        {threadKeys.has(`hook:${h.id}`) && <button className="btn btn-sm flex-none" title="Its messages go to a chat of their own" onClick={() => openThread(`hook:${h.id}`)}>Open its chat</button>}
      </div>)}
      {telegram && <Link to="/settings/workspace" className={row} title="The workspace's Telegram chat: /command followed by a task messages this bot">
        <Icon name="send" size={12} className="flex-none text-ink-3" />
        <span className="min-w-0 flex-1"><span className="block truncate">Telegram /{bot.slug.replace(/-/g, "_")}</span><span className="block truncate text-[12px] text-ink-3">From the workspace's chat{askBot ? " · plain text too" : ""}</span></span>
      </Link>}
      {!routines.length && !hooks.length && !telegram && !adding && <p className="m-0 text-[12px] text-ink-3">No trigger yet: {bot.settings.name} runs only when you message it.</p>}
    </div>
    {adding ? <NewRoutine bot={bot} harness={harness} onDone={() => setAdding(false)} />
      : <button className="btn btn-sm w-fit" data-tour="bots.routine-new" title="A routine: on a schedule, on GitHub activity, when another agent is done, on a Launchpad failure or on a new message in a channel. Incoming hooks are created in Settings → Connections." onClick={() => setAdding(true)}><Icon name="plus" size={12} />New trigger</button>}
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
  const layers = useLayers(bot.workspace_id, s.folder ?? null, s.tools, true);
  const slack = (useSlackProfiles().data ?? []).filter((p) => defaultOn(layers, slackKey(p.handle), p.default_on));
  const tools = [...useGrantedTools(bot.workspace_id).filter((t) => t.id !== "launchpad" && defaultOn(layers, t.id)), ...slack.map((p) => ({ name: `Slack · ${p.name}` }))];
  const access = CLI_ACCESS.find((a) => a.id === (s.launchpad ?? "propose"))!;
  const parts = [
    profile ? profile.name : "no profile",
    s.model ?? profile?.model ?? "default model",
    bot.folder_unused ? "a temporary folder (its plain folder is not used)" : folderLabel(projects, s.folder),
    PERMISSIONS.find((p) => p.id === s.permission_mode)?.label,
    `${access.label.toLowerCase()}${(s.proposals ?? "manual") === "auto" ? " (auto proposals)" : ""}`,
    tools.length ? tools.map((t) => t.name).join(", ") : "no tools",
    `up to ${s.max_threads ?? 16} chats at once`,
    s.recall ? "recall" : "",
  ].filter(Boolean);
  return <Section title="Access" tour="bots.settings" action={!editing && <button className="btn btn-sm" onClick={() => setEditing(true)}><Icon name="pencil" size={12} />Edit</button>}>
    {bot.folder_unused && <p className="m-0 rounded-control bg-attn-soft px-2 py-1 text-[12px] text-attn" data-tour="bots.plain-folder">{bot.folder_unused}</p>}
    {editing ? <SettingsForm bot={bot} onClose={() => setEditing(false)} /> : <>
      <p className="m-0 text-[13px] text-ink-2" title="Profile · model · folder · permission mode · Launchpad CLI access · tools · chats at once">{parts.join(" · ")}</p>
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
  // Bots work in a Git folder or a temporary folder; a plain folder bot.toml still names stays listed, marked unused.
  const folders = projects.filter((p) => p.workspace_id === bot.workspace_id && !p.is_temp && (!p.plain || p.id === bot.settings.folder));
  const dirty = JSON.stringify(s) !== JSON.stringify(bot.settings);
  const toolLayers = useLayers(bot.workspace_id, s.folder ?? null, undefined, true);
  const access = s.launchpad ?? "propose";
  const save = async () => {
    try {
      await runAction<Bot>("bot.update", { id: bot.id }, { settings: s });
      await qc.invalidateQueries({ queryKey: ["bots"] });
      toast(s.profile !== bot.settings.profile || (s.folder ?? null) !== (bot.settings.folder ?? null) || s.model !== bot.settings.model || s.effort !== bot.settings.effort || s.permission_mode !== bot.settings.permission_mode ? "Saved: it applies to new chats; an open chat continues in a new one with its next message" : "Saved");
      onClose();
    } catch (e) { toastError(e); }
  };
  const field = "grid gap-1 text-[12px] font-semibold text-ink-2";
  return <div className="grid gap-2.5">
    <label className={field}>Name<input className="field-input font-normal" maxLength={40} value={s.name} onChange={(e) => setS({ ...s, name: e.target.value })} /></label>
    <div className="grid gap-1">
      <span className={field}>Profiles · first choice first</span>
      {(s.profiles?.length ? s.profiles : s.profile ? [s.profile] : []).map((id, i, list) => <div key={i} className="flex items-center gap-1">
        <select aria-label={`Profile ${i + 1}`} className="field-input min-w-0 flex-1" value={id} onChange={(e) => setS({ ...s, profile: undefined, profiles: list.map((v, n) => n === i ? e.target.value : v), model: undefined, effort: undefined })}>
          {!profiles.some((p) => p.id === id) && <option value={id}>Removed profile</option>}
          {profiles.map((p) => <option key={p.id} value={p.id}>{p.name}</option>)}
        </select>
        <button className="btn btn-sm" aria-label="Move profile up" disabled={!i} onClick={() => { const next = [...list]; [next[i - 1], next[i]] = [next[i], next[i - 1]]; setS({ ...s, profile: undefined, model: undefined, effort: undefined, profiles: next }); }}>↑</button>
        <button className="btn btn-sm" aria-label="Remove profile" onClick={() => setS({ ...s, profile: undefined, model: undefined, effort: undefined, profiles: list.filter((_, n) => n !== i) })}>×</button>
      </div>)}
      <button className="btn btn-sm w-fit" disabled={!fallback} onClick={() => setS({ ...s, profile: undefined, model: undefined, effort: undefined, profiles: [...(s.profiles?.length ? s.profiles : s.profile ? [s.profile] : []), fallback?.id ?? ""] })}>Add profile</button>
      <span className="text-[12px] text-ink-3">Uses the first subscription at 95% or less, then one below 100%, then pay as you go. With no profiles, uses the default.</span>
    </div>
    <label className={field}>Model
      <select className="field-input font-normal" value={s.model ?? ""} onChange={(e) => setS({ ...s, profile: e.target.value ? profile?.id : s.profile, model: e.target.value || undefined })}>
        <option value="">Default of the profile{profile?.model ? ` (${profile.model})` : ""}</option>
        {models.map((m) => <option key={m.id} value={m.id}>{m.display_name || m.id}</option>)}
        {s.model && !models.some((m) => m.id === s.model) && <option value={s.model}>{s.model}</option>}
      </select>
    </label>
    <label className={field}>Folder
      <select className="field-input font-normal" value={s.folder ?? ""} onChange={(e) => setS({ ...s, folder: e.target.value || undefined })}>
        <option value="">None: a temporary folder per session</option>
        {s.folder && !folders.some((p) => p.id === s.folder) && <option value={s.folder}>A removed folder</option>}
        {folders.map((p) => <option key={p.id} value={p.id}>{p.name}{p.plain ? " · plain folder, not used by bots" : ""}</option>)}
      </select>
    </label>
    <label className={field}>Chats at once
      <input className="field-input font-normal" type="number" min={1} max={64} value={s.max_threads ?? 16} onChange={(e) => { const n = Number(e.target.value); setS({ ...s, max_threads: !n || n === 16 ? undefined : Math.min(64, Math.max(1, Math.round(n))) }); }} />
      <span className="font-normal text-ink-3">Automatic messages to another chat wait for a free slot; yours never wait.</span>
    </label>
    <label className={field}>Recall
      <select className="field-input font-normal" value={s.recall ? "on" : "off"} onChange={(e) => setS({ ...s, recall: e.target.value === "on" ? true : undefined })}>
        <option value="off">Off</option>
        <option value="on">On: may search the workspace's past sessions</option>
      </select>
      <span className="font-normal text-ink-3">With `launchpad recall`, when recall is on for Home (Settings → Home and runners). Results are evidence, never an authority.</span>
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
