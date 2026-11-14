// Today's rows: one compact line per decision (icon of its kind, title, one meta line, its buttons). Rows whose
// decision needs reading (a question, a proposal, a message, a suggestion, an agent's last message) open their
// full card below the line with the title or the arrow button. The bot chat keeps the cards (TodayCards.tsx).

import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Link, useNavigate } from "@tanstack/react-router";
import { memo, useEffect, useRef, useState, type ReactNode } from "react";
import type { ChannelPage } from "@/gen/ChannelPage";
import type { Proposal } from "@/gen/Proposal";
import type { Session } from "@/gen/Session";
import type { SessionEventPage } from "@/gen/SessionEventPage";
import type { Today } from "@/gen/Today";
import type { Suggestion } from "@/gen/Suggestion";
import { ActionButton, ActionMenuItem } from "@/components/ActionButton";
import { useHarnessInfo } from "@/components/HarnessMark";
import { Markdown } from "@/components/Markdown";
import { Icon, type IconName } from "@/components/Icon";
import { toast, toastError } from "@/components/Toast";
import { MenuItem } from "@/components/Popover";
import { ErrorState, Loading, RowActions } from "@/components/ui";
import { AskCard } from "@/features/agents/AskCard";
import { ChannelComposer } from "@/features/channels/ChannelComposer";
import { MessageBox } from "@/features/agents/MessageBox";
import { useSessionWorktrees } from "@/features/agents/Worktrees";
import { MessageCard, useOutboxMessages } from "@/features/outbox/MessageCard";
import { ProposalCard, proposalIcon, runProposalAction, START } from "@/features/proposals/ProposalCard";
import { SuggestionCard } from "@/features/suggestions/Suggestions";
import { runAction, type ActionName } from "@/lib/actions";
import { api } from "@/lib/api";
import { ago } from "@/lib/format";
import { suggestionItemUrl } from "@/lib/githubLinks";
import { openExternal } from "@/lib/platform";
import { usePhone } from "@/lib/phone";
import { useSession } from "@/lib/queries";
import type { TodayItem } from "./data";
import { CliUpdateDetails } from "@/features/settings/CliUpdates";
import { FixMergeButton, MergeButton, NotMerged, SessionMeta, useBranch, useOpenQuestion } from "./TodayCards";

/** The icon, color and name of each kind: GitHub's icons for pull requests. */
export const LOOK: Record<TodayItem["kind"], { icon: IconName; color: string; name: string }> = {
  question: { icon: "alert", color: "text-attn", name: "An agent asks you a question" },
  finished: { icon: "okcircle", color: "text-gh-open", name: "Finished agent" },
  failed: { icon: "xcircle", color: "text-gh-closed", name: "Failed or interrupted agent" },
  local_pr: { icon: "pr-local", color: "text-teal", name: "Local PR to publish" },
  proposal: { icon: "spark", color: "text-work", name: "Proposal" },
  outbox: { icon: "send", color: "text-work", name: "Message to send" },
  suggestion: { icon: "comment", color: "text-teal", name: "Suggestion to post on GitHub" },
  routine: { icon: "refresh", color: "text-gh-closed", name: "Routine whose last run failed" },
  runner: { icon: "window", color: "text-attn", name: "Runner to update" },
  home_update: { icon: "download", color: "text-attn", name: "Update for Home" },
  cli_update: { icon: "download", color: "text-teal", name: "Agent CLI update" },
  channel_ask: { icon: "comment", color: "text-attn", name: "A bot asks you in a channel" },
};

type RowProps = { item: TodayItem; expanded: boolean; toggle: (id: string) => void; highlight: boolean };
type Menu = ((close: () => void) => ReactNode) | false | null | undefined;

/** The line: icon, title (opens the details when there are some, else `to`), meta, then the action group: `main`
 * (one outline button in a fixed column, followed by `second` on a laptop: Archive for agents) and ⋯ with the other
 * actions (`menu`). On the phone the line keeps only
 * `main`, a one-word outline button of one minimum width without its icon (the column never moves), and the title
 * up to two lines: a tap on the title or meta opens the details with the other actions. */
