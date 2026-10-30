// Text and accepted references form one document. Plain typed tokens never create context.
import type { MessageContext } from "../gen/MessageContext.ts";
import type { ContextProject } from "../gen/ContextProject.ts";
import type { GitHubRef } from "../gen/GitHubRef.ts";
import type { AgentTraceRef } from "../gen/AgentTraceRef.ts";
import type { SkillRef } from "../gen/SkillRef.ts";

export type PromptDocument = MessageContext & { text: string };
export const emptyContext = (): MessageContext => ({ context_projects: [], github_refs: [], agent_traces: [], mentions: [], skills: [] });
export type ContextPick = { kind: "project"; value: ContextProject } | { kind: "github"; value: GitHubRef } | { kind: "trace"; value: AgentTraceRef } | { kind: "skill"; value: SkillRef };
export const projectKey = (p: ContextProject) => `project:${p.project_id}`;
export const githubKey = (g: GitHubRef) => `github:${g.url}`;
export const traceKey = (t: AgentTraceRef) => `trace:${t.session_id}:${t.through_seq}`;
export const skillKey = (s: SkillRef) => `skill:${s.path}`;
export const pickKey = (p: ContextPick) => p.kind === "project" ? projectKey(p.value) : p.kind === "github" ? githubKey(p.value) : p.kind === "trace" ? traceKey(p.value) : skillKey(p.value);
export const contextKeys = (d: Omit<PromptDocument, "text">) => new Set([
  ...d.context_projects.map(projectKey), ...d.github_refs.map(githubKey), ...d.agent_traces.map(traceKey), ...(d.skills ?? []).map(skillKey),
]);
function without(d: PromptDocument, keys: Set<string>): PromptDocument {
  return { ...d, context_projects: d.context_projects.filter((p) => !keys.has(projectKey(p))), github_refs: d.github_refs.filter((p) => !keys.has(githubKey(p))), agent_traces: d.agent_traces.filter((p) => !keys.has(traceKey(p))), skills: (d.skills ?? []).filter((p) => !keys.has(skillKey(p))) };
}
/** Apply an exact textarea edit, shifting surviving ranges and invalidating overlapping occurrences. */
export function replaceText<T extends PromptDocument>(doc: T, start: number, end: number, inserted: string): T {
  const delta = inserted.length - (end - start);
  const mentions = (doc.mentions ?? []).flatMap((m) => {
    if (m.end <= start) return [m];
    if (m.start >= end) return [{ ...m, start: m.start + delta, end: m.end + delta }];
    return [];
  });
  return reconcile(doc, { ...doc, text: doc.text.slice(0, start) + inserted + doc.text.slice(end), mentions });
}
/** Fallback for programmatic text updates; native input supplies the actual replaced selection. */
export function editText<T extends PromptDocument>(doc: T, text: string): T {
  if (doc.text === text) return doc;
  let start = 0, oldEnd = doc.text.length, newEnd = text.length;
  while (start < oldEnd && start < newEnd && doc.text[start] === text[start]) start++;
  while (oldEnd > start && newEnd > start && doc.text[oldEnd - 1] === text[newEnd - 1]) { oldEnd--; newEnd--; }
  return replaceText(doc, start, oldEnd, text.slice(start, newEnd));
}
/** Atomic selection: attach once, preserving existing picker refinements and ownership. */
export function insertMention<T extends PromptDocument>(doc: T, start: number, end: number, token: string, pick?: ContextPick): T {
  let next = replaceText(doc, start, end, `${token} `);
  if (!pick) return next;
  const key = pickKey(pick);
  const existing = (next.mentions ?? []).find((m) => m.key === key);
  const owned = existing?.owned ?? !contextKeys(next).has(key);
  if (!contextKeys(next).has(key)) {
    if (pick.kind === "project") next = { ...next, context_projects: [...next.context_projects, pick.value] };
    if (pick.kind === "github") next = { ...next, github_refs: [...next.github_refs, pick.value] };
    if (pick.kind === "trace") next = { ...next, agent_traces: [...next.agent_traces, pick.value] };
    if (pick.kind === "skill") next = { ...next, skills: [...(next.skills ?? []), pick.value] };
  }
  return { ...next, mentions: [...(next.mentions ?? []), { id: Array.from(crypto.getRandomValues(new Uint32Array(4)), (n) => n.toString(16).padStart(8, "0")).join(""), start, end: start + token.length, text: token, key, owned }] };
}
/** Called by stores, including chip edits. Never infer attachment identity from visible labels. */
export function reconcile<T extends PromptDocument>(previous: T, candidate: T): T {
  let next = candidate;
  if (previous.text !== next.text && previous.mentions === next.mentions) {
    const edited = editText(previous, next.text);
    const surviving = contextKeys(edited);
    const removed = new Set([...contextKeys(previous)].filter((key) => !surviving.has(key)));
    next = { ...without(next, removed), text: edited.text, mentions: edited.mentions } as T;
  }
  // A picker removal deletes only the linked occurrences, from right to left.
  const keys = contextKeys(next);
  let mentions = [...(next.mentions ?? [])];
  for (const m of [...mentions].sort((a, b) => b.start - a.start)) {
    if (keys.has(m.key)) continue;
    next = { ...next, text: next.text.slice(0, m.start) + next.text.slice(m.end) };
    mentions = mentions.filter((v) => v.id !== m.id).map((v) => v.start >= m.end ? { ...v, start: v.start - (m.end - m.start), end: v.end - (m.end - m.start) } : v);
  }
  mentions = mentions.filter((m) => next.text.slice(m.start, m.end) === m.text);
  const orphaned = new Set((previous.mentions ?? []).filter((m) => m.owned && !mentions.some((v) => v.key === m.key)).map((m) => m.key));
  return { ...without(next, orphaned), mentions } as T;
}

/** Full document history restores deleted reference identities with the text. Bounded per editor. */
export class PromptHistory<T extends PromptDocument> {
  past: T[] = [];
  future: T[] = [];
  current: T;
  constructor(doc: T) { this.current = doc; }
  sync(doc: T) { if (JSON.stringify(doc) !== JSON.stringify(this.current)) this.record(doc); }
  record(doc: T) { if (JSON.stringify(doc) === JSON.stringify(this.current)) return; this.past.push(this.current); if (this.past.length > 100) this.past.shift(); this.current = doc; this.future = []; }
  undo() { const doc = this.past.pop(); if (!doc) return null; this.future.push(this.current); this.current = doc; return doc; }
  redo() { const doc = this.future.pop(); if (!doc) return null; this.past.push(this.current); this.current = doc; return doc; }
}
