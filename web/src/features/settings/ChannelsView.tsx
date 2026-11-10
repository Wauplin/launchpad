// Settings → Channels: outgoing webhooks (their messages wait in the Outbox) and incoming hooks, plus a pointer to
// each workspace's Telegram (Settings → Workspace). Everything lives on Home; webhook URLs and hook secrets are write-only.

import { useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Link, useNavigate } from "@tanstack/react-router";
import type { Channel } from "@/gen/Channel";
import type { ChannelKind } from "@/gen/ChannelKind";
import type { CreatedHook } from "@/gen/CreatedHook";
import type { Hook } from "@/gen/Hook";
import type { HookTarget } from "@/gen/HookTarget";
import { Icon } from "@/components/Icon";
import { toast, toastError } from "@/components/Toast";
import { ConfirmButton, Empty, ErrorState, Choice, Choices, Loading, MenuSection, RowActions } from "@/components/ui";
import { MenuItem } from "@/components/Popover";
import { api } from "@/lib/api";
import { useBots } from "@/lib/bots";
import { copyText } from "@/lib/clipboard";
import { ago } from "@/lib/format";
import { Card, SettingsPage } from "./SettingsView";

export type ChannelsOverview = { here: boolean; origin: string; names: string[]; channels: Channel[]; hooks: Hook[] };

export const useChannels = () => useQuery({ queryKey: ["channels"], queryFn: () => api.get<ChannelsOverview>("/api/channels"), staleTime: 30_000 });

const copy = (text: string, what: string) => void copyText(text).then(() => toast(`${what} copied`), toastError);

/** The page explanation (ⓘ): one paragraph per kind of channel. */
const INTRO = <>
  How agents reach you and other people, and how outside tools reach your agents. Home keeps these. URLs, tokens and secrets are never shown again.
  <span className="mt-1 block"><b>Telegram.</b> Each workspace has its own Telegram bot and chat, set up in Settings → Workspace. Messages to your chat go out directly: that workspace's notifications, the answers to what you send from Telegram, and its agents' <code>launchpad send telegram</code>. Plain text goes to the workspace's Ask bot; <code>/&lt;bot&gt; text</code> gives another of its bots a task. Messages from Telegram never start paid usage.</span>
  <span className="mt-1 block"><b>Outgoing.</b> Agents draft messages with <code>launchpad send &lt;name&gt; --body-file &lt;file&gt;</code>. Drafts wait in Today (Send): you edit them and click Send. Nothing is sent on its own or sent twice.</span>
  <span className="mt-1 block"><b>Incoming hooks.</b> A hook turns a POST request into a message to a bot (Ask Launchpad or another): a short note naming the hook, then the request body (at most 16 KB) as data. One message every 10 seconds per hook; extra requests get 429. Home is a private Space, so callers also send <code>Authorization: Bearer &lt;HF token with read access to the Space&gt;</code>: curl and CI jobs can, GitHub's own webhooks cannot.</span>
</>;

export function ChannelsView() {
  const q = useChannels();
  return (
    <SettingsPage tour="settings.channels" title="Channels" intro={INTRO}>
      {q.isLoading ? <Loading what="channels" /> : q.error ? <ErrorState error={q.error} /> : q.data && <>
        <TelegramPointer />
        <Outgoing channels={q.data.channels} />
        <Incoming hooks={q.data.hooks} origin={q.data.origin} />
      </>}
    </SettingsPage>
  );
}

// ---------------------------------------------------------------- Telegram

/** Telegram is a setting of each workspace: this page only points there. */
function TelegramPointer() {
  return (
    <Card title="Telegram: your own chat" end={<Link to="/settings/workspace" className="btn btn-sm no-underline">Open Workspace settings</Link>}>
      <span className="text-[13px] text-ink-2">Each workspace has its own Telegram bot and chat, in Settings → Workspace.</span>
    </Card>
  );
}

// ---------------------------------------------------------------- outgoing

const KINDS: { id: ChannelKind; label: string; placeholder: string; hint: string }[] = [
  { id: "slack", label: "Slack", placeholder: "https://hooks.slack.com/services/…", hint: "A Slack incoming webhook (Slack app → Incoming Webhooks). Sends {\"text\": …}." },
  { id: "discord", label: "Discord", placeholder: "https://discord.com/api/webhooks/…", hint: "A Discord webhook (channel settings → Integrations → Webhooks). Sends {\"content\": …}, without mentions." },
  { id: "generic", label: "Webhook", placeholder: "https://…", hint: "Any HTTPS URL receiving JSON: {\"text\": …, \"title\": …, \"session_id\": …}." },
];