function Row({ item, expanded, toggle, highlight, icon, meta, age = true, to, main, second, menu, details, card = false }: RowProps & { icon?: IconName; meta?: ReactNode; age?: boolean; to?: ReactNode; main?: ReactNode; second?: ReactNode; menu?: Menu; details?: ReactNode; card?: boolean }) {
  // `card`: the details are a full card with its own buttons, so the line hides its buttons while it is open.
  const buttons = !(card && expanded && details);
  const phone = usePhone();
  const opens = !!details || (phone && !!menu);
  const look = LOOK[item.kind];
  const title = <span className={`block text-[13px] font-semibold ${phone ? "line-clamp-2 break-words" : "truncate"}`} title={item.title}>{item.title}</span>;
  const metaLine = <div className="flex min-w-0 items-center gap-x-1.5 text-[12px] text-ink-3">{meta ?? <span className="truncate">{item.detail}</span>}{age && <span className="flex-none whitespace-nowrap">· {ago(item.at)}</span>}</div>;
  // The agent's Local PRs: links, so on the phone they sit under the line's button, not inside it.
  const grouped = item.grouped.map((g) => <LocalPrLine key={g.id} item={g} phone={phone} />);
  const kind = <span className={`grid size-6 flex-none place-items-center ${look.color}`} title={look.name} aria-label={look.name} role="img"><Icon name={icon ?? look.icon} size={16} /></span>;
  const chev = (size: number) => <Icon name="chev" size={size} className={`flex-none text-ink-3 transition-transform ${expanded ? "rotate-180" : ""}`} />;
  const label = expanded ? "Hide the details" : "Show the details";
  if (phone) return (
    <article data-tour="today.card" data-tour-id={item.id} className={`border-b border-line-2 ${highlight ? "bg-select" : expanded ? "bg-chrome" : ""}`}>
      <div className="flex min-h-[60px] min-w-0 flex-wrap items-center gap-y-1 py-1.5 pr-3">
        {opens
          ? <button type="button" className="flex min-h-11 min-w-0 flex-[1_1_160px] items-center gap-2.5 pl-3 text-left" aria-expanded={expanded} aria-label={`${item.title}: ${label.toLowerCase()}`} onClick={() => toggle(item.id)}>
            {kind}<span className="min-w-0 flex-1">{title}{metaLine}</span>
          </button>
          : <div className="flex min-w-0 flex-[1_1_160px] items-center gap-2.5 pl-3">{kind}<div className="min-w-0 flex-1">{to ?? title}{metaLine}</div></div>}
        {/* An armed confirmation (Merge) wraps under the title instead of pushing the row sideways. */}
        {buttons && main && <div className="today-main ml-auto flex max-w-full min-w-[88px] flex-none items-center justify-end pl-2">{main}</div>}
        {grouped.length > 0 && <div className="w-full min-w-0 pl-[46px]">{grouped}</div>}
      </div>
      {expanded && opens && <div className="grid min-w-0 grid-cols-[minmax(0,1fr)] gap-2 px-3 pb-3">
        {details}
        {buttons && menu && <div className="grid gap-px rounded-card border border-line bg-paper p-1">{menu(() => toggle(item.id))}</div>}
      </div>}
    </article>
  );
  return (
    <article data-tour="today.card" data-tour-id={item.id} className={`group border-b border-line-2 ${highlight ? "bg-select" : expanded ? "bg-chrome" : "hover:bg-chrome"}`}>
      <div className="flex min-w-0 flex-wrap items-center gap-x-3 gap-y-1.5 px-3 py-2">
        <div className="flex min-w-0 flex-[1_1_300px] items-center gap-2.5">
          {kind}
          <div className="min-w-0 flex-1">
            {details
              ? <button type="button" className="flex max-w-full items-center gap-1 text-left hover:underline" title={label} aria-expanded={expanded} onClick={() => toggle(item.id)}>{title}{chev(11)}</button>
              : to ?? title}
            {metaLine}
            {grouped}
          </div>
        </div>
        <span className="ml-auto max-w-full">
          <RowActions label={`More actions for ${item.title}`} menuWidth={260} main={buttons && (second ? <span className="flex items-center gap-1.5">{main}{second}</span> : main)} more={buttons && menu} />
        </span>
      </div>
      {expanded && opens && <div className="grid min-w-0 grid-cols-[minmax(0,1fr)] gap-2 pr-3 pb-3 pl-[46px]">{details}</div>}
    </article>
  );
}

