// Today's rows: one compact line per decision (icon of its kind, title, one meta line, its buttons). Rows whose
// decision needs reading (a question, a proposal, a message, a suggestion, an agent's last message) open their
// full card below the line with the title or the arrow button. The bot chat keeps the cards (TodayCards.tsx).

import { useQueryClient } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { memo, type ReactNode } from "react";
import type { Proposal } from "@/gen/Proposal";
import type { Suggestion } from "@/gen/Suggestion";
import { ActionButton } from "@/components/ActionButton";
import { Icon, type IconName } from "@/components/Icon";
import { toastError } from "@/components/Toast";
import { Loading } from "@/components/ui";
import { AskCard } from "@/features/agents/AskCard";
import { useSessionWorktrees } from "@/features/agents/Worktrees";
import { MessageCard, useOutboxMessages } from "@/features/outbox/MessageCard";
import { ProposalCard, proposalIcon, runProposalAction, START } from "@/features/proposals/ProposalCard";
import { SuggestionCard } from "@/features/suggestions/Suggestions";
import { runAction } from "@/lib/actions";
import { formatAge } from "@/lib/format";
import { usePhone } from "@/lib/phone";
import { useSession } from "@/lib/queries";
import type { TodayItem } from "./data";
import { ghParams, githubRef, MergeButton, OpenSession, ReviewWithAgent, SessionMeta, useBranch, useOpenQuestion } from "./TodayCards";

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
};

type RowProps = { item: TodayItem; expanded: boolean; toggle: (id: string) => void; highlight: boolean };

/** The line: icon, title (opens the details when there are some, else `to`), meta, `more` (on hover), `main` (the
 * primary button), the other buttons (`actions`), and the arrow that opens the details. On the phone the line keeps only
 * `main`: a tap on the title or meta opens the details with the other buttons, so each row stays one line high. */
function Row({ item, expanded, toggle, highlight, icon, meta, age = true, to, main, actions, more, details, card = false }: RowProps & { icon?: IconName; meta?: ReactNode; age?: boolean; to?: ReactNode; main?: ReactNode; actions?: ReactNode; more?: ReactNode; details?: ReactNode; card?: boolean }) {
  // `card`: the details are a full card with its own buttons, so the line hides its buttons while it is open.
  const buttons = !(card && expanded && details);
  const phone = usePhone();
  const opens = !!details || (phone && (!!actions || !!more));
  const look = LOOK[item.kind];
  const title = <span className="block truncate text-[13px] font-semibold" title={item.title}>{item.title}</span>;
  const metaLine = <div className="flex min-w-0 items-center gap-x-1.5 text-[12px] text-ink-3">{meta ?? <span className="truncate">{item.detail}</span>}{age && <span className="flex-none whitespace-nowrap">· {formatAge(item.at)} ago</span>}</div>;
  const kind = <span className={`grid size-6 flex-none place-items-center ${look.color}`} title={look.name} aria-label={look.name} role="img"><Icon name={icon ?? look.icon} size={16} /></span>;
  const chev = (size: number) => <Icon name="chev" size={size} className={`flex-none transition-transform ${expanded ? "rotate-180" : ""}`} />;
  const label = expanded ? "Hide the details" : "Show the details";
  if (phone) return (
    <article data-tour="today.card" data-tour-id={item.id} className={`border-b border-line-2 ${highlight ? "bg-select" : expanded ? "bg-chrome" : ""}`}>
      <div className="flex min-h-[60px] min-w-0 items-center gap-2 py-1.5 pr-3">
        {opens
          ? <button type="button" className="flex min-h-11 min-w-0 flex-1 items-center gap-2.5 pl-3 text-left" aria-expanded={expanded} aria-label={`${item.title}: ${label.toLowerCase()}`} onClick={() => toggle(item.id)}>
            {kind}<span className="min-w-0 flex-1">{title}{metaLine}</span><span className="text-ink-3">{chev(12)}</span>
          </button>
          : <div className="flex min-w-0 flex-1 items-center gap-2.5 pl-3">{kind}<div className="min-w-0 flex-1">{to ?? title}{metaLine}</div></div>}
        {buttons && main && <div className="flex flex-none items-center">{main}</div>}
      </div>
      {expanded && opens && <div className="grid min-w-0 grid-cols-[minmax(0,1fr)] gap-2 px-3 pb-3">
        {details}
        {buttons && (actions || more) && <div className="flex flex-wrap items-center gap-1.5">{actions}{more}</div>}
      </div>}
    </article>
  );
  return (
    <article data-tour="today.card" data-tour-id={item.id} className={`group border-b border-line-2 ${highlight ? "bg-select" : expanded ? "bg-chrome" : "hover:bg-chrome"}`}>
      <div className="flex min-w-0 flex-wrap items-center gap-x-3 gap-y-1.5 px-3 py-2">
        <div className="flex min-w-0 flex-[1_1_300px] items-center gap-2.5">
          {kind}
          <div className="min-w-0 flex-1">
            {details ? <button type="button" className="block max-w-full text-left hover:underline" aria-expanded={expanded} onClick={() => toggle(item.id)}>{title}</button> : to ?? title}
            {metaLine}
          </div>
        </div>
        <div className="ml-auto flex max-w-full flex-none flex-wrap items-center justify-end gap-1.5">
          {buttons && more && <span className="lp-row-more inline-flex items-center gap-1.5">{more}</span>}
          {buttons && main}
          {buttons && actions}
          {opens && <button type="button" className="btn btn-sm btn-ghost px-1.5" title={label} aria-label={label} aria-expanded={expanded} onClick={() => toggle(item.id)}>{chev(12)}</button>}
        </div>
      </div>
      {expanded && opens && <div className="grid min-w-0 grid-cols-[minmax(0,1fr)] gap-2 pr-3 pb-3 pl-[46px]">{details}</div>}
    </article>
  );
}

