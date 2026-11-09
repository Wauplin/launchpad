// Today's rows: one compact line per decision (icon of its kind, title, one meta line, its buttons). Rows whose
// decision needs reading (a question, a proposal, a message, a suggestion, an agent's last message) open their
// full card below the line with the title or the arrow button. The bot chat keeps the cards (TodayCards.tsx).

import { useQueryClient } from "@tanstack/react-query";
import { Link, useNavigate } from "@tanstack/react-router";
import { memo, type ReactNode } from "react";
import type { Proposal } from "@/gen/Proposal";
import type { Suggestion } from "@/gen/Suggestion";
import { ActionButton, ActionMenuItem } from "@/components/ActionButton";
import { Icon, type IconName } from "@/components/Icon";
import { toastError } from "@/components/Toast";
import { MenuItem } from "@/components/Popover";
import { Loading, RowActions } from "@/components/ui";
import { AskCard } from "@/features/agents/AskCard";
import { useSessionWorktrees } from "@/features/agents/Worktrees";
import { MessageCard, useOutboxMessages } from "@/features/outbox/MessageCard";
import { ProposalCard, proposalIcon, runProposalAction, START } from "@/features/proposals/ProposalCard";
import { SuggestionCard } from "@/features/suggestions/Suggestions";
import { runAction } from "@/lib/actions";
import { ago } from "@/lib/format";
import { usePhone } from "@/lib/phone";
import { useSession } from "@/lib/queries";
import type { TodayItem } from "./data";
import { ghParams, githubRef, MergeButton, ReviewWithAgent, SessionMeta, useBranch, useOpenQuestion } from "./TodayCards";

/** The icon, color and name of each kind: GitHub's icons for pull requests. */
export const LOOK: Record<TodayItem["kind"], { icon: IconName; color: string; name: string }> = {
  question: { icon: "alert", color: "text-attn", name: "An agent asks you a question" },
  finished: { icon: "okcircle", color: "text-gh-open", name: "Finished agent" },
  failed: { icon: "xcircle", color: "text-gh-closed", name: "Failed or interrupted agent" },
  local_pr: { icon: "pr-local", color: "text-teal", name: "Local PR to publish" },
  proposal: { icon: "spark", color: "text-work", name: "Proposal" },
  outbox: { icon: "send", color: "text-work", name: "Message to send" },
  suggestion: { icon: "comment", color: "text-teal", name: "Suggestion to post on GitHub" },
  review_request: { icon: "pr", color: "text-gh-open", name: "Pull request that requests your review" },
  routine: { icon: "refresh", color: "text-gh-closed", name: "Routine whose last run failed" },
  runner: { icon: "window", color: "text-attn", name: "Runner to update" },
  home_update: { icon: "download", color: "text-attn", name: "Update for Home" },
};

type RowProps = { item: TodayItem; expanded: boolean; toggle: (id: string) => void; highlight: boolean };
type Menu = ((close: () => void) => ReactNode) | false | null | undefined;

/** The line: icon, title (opens the details when there are some, else `to`), meta, then the action group: `main`
 * (one outline button in a fixed column) and ⋯ with the other actions (`menu`). On the phone the line keeps only
 * `main`, a one-word outline button of one minimum width without its icon (the column never moves), and the title
 * up to two lines: a tap on the title or meta opens the details with the other actions. */