/** A Local PR of the row's agent (`TodayItem.grouped`): one line under the meta line that opens its page (44 px
 * high on the phone, a touch target). */
const LocalPrLine = ({ item, phone }: { item: TodayItem; phone: boolean }) => (
  <Link className={`flex min-w-0 items-center gap-1.5 text-[12px] text-teal no-underline hover:underline ${phone ? "min-h-11" : ""}`} to="/predrafts/$id" params={{ id: item.record }} title={`${item.title}: publish to GitHub or merge locally from its page`}>
    <Icon name="pr-local" size={12} className="flex-none" /><span className="truncate">{item.detail}</span><span className="flex-none whitespace-nowrap text-ink-3">· {ago(item.at)}</span>
  </Link>
);

/** Review: opens the page of a Local PR (publish to GitHub or merge locally there). */
const ReviewLocalPr = ({ id }: { id: string }) =>
  <Link className="btn btn-sm no-underline" to="/predrafts/$id" params={{ id }} title="Publish to GitHub or merge locally from its page">Local PR</Link>;

/** The main button of a row whose decision needs reading first: it opens the card. */
const Expand = ({ item, expanded, toggle, children }: Pick<RowProps, "item" | "expanded" | "toggle"> & { children: ReactNode }) =>
  expanded ? null : <button type="button" className="btn btn-sm" aria-expanded={false} onClick={() => toggle(item.id)}>{children}</button>;

/** Opens a page from a ⋯ menu. */
function Go({ close, label, icon = "ext", detail, go }: { close: () => void; label: ReactNode; icon?: IconName; detail?: ReactNode; go: (navigate: ReturnType<typeof useNavigate>) => unknown }) {
  const navigate = useNavigate();
  return <MenuItem icon={<Icon name={icon} size={14} />} label={label} detail={detail} onClick={() => { close(); void go(navigate); }} />;
}

const OpenAgent = ({ id, close }: { id: string; close: () => void }) => {
  const bot = useSession(id)?.launch_spec.bot;
  return <Go close={close} icon="agents" label={bot ? "Open in Agents" : "Open agent"} detail={bot ? "The full trace of this turn" : undefined} go={(n) => n({ to: "/agents", search: { session: id } })} />;
};

/** A bot's session opens the bot's chat (bots talk there; the trace stays in Agents). */
const OpenChat = ({ bot, close }: { bot: { id: string; name: string; thread?: string }; close?: () => void }) => {
  const phone = usePhone();
  // The session's thread of the bot (main needs no `?thread=`).
  const search = bot.thread && bot.thread !== "main" ? { thread: bot.thread } : {};
  return close ? <Go close={close} icon="spark" label={`Open the chat of ${bot.name}`} go={(n) => n({ to: "/bots/$id", params: { id: bot.id }, search } as never)} />
    : <Link className="btn btn-sm no-underline" to="/bots/$id" params={{ id: bot.id }} search={search as never} title={`Open the chat of ${bot.name}`}>{phone ? "Open" : "Open chat"}</Link>;
};

/** Open agent (unless it is the row's main button), Archive (unless it is the row's second button) and Mark seen: the
 * ⋯ of a finished or failed agent. */
const SessionMenu = ({ id, open = true, archive = true, reply }: { id: string; open?: boolean; archive?: boolean; reply?: () => void }) => (close: () => void) => <>
  {reply && <MenuItem icon={<Icon name="send" size={14} />} label="Reply" detail="Message the agent from here: it leaves Today until it finishes again" onClick={() => { close(); reply(); }} />}
  {open && <OpenAgent id={id} close={close} />}
  {archive && <ActionMenuItem name="session.archive" params={{ id }} close={close} detail="It stays resumable" />}
  <ActionMenuItem name="session.seen" params={{ id }} close={close} detail="Keep the agent and remove it from Today" />
</>;

/** Archive next to the main button of an agent's line, on a laptop (on the phone it stays in the details' menu). */
const Archive = ({ id }: { id: string }) => <ActionButton name="session.archive" params={{ id }} title="Archive the agent: it stays resumable" />;

/** The agent's last message: its preview (cut at 160 characters, ending with "…"), with an arrow after the "…" that
 * shows the full message (the last assistant message of the stored log, as Markdown) and an arrow that folds it back. */