/** The main button of a row whose decision needs reading first: it opens the card. */
const Expand = ({ item, expanded, toggle, children }: Pick<RowProps, "item" | "expanded" | "toggle"> & { children: ReactNode }) =>
  expanded ? null : <button type="button" className="btn btn-sm btn-primary" aria-expanded={false} onClick={() => toggle(item.id)}>{children}</button>;

const Seen = ({ id }: { id: string }) => <ActionButton name="session.seen" params={{ id }} title="Keep the agent and remove it from Today" />;

const Last = ({ text }: { text: string | null | undefined }) => text ? <p className="m-0 line-clamp-4 text-[13px] whitespace-pre-wrap text-ink-2">{text}</p> : null;

function QuestionRow(p: RowProps) {
  const { item } = p;
  const s = useSession(item.record);
  const ask = useOpenQuestion(item.record);
  return <Row {...p} age={!s} meta={<SessionMeta s={s} item={item} />}
    main={<Expand {...p}>Answer</Expand>} actions={<OpenSession id={item.record} />}
    details={ask && s?.live ? <AskCard row={ask} sessionId={item.record} live={s.live} hint="Or open the agent to type another answer." /> : <Last text={s?.last_message ?? (item.detail || "The agent asked a question.")} />} />;
}

/** Merge when safe, else Review changes when there is work, else Archive; the branch facts in the details. */
function FinishedRow(p: RowProps) {
  const { item } = p;
  const phone = usePhone();
  const s = useSession(item.record);
  const loading = useSessionWorktrees(item.record).isPending;
  const branch = useBranch(item.record, s);
  const { w, t, safe, facts } = branch;
  const work = !!w && (w.ahead > 0 || w.dirty_files > 0);
  const archiveMain = !loading && !safe && !work;
  return <Row {...p} age={!s} meta={<SessionMeta s={s} item={item} />}
    main={safe ? <MergeButton sessionId={item.record} branch={branch} label={phone ? "Merge" : undefined} onMerged={() => runAction("session.seen", { id: item.record }).catch(toastError)} />
      : work ? <Link className="btn btn-sm btn-primary no-underline" to="/agents" search={{ session: item.record, view: "changes" }}>{phone ? "Review" : "Review changes"}</Link>
      : archiveMain && <ActionButton name="session.archive" params={{ id: item.record }} primary />}
    actions={<OpenSession id={item.record} />}
    more={<>{!archiveMain && <ActionButton name="session.archive" params={{ id: item.record }} />}<Seen id={item.record} /></>}
    details={(s?.last_message || facts) && <>
      <Last text={s?.last_message} />
      {facts && <div className="flex min-w-0 items-center gap-1.5 text-[12px] text-ink-2"><Icon name="branch" size={13} className="flex-none text-ink-3" /><span className="truncate font-mono">{t!.branch}</span><span className="truncate">· {facts}</span></div>}
    </>} />;
}

function FailedRow(p: RowProps) {
  const { item } = p;
  const s = useSession(item.record);
  return <Row {...p} age={!s} meta={<SessionMeta s={s} item={item} />}
    main={<ActionButton name="session.resume" params={{ id: item.record }} body={{ continue_turn: true }} primary />}
    actions={<OpenSession id={item.record} />}
    more={<><ActionButton name="session.archive" params={{ id: item.record }} /><Seen id={item.record} /></>}
    details={s?.last_message && <Last text={s.last_message} />} />;
}

