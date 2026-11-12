import { type QueryClient, useQuery, useQueryClient } from "@tanstack/react-query";
import { Link, useNavigate } from "@tanstack/react-router";
import { memo, useState } from "react";
import type { Proposal } from "@/gen/Proposal";
import type { Session } from "@/gen/Session";
import type { LaunchSpec } from "@/gen/LaunchSpec";
import { HarnessMark } from "@/components/HarnessMark";
import { Icon, type IconName } from "@/components/Icon";
import { toast, toastError } from "@/components/Toast";
import { MenuItem } from "@/components/Popover";
import { Badge, type BadgeTone, ConfirmButton, ErrorState, MenuSection, MoreMenu, RowActions, Token } from "@/components/ui";
import { Markdown } from "@/components/Markdown";
import { api } from "@/lib/api";
import { useProfiles } from "@/lib/profiles";
import { useProjects, useSessions } from "@/lib/queries";
import { useComposer } from "@/stores/composer";
import { useWorkspace } from "@/stores/workspace";
import { useBots } from "@/lib/bots";
import { usePhone } from "@/lib/phone";
import { useChannelName } from "@/lib/channels";
import { browserZone, channelTriggerOf, cronTitle, describeCron, triggerSummary } from "@/lib/routines";
import { PatchView } from "@/features/github/DiffReview";
import { useLayout } from "@/stores/layout";
import { SessionStatus } from "@/features/agents/SessionBits";
import { action as actionOf, runAction, type ActionName } from "@/lib/actions";
import { ActionButton } from "@/components/ActionButton";
import { ACTIONS } from "@/gen/actions";
import { BranchActions, ghParams } from "@/features/today/TodayCards";
import { proposalPurpose, proposalReceipt, REPORT_OUTCOME } from "./proposalState";

export const useProposalList = (enabled = true) => useQuery({ enabled, queryKey: ["proposals"], queryFn: () => api.get<Proposal[]>("/api/proposals"), staleTime: 30_000 });

/** The icon of a proposal's kind: the card's and Today's row. */
export const proposalIcon = (p: Proposal): IconName => p.body.kind === "message" ? "send" : p.body.kind === "schedule" ? "clock" : p.body.kind === "cron" || p.body.kind === "routine" ? "refresh" : p.body.kind === "status" ? "agents" : p.body.kind === "preset" ? "preset" : p.body.kind === "tool" ? "term" : p.body.kind === "bot" ? "spark" : p.body.kind === "bot_instructions" || p.body.kind === "bot_files" ? "file" : p.body.kind === "ready" ? "check" : p.body.kind === "report" ? (p.body.outcome === "needs_you" ? "alert" : "comment") : p.body.kind === "channel" || p.body.kind === "channel_join" ? "comment" : "play";

export const START: Record<Proposal["body"]["kind"], string> = { launch: "Start", message: "Send", schedule: "Schedule", cron: "Create routine", status: "", preset: "Save preset", tool: "Add tool", bot: "Create bot", routine: "Add routine", bot_instructions: "Apply to BOT.md", bot_files: "Apply", channel: "Create channel", channel_join: "Add to channel", ready: "", report: "" };

type Report = Extract<Proposal["body"], { kind: "report" }>;
const TONE: Record<Report["outcome"], BadgeTone> = { needs_you: "attn", merged: "ok", approved: "ok", commented: "neutral", waiting: "neutral", closed: "danger", info: "neutral" };

/** A report: its outcome, GitHub item, author and Markdown. A question takes a reply that goes to the author's
 * thread; any report can be marked read. */