function Outgoing({ channels }: { channels: Channel[] }) {
  const qc = useQueryClient();
  const refresh = () => qc.invalidateQueries({ queryKey: ["channels"] });
  const [open, setOpen] = useState(false);
  const [name, setName] = useState("");
  const [kind, setKind] = useState<ChannelKind>("slack");
  const [url, setUrl] = useState("");
  const [busy, setBusy] = useState(false);
  const k = KINDS.find((x) => x.id === kind)!;
  const save = () => {
    setBusy(true);
    void api.post("/api/channels", { name, kind, url }).then(() => { toast("Saved on Home"); setOpen(false); setName(""); setUrl(""); return refresh(); }, toastError).finally(() => setBusy(false));
  };
  return (
    <Card title="Outgoing: other people" end={<Link to="/" search={{ filter: "send" }} className="text-[12px] text-teal-ink no-underline hover:underline">Drafts to send in Today</Link>}>
      {channels.length === 0 && <Empty icon="send">No outgoing channel yet.</Empty>}
      {channels.map((c) => (
        <div key={c.id} className="flex flex-wrap items-center gap-2 text-[13px]">
          <span className="min-w-0 flex-1"><b className="font-mono">{c.name}</b> <span className="text-[12px] text-ink-3">· {KINDS.find((x) => x.id === c.kind)?.label} · {c.host} · added {ago(c.created_at)}</span></span>
          <ConfirmButton icon="trash" consequence={`Deletes ${c.name} and its URL from Home. Its drafts stay in Today but cannot be sent.`} confirmLabel="Delete" onConfirm={() => api.delete(`/api/channels/${c.id}`).then(refresh, toastError)}>Delete</ConfirmButton>
        </div>
      ))}
      {!open ? <button className="btn btn-sm w-fit" onClick={() => setOpen(true)}><Icon name="plus" size={12} />Add a channel</button> : (
        <div className="grid gap-2 rounded-card border border-line-2 p-2.5 text-[13px]">
          <Choices label="Channel kind">{KINDS.map((x) => <Choice key={x.id} active={kind === x.id} onClick={() => setKind(x.id)}>{x.label}</Choice>)}</Choices>
          <span className="text-[12px] text-ink-3">{k.hint} The URL is a credential: Home stores it in the vault and never shows it again.</span>
          <label className="grid gap-1"><span className="text-ink-2">Name (agents use it)</span><input className="field-input w-[220px] font-mono text-[12px]" value={name} onChange={(e) => setName(e.target.value.toLowerCase().replace(/[^a-z0-9-]/g, "-"))} placeholder="team-chat" /></label>
          <label className="grid gap-1"><span className="text-ink-2">Webhook URL</span><input className="field-input font-mono text-[12px]" type="password" value={url} onChange={(e) => setUrl(e.target.value)} placeholder={k.placeholder} autoComplete="off" /></label>
          <div className="flex gap-2"><button className="btn btn-sm btn-primary" disabled={busy || !name.trim() || !url.trim()} onClick={save}>Save on Home</button><button className="btn btn-sm" onClick={() => setOpen(false)}>Cancel</button></div>
        </div>
      )}
    </Card>
  );
}

// ---------------------------------------------------------------- incoming

function curl(origin: string, id: string, secret: string) {
  return `curl -X POST ${origin}/hooks/${id} \\\n  -H "Authorization: Bearer $HF_TOKEN" \\\n  -H "X-Launchpad-Hook-Secret: ${secret}" \\\n  -H "Content-Type: application/json" \\\n  -d '{"event": "build_failed", "url": "https://…"}'`;
}

function targetName(t: HookTarget, bots: { id: string; settings: { name: string } }[]) {
  return bots.find((b) => b.id === t.id)?.settings.name ?? "a removed bot";
}