function Last({ text, id, seq }: { text: string | null | undefined; id?: string; seq?: number }) {
  const [full, setFull] = useState(false);
  const phone = usePhone();
  const cut = !!id && !!text?.endsWith("…");
  const q = useQuery({
    queryKey: ["last-message", id, seq],
    // The newest event, with the facts of the ones before it: one of the two is the last assistant message.
    queryFn: async () => {
      const page = await api.get<SessionEventPage>(`/api/sessions/${id}/events/page?tail=1`);
      const ev = page.events.findLast((e) => e.body.kind === "assistant_message") ?? page.facts?.last_assistant;
      return ev?.body.kind === "assistant_message" ? ev.body.text : null;
    },
    enabled: cut && full,
    staleTime: Infinity,
  });
  if (!text) return null;
  const arrow = (open: boolean) => <button type="button" className={`ml-1 inline-flex ${phone ? "size-11" : "size-5"} items-center justify-center rounded-control align-middle text-ink-3 hover:bg-sunk hover:text-ink`} title={open ? "Show less" : "Show the full message"} aria-label={open ? "Show less" : "Show the full message"} aria-expanded={open} disabled={q.isFetching} onClick={() => setFull(!open)}>
    <Icon name="chev" size={12} className={open ? "rotate-180" : ""} />
  </button>;
  if (full && q.data) return <div className="min-w-0 text-[13px] text-ink-2"><Markdown text={q.data} runShell />{arrow(true)}</div>;
  return <p className="m-0 text-[13px] whitespace-pre-wrap text-ink-2">{text}{cut && arrow(false)}{full && q.isError && <span className="ml-1 text-[12px] text-gh-closed">Could not load the full message.</span>}</p>;
}

/** Steer a finished or failed agent from Today: the session's message box (same draft as its page). Once a message is
 * sent the row leaves Today at once (the agent works again); it comes back when the agent needs you again. Not for a
 * bot's session: bots are messaged in their chat. */
function ReplyBox({ s, item, expanded, toggle, focus, focused }: Pick<RowProps, "item" | "expanded" | "toggle"> & { s: Session; focus: boolean; focused: () => void }) {
  const qc = useQueryClient();
  const harness = useHarnessInfo(s.harness);
  const inputRef = useRef<HTMLTextAreaElement>(null);
  const dropImages = useRef<((files: File[]) => void) | null>(null);
  useEffect(() => {
    if (!focus) return;
    inputRef.current?.focus();
    focused();
  }, [focus, focused]);
  const sent = () => {
    toast("Sent to the agent");
    if (expanded) toggle(item.id);
    qc.setQueryData<Today>(["today"], (old) => old && { ...old, items: old.items.filter((i) => i.id !== item.id) });
    void qc.invalidateQueries({ queryKey: ["today"] });
    void qc.invalidateQueries({ queryKey: ["sessions"] });
  };
  return <MessageBox className="pt-1" sessionId={s.id} acceptsImages={!!harness?.capabilities.images} dropImages={dropImages} inputRef={inputRef} ask={null} onSent={sent}
    disabled={s.status === "in_terminal" || !!s.quota_pause} placeholder={s.status === "in_terminal" ? "The agent runs in a terminal" : s.quota_pause ? "Paused for quota: open the agent to queue a message" : "Message the agent: it continues from here"} />;
}

/** The message box of a finished or failed agent's details, and the Reply entry of its ⋯ (laptop) that opens the row
 * and focuses the box. */
function useReply({ item, expanded, toggle }: Pick<RowProps, "item" | "expanded" | "toggle">, s: Session | null) {
  const [focus, setFocus] = useState(false);
  const focused = useRef(() => setFocus(false)).current;
  const phone = usePhone();
  if (!s || s.launch_spec.bot) return { open: undefined, box: null };
  return {
    // The phone's opened row already shows the box above its menu.
    open: phone ? undefined : () => { if (!expanded) toggle(item.id); setFocus(true); },
    box: <ReplyBox s={s} item={item} expanded={expanded} toggle={toggle} focus={focus} focused={focused} />,
  };
}