function ReportBody({ p, body, busy, dismiss }: { p: Proposal; body: Report; busy: boolean; dismiss: () => void }) {
  const qc = useQueryClient();
  const [text, setText] = useState("");
  const [sending, setSending] = useState(false);
  const label = REPORT_OUTCOME[body.outcome];
  const tone = TONE[body.outcome];
  const ref = body.github?.match(/^([^/]+\/[^#]+)#(\d+)$/);
  const waiting = p.state === "proposed" || p.state === "failed";
  const question = body.outcome === "needs_you";
  const reply = async () => {
    setSending(true);
    try {
      const next = await runAction<Proposal>("proposal.reply", { id: p.id }, { text });
      qc.setQueryData<Proposal[]>(["proposals"], (old) => old?.map((x) => (x.id === next.id ? next : x)));
      if (next.state === "failed") toastError(new Error(next.error ?? "The reply was not sent"));
      else { setText(""); toast("Reply sent"); }
    } catch (e) { toastError(e); } finally { setSending(false); }
  };
  return (
    <>
      <span className="flex flex-wrap items-center gap-1.5 text-[12px]">
        <Badge tone={tone}>{label}</Badge>
        {ref && <Link to="/github/$owner/$repo/$number" params={ghParams(ref[1], Number(ref[2]))} className="text-teal-ink no-underline hover:underline">{body.github}</Link>}
        {p.session_id && <Link to="/agents" search={{ session: p.session_id }} className="text-teal-ink no-underline hover:underline">Open agent</Link>}
      </span>
      <div className="max-h-[360px] overflow-auto"><Markdown text={body.body} className="text-[13px]" /></div>
      {p.state === "started" && <span className="text-[12px] text-gh-open"><Icon name="check" size={12} /> Answered</span>}
      {p.state === "dismissed" && <span className="text-[12px] text-ink-3">Read</span>}
      {waiting && question && <textarea aria-label="Reply" placeholder="Reply to the agent…" className="field-input min-h-16" disabled={sending} value={text} onChange={(e) => setText(e.target.value)} />}
      {waiting && (question
        ? <div className="flex flex-wrap items-center gap-1.5">
            <button className="btn btn-sm btn-primary" disabled={sending || busy || !text.trim()} onClick={() => void reply()}><Icon name="send" size={12} />{sending ? "Sending…" : "Reply"}</button>
            <MoreMenu label="More actions">{(close) => <MenuItem icon={<Icon name="check" size={14} />} label="Mark read" disabled={busy} onClick={() => { close(); dismiss(); }} />}</MoreMenu>
          </div>
        : <button className="btn btn-sm w-fit" disabled={busy} onClick={dismiss}><Icon name="check" size={12} />Mark read</button>)}
    </>
  );
}

/** A new BOT.md: the bot and the diff. Applying fails when BOT.md changed since. */
function BotInstructionsBody({ body }: { body: Extract<Proposal["body"], { kind: "bot_instructions" }> }) {
  const bots = useBots().data ?? [];
  return (
    <>
      <span className="flex flex-wrap gap-1"><Token icon="file">BOT.md of {bots.find((b) => b.id === body.bot_id)?.settings.name ?? "a bot"}</Token></span>
      <div className="max-h-[320px] overflow-auto rounded-control border border-line"><PatchView patch={body.patch} path="BOT.md" language="markdown" split={false} wrap /></div>
    </>
  );
}

/** Changes to a bot's files (BOT.md, routines/<name>.toml): one diff per file. Applying writes them all at once and
 * fails when one changed since. */
function BotFilesBody({ body }: { body: Extract<Proposal["body"], { kind: "bot_files" }> }) {
  const bots = useBots().data ?? [];
  const channelName = useChannelName();
  const paths = Object.keys(body.files).sort((a, b) => (a === "BOT.md" ? -1 : b === "BOT.md" ? 1 : a.localeCompare(b)));
  const botName = bots.find((b) => b.id === body.bot_id)?.settings.name;
  // A channel trigger's file names the channel by its ID: say which one.
  const channelTriggers = paths.flatMap((path) => { const t = channelTriggerOf(body.files[path]); return t ? [{ path, t }] : []; });
  return (
    <>
      <span className="flex flex-wrap gap-1"><Token icon="file">Files of {botName ?? "a bot"} · {paths.length}</Token>
        {channelTriggers.map(({ path, t }) => <Token key={path} icon="comment" title={path}>{triggerSummary({ trigger: t, schedule: "" }, undefined, channelName, botName)}</Token>)}
      </span>
      {paths.map((path) => <div key={path} className="grid gap-1">
        <span className="flex items-center gap-1.5 font-mono text-[12px] text-ink-2">{path}<span className="font-sans text-ink-3">{body.files[path] == null ? "· deleted" : body.before[path] == null ? "· new" : ""}</span></span>
        <div className="max-h-[320px] overflow-auto rounded-control border border-line"><PatchView patch={body.patches[path] ?? ""} path={path} language={path.endsWith(".toml") ? "toml" : "markdown"} split={false} wrap /></div>
      </div>)}
    </>
  );
}

/** A routine card: the bot, when it runs and what it does at each run. Adding it starts nothing now. */
function RoutineBody({ body }: { body: Extract<Proposal["body"], { kind: "routine" }> }) {
  const bots = useBots().data ?? [];
  const channelName = useChannelName();
  const projects = useProjects().data ?? [];
  return (
    <>
      <span className="flex flex-wrap gap-1">
        <Token icon="refresh">Routine of {bots.find((b) => b.id === body.bot_id)?.settings.name ?? "a bot"}</Token>
        <Token title={cronTitle(body)}>{triggerSummary({ ...body, time_zone: browserZone() }, (id) => projects.find((p) => p.id === id)?.name ?? "a folder", channelName, bots.find((b) => b.id === body.bot_id)?.settings.name)}</Token>
      </span>
      <p className="m-0 line-clamp-4 text-[12px] whitespace-pre-wrap text-ink-2">{body.instructions}</p>
    </>
  );
}

/** A bot card: its name, profile, folder and instructions (BOT.md). Creating it starts nothing. */
function BotBody({ body }: { body: Extract<Proposal["body"], { kind: "bot" }> }) {
  const profiles = useProfiles();
  const projects = useProjects().data ?? [];
  return (
    <>
      <span className="flex flex-wrap gap-1">
        <Token icon="spark">New bot · {body.name}</Token>
        <Token>{profiles.find((p) => p.id === body.profile)?.name ?? body.profile ?? "no profile"}</Token>
        <Token icon="folder">{body.folder ? projects.find((p) => p.id === body.folder)?.name ?? "a folder" : "its own new folder"}</Token>
      </span>
      <p className="m-0 line-clamp-4 text-[12px] whitespace-pre-wrap text-ink-2">{body.instructions}</p>
    </>
  );
}

/** A preset card: new or updated label, availability and text. */
function PresetBody({ body }: { body: Extract<Proposal["body"], { kind: "preset" }> }) {
  return (
    <>
      <span className="flex flex-wrap gap-1">
        <Token icon="preset">{body.preset_id ? "Update preset" : "New preset"} · {body.label}</Token>
        <Token>{body.agent ? "agent composer / steering" : "not in agent composer"}</Token>
      </span>
      <p className="m-0 line-clamp-3 text-[12px] whitespace-pre-wrap text-ink-2">{body.text}</p>
    </>
  );
}

/** A tool card: what runners install, the variables agents get and the hosts that receive the token. */
function ToolBody({ body }: { body: Extract<Proposal["body"], { kind: "tool" }> }) {
  const t = body.tool;
  return (
    <span className="flex flex-wrap gap-1">
      <Token icon="term">{t.name}</Token>
      {t.install && <Token>installs {t.install.kind === "github_release" ? `GitHub release ${t.install.source}` : `${t.install.kind} ${t.install.source}`}</Token>}
      {t.env.length > 0 && <Token>{t.env.join(", ")}</Token>}
      {Object.keys(t.vars ?? {}).length > 0 && <Token>settings {Object.entries(t.vars).map(([k, v]) => `${k}=${v}`).join(", ")}</Token>}
      {t.hosts.length > 0 && <Token tone="warn">token sent to {t.hosts.map((h) => h.host).join(", ")}</Token>}
      {body.token_url && <a className="text-[12px] text-teal-ink hover:underline" href={body.token_url} target="_blank" rel="noreferrer">Create the token</a>}
    </span>
  );
}

/** A new channel: its name, purpose and member bots with their modes. Creating it starts nothing. */
function ChannelBody({ body }: { body: Extract<Proposal["body"], { kind: "channel" }> }) {
  return (
    <>
      <span className="flex flex-wrap gap-1">
        <Token icon="comment">New channel · #{body.name}</Token>
        {body.members.map((m) => <Token key={m.bot}>{m.name || "a bot"} · {m.mode}</Token>)}
        {body.members.length === 0 && <Token>no bots yet</Token>}
      </span>
      {body.purpose && <p className="m-0 line-clamp-2 text-[12px] text-ink-2">{body.purpose}</p>}
    </>
  );
}

/** A bot asks to join a channel: the bot, the channel and the mode it asks for. */
function ChannelJoinBody({ body }: { body: Extract<Proposal["body"], { kind: "channel_join" }> }) {
  return (
    <span className="flex flex-wrap items-center gap-1">
      <Token icon="spark">{body.bot_name || "a bot"}</Token>
      <Link to="/channels/$id" params={{ id: body.channel }} className="text-[12px] text-teal-ink no-underline hover:underline">#{body.channel_name}</Link>
      <Token>mode {body.mode}</Token>
    </span>
  );
}

type Ready = Extract<Proposal["body"], { kind: "ready" }>;

/** Where the owner checks what a ready click acts on. */
function readyLink(body: Ready): { to: string; params?: Record<string, string>; search?: Record<string, string>; label: string } | null {
  const id = body.args[0] ?? "";
  switch (body.action) {
    case "local-pr.publish": case "local-pr.merge": return { to: "/predrafts/$id", params: { id }, label: "Review the Local PR" };
    case "outbox.send": return { to: "/", search: { filter: "send" }, label: "Open in Today" };
    case "suggestion.post": return { to: "/", search: { filter: "github" }, label: "Open in Today" };
    case "inbox.done": return { to: "/inbox", label: "Open the Inbox" };
    case "home.update": return { to: "/settings/runners", label: "Open Home and runners" };
    case "github.run-ci": return { to: "/github/$owner/$repo/$number", params: { owner: body.args[0] ?? "", repo: body.args[1] ?? "", number: body.args[2] ?? "" }, label: "Open the pull request" };
    default: return body.session_id ? { to: "/agents", search: { session: body.session_id, view: "changes" }, label: "Review changes" } : null;
  }
}

/** A ready click: an owner action an agent prepared. The owner's click runs the action's own call with its
 * confirmation, as on its page (a merge shows the branch and Merge only when safe); then the card is started. */
/** After the owner's click ran a ready card's action: the card is started. */
function useRan(p: Proposal) {
  const qc = useQueryClient();
  return async () => {
    const next = await api.post<Proposal>(`/api/proposals/${p.id}/ran`);
    qc.setQueryData<Proposal[]>(["proposals"], (old) => old?.map((x) => (x.id === next.id ? next : x)));
    void qc.invalidateQueries({ queryKey: ["today"] });
  };
}

/** A ready card's action and its route parameters (from `args`, in the route's order). */
function readyCall(body: Ready): { name: ActionName; params: Record<string, string> } | null {
  if (!(body.action in ACTIONS)) return null;
  const name = body.action as ActionName;
  const route = actionOf(name).route.match(/\{(\w+)\}/g)?.map((x) => x.slice(1, -1)) ?? [];
  return { name, params: Object.fromEntries(route.map((k, i) => [k, body.args[i] ?? ""])) };
}

const READY_VARS = { base: "the base branch", branch: "the branch", connection: "its connection", version: "the newest version" };

function ReadyBody({ p, body, dismiss }: { p: Proposal; body: Ready; dismiss: () => void }) {
  const navigate = useNavigate();
  const session = useSessions().data?.find((s) => s.id === body.session_id) ?? null;
  const waiting = p.state === "proposed" || p.state === "failed";
  const ran = useRan(p);
  const call = readyCall(body);
  if (!call) return <ErrorState error={`Unknown action ${body.action}`} />;
  const { name, params } = call;
  const a = actionOf(name);
  const link = readyLink(body);
  // The action is the one visible button; the page link and Dismiss go in ⋯.
  const more = <MoreMenu label="More actions">{(close) => <>
    {link && <MenuItem icon={<Icon name="ext" size={14} />} label={link.label} onClick={() => { close(); void navigate({ to: link.to as never, params: link.params as never, search: link.search as never }); }} />}
    <MenuItem icon={<Icon name="x" size={14} />} label="Dismiss" onClick={() => { close(); dismiss(); }} />
  </>}</MoreMenu>;
  return (
    <>
      <span className="flex flex-wrap gap-1"><Token icon="alert">Your click · {a.help}</Token></span>
      {Object.keys(body.input).length > 0 && <span className="text-[12px] text-ink-2 [overflow-wrap:anywhere]">{Object.entries(body.input).map(([k, v]) => `${k.replace(/_/g, " ")}: ${typeof v === "string" ? v : JSON.stringify(v)}`).join(" · ")}</span>}
      {waiting && (name === "worktree.merge" && body.session_id
        ? <BranchActions sessionId={body.session_id} s={session} head={typeof body.input.head === "string" ? body.input.head : undefined} onMerged={ran}>{more}</BranchActions>
        : <div className="flex flex-wrap items-center gap-1.5">
            <ActionButton name={name} params={params} body={body.input} primary vars={{ ...READY_VARS, ...params }} onDone={() => ran().catch(toastError)} />
            {more}
          </div>)}
    </>
  );
}

function specOf(p: Proposal): LaunchSpec | null {
  return p.body.kind === "launch" || p.body.kind === "schedule" || p.body.kind === "cron" ? p.body.spec : null;
}

/** Profile, then only values that differ from defaults, then warnings, like the composer's option line. */
function SpecTokens({ spec }: { spec: LaunchSpec }) {
  const profiles = useProfiles();
  const projects = useProjects().data ?? [];
  const profile = profiles.find((x) => x.id === spec.profile);
  const folder = spec.target.kind === "project" || spec.target.kind === "predraft" ? projects.find((x) => x.id === (spec.target as { project_id: string }).project_id)?.name : spec.target.kind === "temp" ? "temporary folder" : null;
  return (
    <span className="flex flex-wrap gap-1">
      {folder && <Token icon="folder">{folder}</Token>}
      <Token>{spec.harness && <HarnessMark harness={spec.harness} size={11} />}{profile?.name ?? spec.profile ?? "no profile"}</Token>
      {spec.model && spec.model !== profile?.model && <Token>{spec.model}</Token>}
      {spec.target.kind === "project" && !spec.worktree && <Token>checkout</Token>}
      {spec.worktree?.when_done === "merge" && <Token>merge when done</Token>}
      {spec.worktree?.when_done === "local_pr" && <Token>Local PR when done</Token>}
      {spec.worktree?.when_done === "read_only" && <Token>read only</Token>}
      {spec.github_refs.length > 0 && <Token icon="pr">{spec.github_refs.length} GitHub item{spec.github_refs.length === 1 ? "" : "s"}</Token>}
      {(spec.agent_traces ?? []).length > 0 && <Token icon="agents">+ {spec.agent_traces.length} trace</Token>}
      {profile?.billing === "pay_as_you_go" && <Token tone="warn">pay as you go</Token>}
      {spec.budget_usd != null && <Token tone="warn">limit ${spec.budget_usd.toFixed(2)}</Token>}
    </span>
  );
}

const PROPOSAL_ACTIONS = { accept: "proposal.start", dismiss: "proposal.dismiss", recheck: "proposal.recheck" } as const;
/** Starts, dismisses or rechecks a card. A routine or cron keeps the time zone of the browser that accepts it. */
export const runProposalAction = <T = unknown,>(id: string, action: keyof typeof PROPOSAL_ACTIONS) =>
  runAction<T>(PROPOSAL_ACTIONS[action], { id }, undefined, action === "accept" ? { time_zone: browserZone() } : undefined);

/** Replaces the card (so it cannot start twice), then puts its spec into the composer draft. */
async function editInComposer(p: Proposal, qc: QueryClient) {
  const spec = specOf(p);
  if (!spec) return;
  try {
    const next = await api.post<Proposal>(`/api/proposals/${p.id}/edit`);
    qc.setQueryData<Proposal[]>(["proposals"], (old) => old?.map((x) => (x.id === next.id ? next : x)));
  } catch (e) { toastError(e); return; }
  const c = useComposer.getState();
  useComposer.setState({ open: true, minimized: false });
  c.update((s) => ({ ...s, ...spec }));
  if (p.body.kind === "schedule") c.setConditions(p.body.conditions);
  if (p.body.kind === "cron") toast(`Pick Repeat on a schedule… in Launch ▾ (cron ${p.body.schedule})`);
}

/** Edit in the composer, from the card's ⋯ (last: replacing a draft asks first). */
function EditMenuItem({ p, close }: { p: Proposal; close: () => void }) {
  const qc = useQueryClient();
  const hasDraft = useComposer((s) => s.open && (!!s.spec.text.trim() || s.spec.github_refs.length > 0));
  if (!specOf(p)) return null;
  if (hasDraft) return <MenuSection><ConfirmButton tone="publish" icon="pencil" consequence="Replace the current draft? This card is then replaced by what you launch." confirmLabel="Replace draft" onConfirm={() => { close(); return editInComposer(p, qc); }}>Edit</ConfirmButton></MenuSection>;
  return <MenuItem icon={<Icon name="pencil" size={14} />} label="Edit" detail="In the composer; this card is then replaced by what you launch" onClick={() => { close(); void editInComposer(p, qc); }} />;
}

const SessionRow = memo(function SessionRow({ s }: { s: Session }) {
  const show = useLayout((x) => x.show);
  const navigate = useNavigate();
  return (
    <button className="flex min-h-10 w-full items-center gap-2 rounded-control px-2 text-left hover:bg-sunk" onClick={() => { show(s.id); void navigate({ to: "/agents", search: { session: s.id } }); }}>
      <span className="min-w-0 flex-1"><span className="block truncate text-[13px] font-semibold">{s.title}</span>{s.last_message && <span className="block truncate text-[12px] text-ink-3">{s.last_message}</span>}</span>
      <SessionStatus s={s} />
    </button>
  );
});

/** Start, Dismiss or Recheck a card, with its busy state; the card list takes the new state at once. */
function useProposalAct(p: Proposal, onAccepted?: () => void) {
  const qc = useQueryClient();
  const [busy, setBusy] = useState(false);
  const act = async (action: "accept" | "dismiss" | "recheck") => {
    setBusy(true);
    try {
      const next = await runProposalAction<Proposal>(p.id, action);
      qc.setQueryData<Proposal[]>(["proposals"], (old) => old?.map((x) => (x.id === next.id ? next : x)));
      if (next.state === "started" && (action === "accept" || action === "recheck")) {
        await qc.invalidateQueries({ queryKey: ["sessions"] });
        onAccepted?.();
      }
      if (next.state === "failed") toastError(new Error(next.error ?? "It could not start"));
    } catch (e) { toastError(e); } finally { setBusy(false); }
  };
  return { busy, act };
}

export const ProposalCard = memo(function ProposalCard({ p, onAccepted, anchor = false }: { p: Proposal; onAccepted?: () => void; anchor?: boolean }) {
  const sessions = useSessions().data ?? [];
  const { busy, act } = useProposalAct(p, onAccepted);
  const spec = specOf(p);
  const started = p.state === "started";
  const session = started && (p.body.kind === "launch" || p.body.kind === "message") ? sessions.find((s) => s.id === p.result_id) : undefined;
  const statusSessions = p.body.kind === "status" ? p.body.session_ids.map((id) => sessions.find((s) => s.id === id)).filter((s): s is Session => !!s) : [];
  return (
    <article id={anchor ? `proposal-${p.id}` : undefined} data-tour="proposal.card" data-tour-id={p.id} tabIndex={-1} className={`grid min-w-0 grid-cols-[minmax(0,1fr)] gap-2 rounded-card border bg-paper p-3 [overflow-wrap:anywhere] ${(p.body.kind === "cron" || p.body.kind === "routine" || (p.body.kind === "bot_files" && Object.keys(p.body.files).some((f) => f.startsWith("routines/")))) && !started ? "border-attn-line" : "border-line"}`}>
      <div className="flex items-start gap-2">
        <Icon name={proposalIcon(p)} size={14} className="mt-0.5 flex-none text-ink-3" />
        <div className="min-w-0 flex-1">
          <b className="block text-[13px]">{p.title}</b>
          {p.note && <p className="m-0 text-[12px] text-ink-2">{p.note}</p>}
        </div>
      </div>
      {spec && <SpecTokens spec={spec} />}
      {p.body.kind === "cron" && <Token icon="refresh" title={`Cron: ${p.body.schedule}`}>Creates a routine · {describeCron(p.body.schedule)} ({browserZone()})</Token>}
      {p.body.kind === "schedule" && <Token icon="clock">{p.body.conditions.length} start condition{p.body.conditions.length === 1 ? "" : "s"}</Token>}
      {spec && !started && <p className="m-0 line-clamp-3 text-[12px] whitespace-pre-wrap text-ink-2">{spec.text}</p>}
      {p.body.kind === "message" && !started && <p className="m-0 line-clamp-3 text-[12px] whitespace-pre-wrap text-ink-2">To “{sessions.find((s) => s.id === (p.body as { session_id: string }).session_id)?.title ?? "an agent"}”: {p.body.text}</p>}
      {p.body.kind === "preset" && <PresetBody body={p.body} />}
      {p.body.kind === "tool" && <ToolBody body={p.body} />}
      {p.body.kind === "bot" && <BotBody body={p.body} />}
      {p.body.kind === "routine" && <RoutineBody body={p.body} />}
      {p.body.kind === "bot_instructions" && <BotInstructionsBody body={p.body} />}
      {p.body.kind === "bot_files" && <BotFilesBody body={p.body} />}
      {p.body.kind === "channel" && <ChannelBody body={p.body} />}
      {p.body.kind === "channel_join" && <ChannelJoinBody body={p.body} />}
      {p.body.kind === "ready" && <ReadyBody p={p} body={p.body} dismiss={() => void act("dismiss")} />}
      {p.body.kind === "report" && <ReportBody p={p} body={p.body} busy={busy} dismiss={() => void act("dismiss")} />}
      {statusSessions.length > 0 && <div className="grid">{statusSessions.map((s) => <SessionRow key={s.id} s={s} />)}</div>}
      {session && <SessionRow s={session} />}
      {started && !session && p.body.kind !== "status" && p.body.kind !== "report" && <span className="text-[12px] text-gh-open"><Icon name="check" size={12} /> {p.body.kind === "schedule" ? <>Scheduled: see <Link className="underline" to="/schedules" search={{ tab: "launches" }}>Schedules</Link></> : p.body.kind === "bot_instructions" ? <>Applied: see the bot's <Link className="underline" to="/bots/$id" params={{ id: p.result_id ?? "" }} search={{ panel: "about" }}>About</Link></> : p.body.kind === "bot_files" ? <>Applied: see <Link className="underline" to="/bots/$id" params={{ id: p.result_id ?? "" }} search={{ panel: "about" }}>the bot</Link></> : p.body.kind === "cron" || p.body.kind === "routine" ? <>Routine added: see <Link className="underline" to="/schedules" search={{ routine: p.result_id ?? undefined }}>Schedules</Link></> : p.body.kind === "preset" ? "Preset saved: see Settings → Presets" : p.body.kind === "bot" ? <>Bot created. <Link className="underline" to="/bots/$id" params={{ id: p.result_id ?? "" }}>Send it its first task</Link>.</> : p.body.kind === "tool" ? <>Tool added. <Link className="underline" to="/settings/credentials" search={{ tool: p.result_id ?? undefined }}>Paste its token</Link> to use it.</> : p.body.kind === "channel" || p.body.kind === "channel_join" ? <>{p.body.kind === "channel" ? "Channel created" : "Added"}: <Link className="underline" to="/channels/$id" params={{ id: p.result_id ?? "" }}>open the channel</Link></> : p.body.kind === "ready" ? "Done" : "Started"}</span>}
      {p.state === "replaced" && <span className="text-[12px] text-ink-3">{p.replaced_by ? "Replaced" : "Edited in the composer"}{p.replaced_by && <> · <Link to="/" search={{ card: `proposal:${p.replaced_by}` }} onClick={() => { if (p.workspace_id) useWorkspace.getState().switchTo(p.workspace_id); }} className="underline">View replacement</Link></>}</span>}
      {p.state === "dismissed" && p.body.kind !== "report" && <span className="text-[12px] text-ink-3">Dismissed</span>}
      {p.state === "failed" && p.error && <ErrorState error={p.error} />}
      {p.state === "uncertain" && <div className="grid gap-1.5 text-[12px] text-attn">Delivery not confirmed. Check Agents before starting a new attempt.<button className="btn btn-sm w-fit" disabled={busy} onClick={() => void act("recheck")}>I checked Agents — check again</button></div>}
      {(p.state === "proposed" || p.state === "failed") && p.body.kind !== "status" && p.body.kind !== "ready" && p.body.kind !== "report" && (
        <div className="flex flex-wrap items-center gap-1.5">
          <button className="btn btn-sm btn-primary" disabled={busy} onClick={() => void act("accept")}><Icon name="play" size={12} />{busy ? "Starting…" : START[p.body.kind]}</button>
          <MoreMenu label="More actions">{(close) => <>
            <MenuItem icon={<Icon name="x" size={14} />} label="Dismiss" disabled={busy} onClick={() => { close(); void act("dismiss"); }} />
            <EditMenuItem p={p} close={close} />
          </>}</MoreMenu>
        </div>
      )}
      {p.body.kind === "status" && p.state === "proposed" && <button className="btn btn-sm w-fit" onClick={() => void act("dismiss")}>Dismiss</button>}
    </article>
  );
});


/** Where a handled card's receipt leads: the record it made, or its replacement. */
function receiptLink(p: Proposal): { to: string; params?: Record<string, string>; search?: Record<string, string> } | null {
  const id = p.result_id ?? "";
  if (p.state === "replaced") return p.replaced_by ? { to: "/", search: { card: `proposal:${p.replaced_by}` } } : null;
  if (p.state !== "started") return null;
  switch (p.body.kind) {
    case "launch": case "message": return id ? { to: "/agents", search: { session: id } } : null;
    case "schedule": return { to: "/schedules", search: { tab: "launches" } };
    case "cron": case "routine": return id ? { to: "/schedules", search: { routine: id } } : null;
    case "bot": return id ? { to: "/bots/$id", params: { id } } : null;
    case "channel": case "channel_join": return id ? { to: "/channels/$id", params: { id } } : null;
    case "bot_instructions": case "bot_files": return id ? { to: "/bots/$id", params: { id }, search: { panel: "about" } } : null;
    case "tool": return id ? { to: "/settings/credentials", search: { tool: id } } : null;
    default: return null;
  }
}

/** A proposal in a bot's chat: one line (icon, title, its main button, ⋯ with Details, Dismiss and Edit) and a
 * second line saying what it does. Details shows the full card in place. A handled card is a one-line receipt
 * ("Added routine Review requests"). `onReply` answers a question report from the chat's message box. Today and
 * the Proposals list keep the full card. */
export const ProposalLine = memo(function ProposalLine({ p, onReply }: { p: Proposal; onReply?: (p: Proposal) => void }) {
  const [open, setOpen] = useState(p.state === "uncertain");
  const { busy, act } = useProposalAct(p);
  const ran = useRan(p);
  const navigate = useNavigate();
  const bots = useBots().data;
  const projects = useProjects().data;
  const sessions = useSessions().data;
  const phone = usePhone();
  const receipt = proposalReceipt(p);
  if (open) return <div className="grid min-w-0 grid-cols-[minmax(0,1fr)] gap-1">
    <ProposalCard p={p} />
    <button className="chat-tap justify-self-start text-[12px] text-teal-ink hover:underline" onClick={() => setOpen(false)}>Hide details</button>
  </div>;
  if (receipt) {
    const link = receiptLink(p);
    return <div className="flex min-h-7 min-w-0 items-center gap-1.5 py-0.5 text-[12px] text-ink-3" data-tour="proposal.card" data-tour-id={p.id}>
      <Icon name={p.state === "started" ? "check" : p.state === "dismissed" ? "x" : proposalIcon(p)} size={12} className={`flex-none ${p.state === "started" ? "text-gh-open" : ""}`} />
      <span className="min-w-0 truncate" title={p.note ?? undefined}>{link ? <Link to={link.to as never} params={link.params as never} search={link.search as never} className="text-inherit no-underline hover:underline">{receipt}</Link> : receipt}</span>
      <button className="chat-tap flex-none text-teal-ink hover:underline" onClick={() => setOpen(true)}>Details</button>
    </div>;
  }
  const purpose = proposalPurpose(p, {
    folder: (id) => projects?.find((x) => x.id === id)?.name,
    bot: (id) => bots?.find((b) => b.id === id)?.settings.name,
    session: (id) => sessions?.find((s) => s.id === id)?.title,
    action: (name) => { const h = (ACTIONS as Record<string, { help: string }>)[name]?.help; return h && h.charAt(0).toLowerCase() + h.slice(1); },
  });
  const b = p.body;
  const ready = b.kind === "ready" ? readyCall(b) : null;
  const link = b.kind === "ready" ? readyLink(b) : null;
  const question = b.kind === "report" && b.outcome === "needs_you";
  const main = b.kind === "report"
    ? question
      ? <button className="btn btn-sm" disabled={busy} title="Answer from the message box" onClick={() => (onReply ? onReply(p) : setOpen(true))}>{!phone && <Icon name="send" size={12} />}Reply</button>
      : <button className="btn btn-sm" disabled={busy} onClick={() => void act("dismiss")}>{!phone && <Icon name="check" size={12} />}{phone ? "Read" : "Mark read"}</button>
    : b.kind === "status" || (b.kind === "ready" && (!ready || (ready.name === "worktree.merge" && b.session_id)))
      ? <button className="btn btn-sm" onClick={() => setOpen(true)}>{b.kind === "status" ? "Show agents" : "Review"}</button>
      : b.kind === "ready" && ready
        ? <ActionButton name={ready.name} params={ready.params} body={b.input} vars={{ ...READY_VARS, ...ready.params }} icon={phone ? null : undefined} onDone={() => ran().catch(toastError)} />
        : <button className="btn btn-sm" disabled={busy} onClick={() => void act("accept")}>{!phone && <Icon name="play" size={12} />}{busy ? "Starting…" : START[b.kind]}</button>;
  const warn = (b.kind === "cron" || b.kind === "routine" || (b.kind === "bot_files" && Object.keys(b.files).some((f) => f.startsWith("routines/"))));
  return (
    <article data-tour="proposal.card" data-tour-id={p.id} className={`group grid min-w-0 grid-cols-[minmax(0,1fr)] gap-0.5 rounded-card border bg-paper py-1.5 pr-1.5 pl-2.5 ${warn ? "border-attn-line" : "border-line"}`}>
      <div className="flex min-w-0 items-center gap-2">
        <Icon name={proposalIcon(p)} size={14} className="flex-none text-ink-3" />
        <span className="min-w-0 flex-1">
          <b className={`block text-[13px] ${phone ? "line-clamp-2" : "truncate"}`} title={p.title}>{p.title}</b>
          <span className={`block text-[12px] ${phone ? "line-clamp-2" : "truncate"} ${p.state === "failed" ? "text-gh-closed" : "text-ink-3"}`} title={p.note ?? purpose}>{p.state === "failed" && p.error ? `Failed: ${p.error}` : purpose}</span>
        </span>
        <RowActions width={phone ? 0 : undefined} label="More actions" main={main} more={(close) => <>
          <MenuItem icon={<Icon name="expand" size={14} />} label="Details" detail="The full card" onClick={() => { close(); setOpen(true); }} />
          {link && <MenuItem icon={<Icon name="ext" size={14} />} label={link.label} onClick={() => { close(); void navigate({ to: link.to as never, params: link.params as never, search: link.search as never }); }} />}
          {question && <MenuItem icon={<Icon name="check" size={14} />} label="Mark read" disabled={busy} onClick={() => { close(); void act("dismiss"); }} />}
          {b.kind !== "report" && <MenuItem icon={<Icon name="x" size={14} />} label="Dismiss" disabled={busy} onClick={() => { close(); void act("dismiss"); }} />}
          <EditMenuItem p={p} close={close} />
        </>} />
      </div>
    </article>
  );
});