function Row({ item, expanded, toggle, highlight, icon, meta, age = true, to, main, menu, details, card = false }: RowProps & { icon?: IconName; meta?: ReactNode; age?: boolean; to?: ReactNode; main?: ReactNode; menu?: Menu; details?: ReactNode; card?: boolean }) {
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
          <RowActions label={`More actions for ${item.title}`} menuWidth={260} main={buttons && main} more={buttons && menu} />
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
  <Link className="btn btn-sm no-underline" to="/predrafts/$id" params={{ id }} title="Publish to GitHub or merge locally from its page">Review</Link>;

/** The main button of a row whose decision needs reading first: it opens the card. */
const Expand = ({ item, expanded, toggle, children }: Pick<RowProps, "item" | "expanded" | "toggle"> & { children: ReactNode }) =>
  expanded ? null : <button type="button" className="btn btn-sm" aria-expanded={false} onClick={() => toggle(item.id)}>{children}</button>;

/** Opens a page from a ⋯ menu. */
function Go({ close, label, icon = "ext", detail, go }: { close: () => void; label: ReactNode; icon?: IconName; detail?: ReactNode; go: (navigate: ReturnType<typeof useNavigate>) => unknown }) {
  const navigate = useNavigate();
  return <MenuItem icon={<Icon name={icon} size={14} />} label={label} detail={detail} onClick={() => { close(); void go(navigate); }} />;
}

const OpenAgent = ({ id, close }: { id: string; close: () => void }) =>
  <Go close={close} icon="agents" label="Open agent" go={(n) => n({ to: "/agents", search: { session: id } })} />;

/** Open agent (unless it is the row's main button), Archive and Mark seen: the ⋯ of a finished or failed agent. */
const SessionMenu = ({ id, open = true }: { id: string; open?: boolean }) => (close: () => void) => <>
  {open && <OpenAgent id={id} close={close} />}
  <ActionMenuItem name="session.archive" params={{ id }} close={close} detail="It stays resumable" />
  <ActionMenuItem name="session.seen" params={{ id }} close={close} detail="Keep the agent and remove it from Today" />
</>;

const Last = ({ text }: { text: string | null | undefined }) => text ? <p className="m-0 line-clamp-4 text-[13px] whitespace-pre-wrap text-ink-2">{text}</p> : null;

function QuestionRow(p: RowProps) {
  const { item } = p;
  const s = useSession(item.record);
  const ask = useOpenQuestion(item.record);
  return <Row {...p} age={!s} meta={<SessionMeta s={s} item={item} />}
    main={<Expand {...p}>Answer</Expand>} menu={(close) => <OpenAgent id={item.record} close={close} />}
    details={ask && s?.live ? <AskCard row={ask} sessionId={item.record} live={s.live} hint="Or open the agent to type another answer." /> : <Last text={s?.last_message ?? (item.detail || "The agent asked a question.")} />} />;
}

/** Review its Local PR when it wrote one, else Merge when safe, else Review changes when there is work, else
 * Open agent (it opens at the last message, to read it in full); the branch facts in the details. */
function FinishedRow(p: RowProps) {
  const { item } = p;
  const pr = item.grouped.find((g) => g.kind === "local_pr");
  const s = useSession(item.record);
  const loading = useSessionWorktrees(item.record).isPending;
  const branch = useBranch(item.record, s);
  const { w, t, safe, facts } = branch;
  const work = !!w && (w.ahead > 0 || w.dirty_files > 0);
  const phone = usePhone();
  return <Row {...p} age={!s} meta={<SessionMeta s={s} item={item} />}
    main={pr ? <ReviewLocalPr id={pr.record} />
      : safe ? <MergeButton sessionId={item.record} branch={branch} label="Merge" primary={false} onMerged={() => runAction("session.seen", { id: item.record }).catch(toastError)} />
      : work ? <Link className="btn btn-sm no-underline" to="/agents" search={{ session: item.record, view: "changes" }} title="Review the changes of the agent">Review</Link>
      : !loading && <Link className="btn btn-sm no-underline" to="/agents" search={{ session: item.record }} title="Open the agent at its last message">{phone ? "Open" : "Open agent"}</Link>}
    menu={SessionMenu({ id: item.record, open: !!pr || safe || work || loading })}
    details={(s?.last_message || facts) && <>
      <Last text={s?.last_message} />
      {facts && <div className="flex min-w-0 items-center gap-1.5 text-[12px] text-ink-2"><Icon name="branch" size={12} className="flex-none text-ink-3" /><span className="truncate font-mono">{t!.branch}</span><span className="truncate">· {facts}</span></div>}
    </>} />;
}

function FailedRow(p: RowProps) {
  const { item } = p;
  const s = useSession(item.record);
  return <Row {...p} age={!s} meta={<SessionMeta s={s} item={item} />}
    main={<ActionButton name="session.resume" params={{ id: item.record }} body={{ continue_turn: true }} />}
    menu={SessionMenu({ id: item.record })}
    details={s?.last_message && <Last text={s.last_message} />} />;
}

function LocalPrRow(p: RowProps) {
  const { item } = p;
  return <Row {...p}
    main={<ReviewLocalPr id={item.record} />}
    menu={!!item.session_id && ((close) => <OpenAgent id={item.session_id!} close={close} />)} />;
}

/** One-word Start labels for the phone's button column (the full label is the tooltip and the card's button). */
const SHORT: Record<Proposal["body"]["kind"], string> = { launch: "Start", message: "Send", schedule: "Schedule", cron: "Create", status: "", preset: "Save", tool: "Add", bot: "Create", routine: "Add", bot_instructions: "Apply", ready: "", report: "" };

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
  const ref = githubRef(p.item);
  return <Row {...p}
    main={<Expand {...p}>Review</Expand>}
    menu={(!!ref || !!s?.session_id) && ((close) => <>
      {ref && <Go close={close} icon="pr" label="Open pull request" go={(n) => n({ to: "/github/$owner/$repo/$number", params: ghParams(ref.repo, ref.number), search: s ? { draft: s.id } : undefined })} />}
      {s?.session_id && <OpenAgent id={s.session_id} close={close} />}
    </>)}
    details={s && s.target.kind === "git_hub" ? <SuggestionCard suggestion={s} /> : <Loading what="the suggestion" />} card />;
}

/** Review with agent; Open (the PR page) and Done (removes the notification here and on GitHub) in ⋯. */
function ReviewRequestRow(p: RowProps) {
  const { item } = p;
  const ref = githubRef(item);
  return <Row {...p}
    to={ref && <Link className="block max-w-full text-ink no-underline hover:underline" to="/github/$owner/$repo/$number" params={ghParams(ref.repo, ref.number)}><span className="block truncate text-[13px] font-semibold" title={item.title}>{item.title}</span></Link>}
    main={<ReviewWithAgent item={item} primary={false} label="Review" />}
    menu={(close) => <>
      {ref && <Go close={close} icon="pr" label="Open pull request" go={(n) => n({ to: "/github/$owner/$repo/$number", params: ghParams(ref.repo, ref.number) })} />}
      <ActionMenuItem name="inbox.done" params={{ id: item.record }} close={close} detail="Not for you: done here and on GitHub" />
    </>} />;
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

export const ItemRow = memo(function ItemRow({ proposal, suggestion, ...p }: RowProps & { proposal: Proposal | undefined; suggestion: Suggestion | undefined }) {
  switch (p.item.kind) {
    case "question": return <QuestionRow {...p} />;
    case "finished": return <FinishedRow {...p} />;
    case "failed": return <FailedRow {...p} />;
    case "local_pr": return <LocalPrRow {...p} />;
    case "proposal": return <ProposalRow {...p} p={proposal} />;
    case "outbox": return <OutboxRow {...p} />;
    case "suggestion": return <SuggestionRow {...p} s={suggestion} />;
    case "review_request": return <ReviewRequestRow {...p} />;
    case "routine": return <RoutineRow {...p} />;
    case "runner": return <RunnerRow {...p} />;
    case "home_update": return <RunnerRow {...p} />; // Opens Settings → Home and runners, where Update Home is.
  }
});