function QuestionRow(p: RowProps) {
  const { item } = p;
  const s = useSession(item.record);
  return <Row {...p} age={!s} meta={<SessionMeta s={s} item={item} />}
    main={<Expand {...p}>Answer</Expand>} menu={(close) => <>{s?.launch_spec.bot && <OpenChat bot={s.launch_spec.bot} close={close} />}<OpenAgent id={item.record} close={close} /></>}
    details={<QuestionDetails item={item} />} />;
}

/** The open question's options, loaded only once the row is expanded (a folded row loads no events). */
function QuestionDetails({ item }: { item: TodayItem }) {
  const s = useSession(item.record);
  const ask = useOpenQuestion(item.record);
  return ask && s?.live ? <AskCard row={ask} sessionId={item.record} live={s.live} hint={s.launch_spec.bot ? "Or open the chat to type another answer." : "Or open the agent to type another answer."} /> : <Last text={s?.last_message ?? (item.detail || "The agent asked a question.")} id={item.record} seq={s?.last_seq} />;
}

/** Fix merge when its merge stopped (the line says "Not merged"), else review its Local PR when it wrote one, else
 * Merge when safe, else Review changes when there is work not handed
 * off yet (merged, in a Local PR or a GitHub pull request), else
 * Open agent (it opens at the last message, to read it in full); the branch facts in the details. */
function FinishedRow(p: RowProps) {
  const { item } = p;
  const pr = item.grouped.find((g) => g.kind === "local_pr");
  const s = useSession(item.record);
  const loading = useSessionWorktrees(item.record).isPending;
  const branch = useBranch(item.record, s);
  const { t, safe, facts, work, stopped } = branch;
  const phone = usePhone();
  const reply = useReply(p, s);
  return <Row {...p} age={!s} meta={<><SessionMeta s={s} item={item} />{stopped && <span className="flex-none font-semibold whitespace-nowrap text-attn">· Not merged</span>}</>}
    main={stopped ? <FixMergeButton sessionId={item.record} branch={branch} label={phone ? "Fix" : undefined} primary={false} />
      : pr ? <ReviewLocalPr id={pr.record} />
      : safe ? <MergeButton sessionId={item.record} branch={branch} label="Merge" primary={false} onMerged={() => runAction("session.seen", { id: item.record }).catch(toastError)} />
      : work ? <Link className="btn btn-sm no-underline" to="/agents" search={{ session: item.record, view: "changes" }} title="Review the changes of the agent">Changes</Link>
      : !loading && (s?.launch_spec.bot ? <OpenChat bot={s.launch_spec.bot} /> : <Link className="btn btn-sm no-underline" to="/agents" search={{ session: item.record }} title="Open the agent at its last message">Read</Link>)}
    second={!phone && <Archive id={item.record} />}
    menu={SessionMenu({ id: item.record, open: !!stopped || !!pr || safe || work || loading || !!s?.launch_spec.bot, archive: phone, reply: reply.open })}
    details={(s?.last_message || facts || stopped || reply.box) && <>
      <NotMerged branch={branch} />
      <Last text={s?.last_message} id={item.record} seq={s?.last_seq} />
      {facts && <div className="flex min-w-0 items-center gap-1.5 text-[12px] text-ink-2"><Icon name="branch" size={12} className="flex-none text-ink-3" /><span className="truncate font-mono">{t!.branch}</span><span className="truncate">· {facts}</span></div>}
      {reply.box}
    </>} />;
}

function FailedRow(p: RowProps) {
  const { item } = p;
  const s = useSession(item.record);
  const phone = usePhone();
  const reply = useReply(p, s);
  return <Row {...p} age={!s} meta={<SessionMeta s={s} item={item} />}
    main={<ActionButton name="session.resume" params={{ id: item.record }} body={{ continue_turn: true }} />}
    second={!phone && <Archive id={item.record} />}
    menu={SessionMenu({ id: item.record, archive: phone, reply: reply.open })}
    details={(s?.last_message || reply.box) && <><Last text={s?.last_message} id={item.record} seq={s?.last_seq} />{reply.box}</>} />;
}

function LocalPrRow(p: RowProps) {
  const { item } = p;
  return <Row {...p}
    main={<ReviewLocalPr id={item.record} />}
    menu={!!item.session_id && ((close) => <OpenAgent id={item.session_id!} close={close} />)} />;
}