function LocalPrRow(p: RowProps) {
  const { item } = p;
  const phone = usePhone();
  return <Row {...p}
    main={<Link className="btn btn-sm btn-primary no-underline" to="/predrafts/$id" params={{ id: item.record }} title="Publish to GitHub or merge locally from its page">{phone ? "Review" : <>Review &amp; publish</>}</Link>}
    actions={item.session_id && <OpenSession id={item.session_id} />} />;
}

/** Start in one click when it is a plain start; ready, status and uncertain cards open first. */
function ProposalRow({ p: proposal, ...p }: RowProps & { p: Proposal | undefined }) {
  const qc = useQueryClient();
  const { item } = p;
  if (!proposal) return <Row {...p} />;
  const waiting = proposal.state === "proposed" || proposal.state === "failed";
  const plain = waiting && proposal.body.kind !== "status" && proposal.body.kind !== "ready";
  const act = async (action: "accept" | "dismiss") => {
    const next = await runProposalAction<Proposal>(proposal.id, action);
    qc.setQueryData<Proposal[]>(["proposals"], (old) => old?.map((x) => (x.id === next.id ? next : x)));
    if (next.state === "started") await qc.invalidateQueries({ queryKey: ["sessions"] });
    if (next.state === "failed") toastError(new Error(next.error ?? "It could not start"));
  };
  const run = (action: "accept" | "dismiss") => () => void act(action).catch(toastError);
  return <Row {...p} icon={proposalIcon(proposal)} meta={item.detail ? undefined : <span className="truncate">{proposal.note ? proposal.note : "Proposal"}</span>}
    main={plain
      ? <button type="button" className="btn btn-sm btn-primary" onClick={run("accept")}><Icon name="play" size={12} />{START[proposal.body.kind]}</button>
      : <Expand {...p}>{proposal.body.kind === "status" ? "Show" : "Review"}</Expand>}
    more={(plain || proposal.body.kind === "status") && <button type="button" className="btn btn-sm btn-ghost" onClick={run("dismiss")}>Dismiss</button>}
    details={<ProposalCard p={proposal} />} card />;
}

function OutboxRow(p: RowProps) {
  const m = useOutboxMessages().data?.find((x) => x.id === p.item.record);
  return <Row {...p} main={<Expand {...p}>Review</Expand>} details={m ? <MessageCard m={m} /> : <Loading what="the message" />} card />;
}

function SuggestionRow({ s, ...p }: RowProps & { s: Suggestion | undefined }) {
  const phone = usePhone();
  const ref = githubRef(p.item);
  return <Row {...p}
    main={<Expand {...p}>{phone ? "Review" : <>Review &amp; post</>}</Expand>}
    actions={ref && <Link className="btn btn-sm no-underline" to="/github/$owner/$repo/$number" params={ghParams(ref.repo, ref.number)} search={s ? { draft: s.id } : undefined}>Open PR</Link>}
    more={s?.session_id && <OpenSession id={s.session_id} />}
    details={s && s.target.kind === "git_hub" ? <SuggestionCard suggestion={s} /> : <Loading what="the suggestion" />} card />;
}

/** Review with agent, Open (the PR page), and Done (removes the notification here and on GitHub). */
function ReviewRequestRow(p: RowProps) {
  const { item } = p;
  const phone = usePhone();
  const ref = githubRef(item);
  const link = (children: ReactNode, className: string) => ref ? <Link className={className} to="/github/$owner/$repo/$number" params={ghParams(ref.repo, ref.number)}>{children}</Link> : null;
  return <Row {...p}
    to={link(<span className="block truncate text-[13px] font-semibold" title={item.title}>{item.title}</span>, "block max-w-full text-ink no-underline hover:underline")}
    main={<ReviewWithAgent item={item} label={phone ? "Review" : undefined} />}
    actions={link("Open", "btn btn-sm no-underline")}
    more={<ActionButton name="inbox.done" params={{ id: item.record }} title="Not for you: mark the notification done, here and on GitHub. It leaves Today." />} />;
}

function RoutineRow(p: RowProps) {
  const { item } = p;
  const phone = usePhone();
  return <Row {...p}
    main={<Link className="btn btn-sm btn-primary no-underline" to="/schedules" search={{ routine: item.record }}>{phone ? "Open" : "Open routine"}</Link>}
    actions={item.session_id && <OpenSession id={item.session_id} />} />;
}

function RunnerRow(p: RowProps) {
  const phone = usePhone();
  return <Row {...p} main={<Link className="btn btn-sm btn-primary no-underline" to="/settings/runners">{phone ? "Open" : "Open runners"}</Link>} />;
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
  }
});
