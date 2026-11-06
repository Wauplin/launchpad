// Slack for agents (slackline): two tools, each with its own Slack app and token. Read-only is the default of every
// launch; write is picked per launch in the composer's Tools. Used in Settings → Credentials and tools and in the setup
// checklists (machine and Home).

import { useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import type { Tool } from "@/gen/Tool";
import type { ToolTest } from "@/gen/ToolTest";
import type { VaultOverview } from "@/gen/VaultOverview";
import { Icon } from "@/components/Icon";
import { Badge } from "@/components/ui";
import { toast, toastError } from "@/components/Toast";
import { api } from "@/lib/api";
import { formatAge } from "@/lib/format";

export const SLACK_READ = "slack-read";
export const SLACK_WRITE = "slack-write";

/** Whether the read-only Slack tool is added with its token (the setup step is done). */
export function slackReady(v: VaultOverview | undefined): boolean {
  return !!v && v.tools.some((t) => t.id === SLACK_READ) && v.entries.some((e) => e.id === SLACK_READ);
}

const MODES = [
  { id: SLACK_READ, title: "Read-only", tag: "Recommended", what: "Agents search and read channels, threads, DMs and people. Write commands are switched off, and the token has read permissions only." },
  { id: SLACK_WRITE, title: "Write", tag: "Optional", what: "Agents can also post, reply, react, pin and upload. A launch gets it only when you pick Slack (write) in the composer's Tools." },
];

export function SlackSetup({ v }: { v: VaultOverview }) {
  return (
    <div className="grid gap-2.5">
      <p className="m-0 text-[12px] text-ink-3">Agents use Slack through <a className="text-teal-ink hover:underline" href="https://github.com/co42/slackline" target="_blank" rel="noreferrer">slackline</a>. Each mode is a separate Slack app with its own token, so a read-only agent cannot post even if it tries. Every launch gets read-only; pick Slack (write) in the composer's Tools when an agent must post.</p>
      {MODES.map((m) => <SlackMode key={m.id} v={v} mode={m} />)}
    </div>
  );
}

function SlackMode({ v, mode }: { v: VaultOverview; mode: (typeof MODES)[number] }) {
  const qc = useQueryClient();
  const template = v.templates.find((t) => t.id === mode.id);
  const help = v.template_help.find((h) => h.id === mode.id);
  const tool = v.tools.find((t) => t.id === mode.id);
  const entry = v.entries.find((e) => e.id === mode.id);
  const ready = !!tool && !!entry;
  const [open, setOpen] = useState(false);
  const [token, setToken] = useState("");
  const [busy, setBusy] = useState(false);
  const [test, setTest] = useState<ToolTest | null>(null);
  if (!template || !help) return null;
  const runTest = () => api.post<ToolTest>(`/api/vault/tools/${mode.id}/test`, {}).then(setTest, toastError);
  const save = async () => {
    setBusy(true);
    try {
      await api.put(`/api/vault/entries/${mode.id}`, { kind: "token", name: template.name, value: token.trim() });
      // Keep the owner's changes (workspaces, settings) of an existing tool; only the token is replaced.
      const body: Tool = tool ?? template;
      await api.put(`/api/vault/tools/${mode.id}`, body);
      toast(`Saved ${template.name}`);
      setToken("");
      setOpen(false);
      await qc.invalidateQueries({ queryKey: ["vault"] });
      await runTest();
    } catch (e) {
      toastError(e);
    } finally {
      setBusy(false);
    }
  };
  const showForm = !ready || open;
  const odd = token.trim() && !token.trim().startsWith("xoxp-");
  return (
    <div className="grid gap-2 rounded-lg border border-line-2 bg-chrome p-2.5 text-[13px]">
      <div className="flex flex-wrap items-center gap-2">
        <span className={`grid size-5 flex-none place-items-center rounded-full ${ready ? "bg-diff-add text-gh-open" : "border border-line bg-paper text-ink-3"}`} aria-hidden="true">{ready && <Icon name="check" size={11} />}</span>
        <b>Slack · {mode.title}</b>
        <span className="text-[11px] text-ink-3">{mode.tag}</span>
        <span className="ml-auto flex flex-wrap items-center gap-1.5">
          {test && <Badge tone={test.ok ? "ok" : "attn"} title={test.message}>{test.ok ? "token works" : "token refused"}</Badge>}
          {ready && <button className="btn btn-sm" onClick={() => void runTest()}>Test</button>}
          {ready && !open && <button className="btn btn-sm" onClick={() => setOpen(true)}>Replace the token</button>}
        </span>
      </div>
      <span className="text-[12px] text-ink-2">{mode.what}</span>
      {ready && entry && <span className="text-[12px] text-ink-3">Token {entry.detail}, saved {formatAge(entry.updated_at)} ago.{tool.workspaces.length === 0 ? " Not granted in any workspace (Tools → Edit)." : ""}</span>}
      {test && !test.ok && <span className="text-[12px] text-attn">{test.message}</span>}
      {showForm && (
        <ol className="m-0 grid list-none gap-2 p-0">
          <Step n={1}>
            <span>Create the Slack app. The link opens Slack with the name and permissions filled in.</span>
            <a className="btn btn-sm btn-primary w-fit no-underline" href={help.token_url} target="_blank" rel="noreferrer"><Icon name="ext" size={12} />{help.link_label}</a>
          </Step>
          <Step n={2}>
            <span>On that page:</span>
            <ul className="m-0 grid list-disc gap-0.5 pl-4 text-[12px] text-ink-2">{help.steps.slice(1).map((s) => <li key={s}>{s}</li>)}</ul>
          </Step>
          <Step n={3}>
            <span>Paste the token. Home keeps it; agents only see a placeholder.</span>
            <span className="flex flex-wrap items-center gap-2">
              <input className="field-input w-[320px] max-w-full font-mono" type="password" value={token} onChange={(e) => setToken(e.target.value)} placeholder="xoxp-…" autoComplete="off" aria-label={`Slack ${mode.title} token`} />
              <button className="btn btn-sm btn-primary" disabled={busy || !token.trim()} onClick={() => void save()}>{busy ? "Saving…" : "Save on Home"}</button>
              {ready && <button className="btn btn-sm" onClick={() => { setOpen(false); setToken(""); }}>Cancel</button>}
            </span>
            {odd && <span className="text-[12px] text-attn">This does not look like a User OAuth Token (xoxp-…). Copy the one under Install App.</span>}
          </Step>
        </ol>
      )}
    </div>
  );
}

function Step({ n, children }: { n: number; children: React.ReactNode }) {
  return (
    <li className="flex gap-2">
      <span className="grid size-5 flex-none place-items-center rounded-full border border-line bg-paper text-[11px] font-semibold text-ink-3">{n}</span>
      <span className="grid min-w-0 flex-1 gap-1.5">{children}</span>
    </li>
  );
}