/** One-word Start labels for the phone's button column (the full label is the tooltip and the card's button). */
const SHORT: Record<Proposal["body"]["kind"], string> = { launch: "Start", message: "Send", schedule: "Schedule", cron: "Create", status: "", preset: "Save", tool: "Add", bot: "Create", routine: "Add", bot_instructions: "Apply", bot_files: "Apply", channel: "Create", channel_join: "Add", ready: "", report: "" };

/** Start in one click when it is a plain start; ready, status and uncertain cards open first. */
function ProposalRow({ p: proposal, ...p }: RowProps & { p: Proposal | undefined }) {
  const qc = useQueryClient();
  const phone = usePhone();
  const { item } = p;
  if (!proposal) return <Row {...p} />;
  const waiting = proposal.state === "proposed" || proposal.state === "failed";
  const report = proposal.body.kind === "report" ? proposal.body : null;
  const plain = waiting && !report && proposal.body.kind !== "status" && proposal.body.kind !== "ready";
  const act = async (action: "accept" | "dismiss") => {
    const next = await runProposalAction<Proposal>(proposal.id, action);
    qc.setQueryData<Proposal[]>(["proposals"], (old) => old?.map((x) => (x.id === next.id ? next : x)));
    if (next.state === "started") await qc.invalidateQueries({ queryKey: ["sessions"] });
    if (next.state === "failed") toastError(new Error(next.error ?? "It could not start"));
  };
  const run = (action: "accept" | "dismiss") => () => void act(action).catch(toastError);
  return <Row {...p} icon={proposalIcon(proposal)} meta={item.detail ? undefined : <span className="truncate">{proposal.note ? proposal.note : "Proposal"}</span>}
    main={report && waiting
      ? report.outcome === "needs_you"
        ? <Expand {...p}>Reply</Expand>
        : <button type="button" className="btn btn-sm" title="Mark read: it moves to Done" onClick={run("dismiss")}><Icon name="check" size={12} />{phone ? "Read" : "Mark read"}</button>
      : plain
      ? <button type="button" className="btn btn-sm" title={START[proposal.body.kind]} onClick={run("accept")}><Icon name="play" size={12} />{phone ? SHORT[proposal.body.kind] : START[proposal.body.kind]}</button>
      : <Expand {...p}>{proposal.body.kind === "status" ? "Show" : "Review"}</Expand>}
    menu={report && waiting ? report.outcome === "needs_you" && ((close) => <MenuItem icon={<Icon name="check" size={14} />} label="Mark read" onClick={() => { close(); run("dismiss")(); }} />)
      : (plain || proposal.body.kind === "status") && ((close) => <MenuItem icon={<Icon name="x" size={14} />} label="Dismiss" onClick={() => { close(); run("dismiss")(); }} />)}
    details={<ProposalCard p={proposal} />} card />;
}

function OutboxRow(p: RowProps) {
  const m = useOutboxMessages().data?.find((x) => x.id === p.item.record);
  return <Row {...p} main={<Expand {...p}>Review</Expand>} details={m ? <MessageCard m={m} /> : <Loading what="the message" />} card />;
}

function SuggestionRow({ s, ...p }: RowProps & { s: Suggestion | undefined }) {
  const url = s ? suggestionItemUrl(s) : null;
  return <Row {...p}
    main={<Expand {...p}>{s?.kind === "review" ? "Review" : "Read"}</Expand>}
    menu={(!!url || !!s?.session_id) && ((close) => <>
      {url && <MenuItem icon={<Icon name="ext" size={14} />} label="Open on GitHub" onClick={() => { close(); openExternal(url); }} />}
      {s?.session_id && <OpenAgent id={s.session_id} close={close} />}
    </>)}
    details={s && s.target.kind === "git_hub" ? <SuggestionCard suggestion={s} /> : <Loading what="the suggestion" />} card />;
}

function RoutineRow(p: RowProps) {
  const { item } = p;
  return <Row {...p}
    main={<Link className="btn btn-sm no-underline" to="/schedules" search={{ routine: item.record }} title="Open the routine">Open</Link>}
    menu={!!item.session_id && ((close) => <OpenAgent id={item.session_id!} close={close} />)} />;
}