function Incoming({ hooks, origin }: { hooks: Hook[]; origin: string }) {
  const navigate = useNavigate();
  const qc = useQueryClient();
  const refresh = () => qc.invalidateQueries({ queryKey: ["channels"] });
  const bots = useBots().data ?? [];
  const [open, setOpen] = useState(false);
  const [name, setName] = useState("");
  const [target, setTarget] = useState("");
  const [created, setCreated] = useState<CreatedHook | null>(null);
  const [busy, setBusy] = useState(false);
  const create = () => {
    setBusy(true);
    const id = target || bots[0]?.id;
    if (!id) return setBusy(false);
    const t: HookTarget = { kind: "bot", id };
    void api.post<CreatedHook>("/api/channels/hooks", { name, target: t }).then((c) => { setCreated(c); setOpen(false); setName(""); return refresh(); }, toastError).finally(() => setBusy(false));
  };
  return (
    <Card title="Incoming: hooks">
      {created && (
        <div className="grid gap-2 rounded-card border border-attn-line bg-attn-soft p-2.5 text-[13px]" role="status">
          <b>Copy the secret of {created.hook.name} now: it is shown only once.</b>
          <span className="flex flex-wrap items-center gap-2"><code className="break-all">{created.secret}</code><button className="btn btn-sm" onClick={() => copy(created.secret, "Secret")}><Icon name="copy" size={12} />Copy secret</button></span>
          <pre className="m-0 overflow-x-auto rounded-control bg-paper p-2 text-[12px]">{curl(origin, created.hook.id, created.secret)}</pre>
          <span className="flex gap-2"><button className="btn btn-sm" onClick={() => copy(curl(origin, created.hook.id, created.secret), "Example")}><Icon name="copy" size={12} />Copy curl example</button><button className="btn btn-sm" onClick={() => setCreated(null)}>Done</button></span>
        </div>
      )}
      {hooks.length === 0 && !created && <Empty icon="info">No incoming hook yet.</Empty>}
      {hooks.map((h) => (
        <div key={h.id} className="group flex flex-wrap items-center gap-2 border-b border-line-2 pb-2 text-[13px] last:border-b-0 last:pb-0">
          <span className="grid min-w-0 flex-1 gap-0.5">
            <span><b>{h.name}</b> <span className="text-[12px] text-ink-3">→ {targetName(h.target, bots)} · {h.count} {h.count === 1 ? "message" : "messages"}{h.last_at ? `, last ${ago(h.last_at)}` : ""}</span></span>
            <code className="break-all text-[12px] text-ink-2">{origin}/hooks/{h.id}</code>
          </span>
          {/* One visible action (Copy URL); the curl example, the last session and Delete in ⋯. */}
          <RowActions label={`More actions for ${h.name}`} menuWidth={260} main={<button className="btn btn-sm" onClick={() => copy(`${origin}/hooks/${h.id}`, "URL")}><Icon name="copy" size={12} />Copy URL</button>} more={(close) => <>
            <MenuItem icon={<Icon name="copy" size={14} />} label="Copy curl example" onClick={() => { close(); copy(curl(origin, h.id, "<secret>"), "Example"); }} />
            {h.last_session && <MenuItem icon={<Icon name="agents" size={14} />} label="Open the last session" onClick={() => { close(); void navigate({ to: "/agents", search: { session: h.last_session! } as never }); }} />}
            <MenuSection><ConfirmButton icon="trash" consequence={`Deletes ${h.name}: its URL stops working.`} confirmLabel="Delete" onConfirm={() => api.delete(`/api/channels/hooks/${h.id}`).then(() => { close(); return refresh(); }, toastError)}>Delete</ConfirmButton></MenuSection>
          </>} />
        </div>
      ))}
      {!open ? <button className="btn btn-sm w-fit" data-tour="channels.add-hook" onClick={() => setOpen(true)}><Icon name="plus" size={12} />Create a hook</button> : (
        <div className="grid gap-2 rounded-card border border-line-2 p-2.5 text-[13px]">
          <div className="flex flex-wrap gap-2">
            <label className="grid gap-1"><span className="text-ink-2">Name</span><input className="field-input" value={name} onChange={(e) => setName(e.target.value)} placeholder="Nightly CI" /></label>
            <label className="grid gap-1"><span className="text-ink-2">Sends to</span>
              <select className="field-input" value={target || bots[0]?.id || ""} onChange={(e) => setTarget(e.target.value)}>
                {bots.map((b) => <option key={b.id} value={b.id}>{b.settings.name}</option>)}
              </select>
            </label>
          </div>
          <div className="flex gap-2"><button className="btn btn-sm btn-primary" disabled={busy || !name.trim() || !bots.length} onClick={create}>Create hook</button><button className="btn btn-sm" onClick={() => setOpen(false)}>Cancel</button></div>
        </div>
      )}
    </Card>
  );
}