function RunnerRow(p: RowProps) {
  return <Row {...p} main={<Link className="btn btn-sm no-underline" to="/settings/runners" title="Open Home and runners">Open</Link>} />;
}

/** An agent CLI update: Approve (a new major version), Retry (did not install) or Mark read (completed); the
 * details list the runners, the command to run where Launchpad cannot update, and the release notes. */
function CliUpdateRow(p: RowProps) {
  const { item } = p;
  const version = item.record.split("@").pop() ?? "";
  const action = item.action as ActionName | null;
  return <Row {...p}
    main={action && <ActionButton name={action} params={{ id: item.record }} vars={{ version }} />}
    menu={(close) => <>
      {action !== "cli-update.dismiss" && <ActionMenuItem name="cli-update.dismiss" params={{ id: item.record }} close={close} label={action === "cli-update.approve" ? "Skip this version" : undefined} />}
      <Go close={close} icon="sliders" label="Open agent CLI updates" go={(n) => n({ to: "/settings/runners" })} />
    </>}
    details={<CliUpdateDetails id={item.record} />} />;
}

/** A bot asks you in a channel or DM (CHN-07): Reply opens the ask and the channel's message box in the row (the
 * same draft as the channel page); your message answers the ask and the row leaves Today. Open channel and Mark read
 * (it leaves Today without an answer) in ⋯. */
function ChannelAskRow(p: RowProps) {
  const { item, expanded, toggle } = p;
  const qc = useQueryClient();
  const phone = usePhone();
  const page = useQuery({ queryKey: ["channel", item.record], queryFn: () => api.get<ChannelPage>(`/api/channels/${item.record}`), enabled: expanded, retry: false });
  const ask = page.data?.messages.find((m) => m.id === item.message);
  const sent = async () => {
    toast("Sent to the channel");
    if (expanded) toggle(item.id);
    qc.setQueryData<Today>(["today"], (old) => old && { ...old, items: old.items.filter((i) => i.id !== item.id) });
    await Promise.all([qc.invalidateQueries({ queryKey: ["today"] }), qc.invalidateQueries({ queryKey: ["channels"] }), qc.invalidateQueries({ queryKey: ["channel", item.record] })]);
  };
  return <Row {...p}
    main={<Expand {...p}>Reply</Expand>}
    menu={(close) => <>
      <Go close={close} icon="comment" label="Open channel" detail="Every message and who woke up" go={(n) => n({ to: "/channels/$id", params: { id: item.record } })} />
      <ActionMenuItem name="channel.read" params={{ id: item.record }} body={{ through: item.message }} close={close} detail="Remove it from Today without an answer" onDone={() => qc.invalidateQueries({ queryKey: ["today"] })} />
    </>}
    details={page.isLoading ? <Loading what="the message" /> : <>
      {ask ? <div className="min-w-0 text-[13px] text-ink-2"><Markdown text={ask.text} /></div> : <p className="m-0 text-[13px] text-ink-2">{item.detail}</p>}
      {page.isError && <ErrorState error={page.error} onRetry={() => void page.refetch()} />}
      {/* The phone's keyboard would cover the ask: there the box takes focus on a tap. */}
      {page.data && <ChannelComposer channel={page.data.channel} replyTo={item.message ?? undefined} className="pt-1" autoFocus={!phone} onSent={sent} />}
    </>} />;
}

export const ItemRow = memo(function ItemRow({ proposal, suggestion, ...p }: RowProps & { proposal: Proposal | undefined; suggestion: Suggestion | undefined }) {
  switch (p.item.kind) {
    case "question": return <QuestionRow {...p} />;
    case "finished": return <FinishedRow {...p} />;
    case "failed": return <FailedRow {...p} />;
    case "local_pr": return <LocalPrRow {...p} />;
    case "proposal": return <ProposalRow {...p} p={proposal} />;
    case "outbox": return <OutboxRow {...p} />;
    case "suggestion": return <SuggestionRow {...p} s={suggestion} />;
    case "routine": return <RoutineRow {...p} />;
    case "runner": return <RunnerRow {...p} />;
    case "home_update": return <RunnerRow {...p} />; // Opens Settings → Home and runners, where Update Home is.
    case "cli_update": return <CliUpdateRow {...p} />;
    case "channel_ask": return <ChannelAskRow {...p} />;
  }
});
