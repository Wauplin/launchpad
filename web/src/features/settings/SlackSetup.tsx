// Slack profiles (`crates/core/src/slack.rs`): each one a name, a description, a handle and a Slack token kept on Home.
// Agents use them with `launchpad slack --profile <handle>`; Home runs slackline with the token. Used in Settings → Slack
// and in Home's setup, which suggests a first "Read as me" profile.

import { useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import type { SlackAppLink } from "@/gen/SlackAppLink";
import type { SlackProfile } from "@/gen/SlackProfile";
import type { SlackProfileSave } from "@/gen/SlackProfileSave";
import type { SlackTest } from "@/gen/SlackTest";
import type { SlackTokenKind } from "@/gen/SlackTokenKind";
import { Icon } from "@/components/Icon";
import { MenuItem } from "@/components/Popover";
import { Badge, Choice, Choices, ConfirmButton, MenuSection, RowActions, Token } from "@/components/ui";
import { toast, toastError } from "@/components/Toast";
import { runAction } from "@/lib/actions";
import { api } from "@/lib/api";
import { ago } from "@/lib/format";
import { handleOf, useSlackProfiles, validHandle } from "@/lib/slack";

/** Whether at least one profile is saved (the setup step is done). */
export function slackReady(profiles: SlackProfile[] | undefined): boolean {
  return !!profiles && profiles.length > 0;
}

/** The first profile the setup suggests: read-only, as the owner, like the former read-only Slack tool. */
const SUGGESTED: Draft = { name: "Read as me", handle: "user-read", description: "Searches and reads Slack as me (channels, threads, DMs, people). Read-only: it cannot post.", kind: "user", write: false, default_on: false, footer: true };

type Draft = { name: string; handle: string; description: string; kind: SlackTokenKind; write: boolean; default_on: boolean; footer: boolean };

/** The profiles and Add a profile. `suggest`: the setup's form, filled with the suggested first profile. */
export function SlackProfiles({ suggest }: { suggest?: boolean }) {
  const profiles = useSlackProfiles();
  const [adding, setAdding] = useState(false);
  const list = profiles.data ?? [];
  return (
    <div className="grid grid-cols-[minmax(0,1fr)] gap-2.5">
      {list.map((p) => <ProfileRow key={p.handle} p={p} />)}
      {adding || (suggest && list.length === 0)
        ? <ProfileForm initial={list.length === 0 ? SUGGESTED : undefined} taken={list.map((p) => p.handle)} onClose={() => setAdding(false)} />
        : <button className="btn btn-sm w-fit" onClick={() => setAdding(true)}><Icon name="plus" size={12} />Add a profile</button>}
    </div>
  );
}

function ProfileRow({ p }: { p: SlackProfile }) {
  const qc = useQueryClient();
  const [editing, setEditing] = useState(false);
  const [test, setTest] = useState<SlackTest | null>(null);
  const runTest = () => runAction<SlackTest>("slack.profile-test", { handle: p.handle }).then(setTest, toastError);
  const remove = () => runAction("slack.profile-delete", { handle: p.handle }).then(() => { toast(`Deleted the Slack profile ${p.name}`); return qc.invalidateQueries({ queryKey: ["slack-profiles"] }); }, toastError);
  if (editing) return <ProfileForm profile={p} taken={[]} onClose={() => setEditing(false)} />;
  return (
    <div className="group grid grid-cols-[minmax(0,1fr)] gap-1 rounded-card border border-line-2 bg-chrome p-2.5 text-[13px]">
      <div className="flex flex-wrap items-center gap-2">
        <b className="min-w-0 truncate">{p.name}</b>
        <Token className="font-mono" title="--profile of launchpad slack">{p.handle}</Token>
        <Badge tone={p.token_kind === "user" ? "attn" : "teal"} title={p.token_kind === "user" ? "A user token (xoxp-): it acts as the person who installed the app" : "A bot token (xoxb-): it acts as the app's bot user"}>{p.token_kind === "user" ? "as a person" : "as a bot"}</Badge>
        {test && <Badge tone={test.ok ? "ok" : "attn"} title={test.message}>{test.ok ? "token works" : "token refused"}</Badge>}
        <span className="ml-auto">
          <RowActions label={`Slack profile ${p.name}`} main={<button className="btn btn-sm" onClick={() => setEditing(true)}><Icon name="pencil" size={12} />Edit</button>} more={(close) => (
            <>
              <MenuItem icon={<Icon name="check" size={14} />} label="Test the token" detail="Asks Slack who it acts as" onClick={() => { close(); void runTest(); }} />
              <MenuSection>
                <ConfirmButton icon="trash" consequence={`Deletes the profile and its token from Home. Agents lose it at once. The token stays valid in Slack until you remove the app.`} confirmLabel="Delete" onConfirm={async () => { await remove(); close(); }}>Delete</ConfirmButton>
              </MenuSection>
            </>
          )} />
        </span>
      </div>
      {p.description && <span className="text-[12px] text-ink-2">{p.description}</span>}
      <span className="text-[12px] text-ink-3">Token {p.token_detail}, saved {ago(p.updated_at)} · {p.default_on ? "every session gets it unless switched off" : "off unless switched on for a launch, a bot or a default"} · {p.footer ? "messages end with Launchpad's attribution line" : "no attribution line"}</span>
      {test && !test.ok && <span className="text-[12px] text-attn">{test.message}</span>}
      {test?.ok && <span className="text-[12px] text-ink-3">{test.message}</span>}
    </div>
  );
}

/** A new profile (`initial` fills it) or the changes of `profile`; its token is required only for a new one. */
function ProfileForm({ profile, initial, taken, onClose }: { profile?: SlackProfile; initial?: Draft; taken: string[]; onClose: () => void }) {
  const qc = useQueryClient();
  const start: Draft = profile
    ? { name: profile.name, handle: profile.handle, description: profile.description, kind: profile.token_kind, write: false, default_on: profile.default_on, footer: profile.footer }
    : initial ?? { name: "", handle: "", description: "", kind: "user", write: false, default_on: false, footer: true };
  const [d, setD] = useState<Draft>(start);
  const [handleEdited, setHandleEdited] = useState(!!profile || !!initial);
  const [token, setToken] = useState("");
  const [busy, setBusy] = useState(false);
  const set = (p: Partial<Draft>) => setD((x) => ({ ...x, ...p }));
  const handle = handleEdited ? d.handle : handleOf(d.name);
  const t = token.trim();
  const tokenKind: SlackTokenKind | null = t.startsWith("xoxp-") ? "user" : t.startsWith("xoxb-") ? "bot" : null;
  const handleError = !profile && (!validHandle(handle) ? "Lowercase letters, digits and single dashes, like user-read." : taken.includes(handle) ? "Another profile has this handle." : null);
  const ready = !!d.name.trim() && !handleError && (!!profile || !!tokenKind) && (!t || !!tokenKind);
  const save = async () => {
    setBusy(true);
    try {
      const body: SlackProfileSave = { name: d.name.trim(), description: d.description.trim(), token: t || null, default_on: d.default_on, footer: d.footer };
      await runAction<SlackProfile>("slack.profile-save", { handle }, body);
      toast(`Saved the Slack profile ${body.name}`);
      await qc.invalidateQueries({ queryKey: ["slack-profiles"] });
      onClose();
    } catch (e) {
      toastError(e);
    } finally {
      setBusy(false);
    }
  };
  return (
    <div className="grid grid-cols-[minmax(0,1fr)] gap-2.5 rounded-card border border-line-2 bg-chrome p-2.5 text-[13px]">
      <div className="flex flex-wrap gap-2">
        <label className="grid min-w-[200px] flex-1 gap-1"><span className="text-[12px] text-ink-2">Name</span>
          <input className="field-input" value={d.name} onChange={(e) => set({ name: e.target.value })} placeholder="Read as me" autoFocus={!profile} />
        </label>
        <label className="grid w-[200px] gap-1"><span className="text-[12px] text-ink-2">Handle</span>
          <input className="field-input font-mono text-[12px]" value={handle} disabled={!!profile} onChange={(e) => { setHandleEdited(true); set({ handle: e.target.value }); }} placeholder="user-read" aria-label="Handle" />
        </label>
      </div>
      {handleError && d.name.trim() && <span className="text-[12px] text-attn">{handleError}</span>}
      <label className="grid gap-1"><span className="text-[12px] text-ink-2">Description: what it can do and in whose name. Agents read it to pick a profile.</span>
        <input className="field-input" value={d.description} onChange={(e) => set({ description: e.target.value })} placeholder="Searches and reads Slack as me. Read-only: it cannot post." />
      </label>
      {!profile && <AppHelper d={d} set={set} />}
      <label className="grid gap-1"><span className="text-[12px] text-ink-2">{profile ? `Token (${profile.token_detail}): paste a new one to replace it` : "Token"}</span>
        <input className="field-input w-[360px] max-w-full font-mono text-[12px]" type="password" value={token} onChange={(e) => setToken(e.target.value)} placeholder="xoxp-… or xoxb-…" autoComplete="off" aria-label="Slack token" />
      </label>
      {t && !tokenKind && <span className="text-[12px] text-attn">{t.startsWith("xapp-") ? "An app-level token (xapp-) only opens Socket Mode connections: it cannot read or send messages. Paste the User OAuth Token (xoxp-) or the Bot User OAuth Token (xoxb-) from OAuth & Permissions." : "Paste a User OAuth Token (xoxp-…) or a Bot User OAuth Token (xoxb-…)."}</span>}
      {tokenKind && <span className="text-[12px] text-ink-3">{tokenKind === "user" ? "A user token: agents act as the person who installed the app." : "A bot token: agents act as the app's bot, in the channels it was added to."}</span>}
      <label className="flex items-start gap-2"><input type="checkbox" className="mt-0.5" checked={d.default_on} onChange={(e) => set({ default_on: e.target.checked })} />
        <span>On by default for every session <span className="text-[12px] text-ink-3">(otherwise only for the launches, bots and defaults you switch it on for)</span></span>
      </label>
      <label className="flex items-start gap-2"><input type="checkbox" className="mt-0.5" checked={d.footer} onChange={(e) => set({ footer: e.target.checked })} />
        <span>Add Launchpad's attribution line to messages <span className="text-[12px] text-ink-3">(“Written by … via Launchpad, posted automatically”: useful when an agent posts in your name)</span></span>
      </label>
      <div className="flex justify-end gap-1.5">
        <button className="btn btn-sm btn-ghost" onClick={onClose}>Cancel</button>
        <button className="btn btn-sm btn-primary" disabled={busy || !ready} onClick={() => void save()}>{busy ? "Saving…" : "Save on Home"}</button>
      </div>
    </div>
  );
}

/** Create the Slack app: in whose name and with which access, then the link with the app's manifest and the steps. */
function AppHelper({ d, set }: { d: Draft; set: (p: Partial<Draft>) => void }) {
  const [appName, setAppName] = useState("");
  const name = appName || d.name || "Launchpad";
  const link = useQuery({
    queryKey: ["slack-app-link", name, d.kind, d.write],
    queryFn: () => api.get<SlackAppLink>(`/api/slack/app-link?${new URLSearchParams({ name, kind: d.kind, write: String(d.write) })}`),
    staleTime: Infinity,
  });
  return (
    <details className="grid gap-2 rounded-control border border-line-2 bg-paper p-2" open>
      <summary className="cursor-pointer text-[12px] font-semibold text-ink-2">Create the Slack app for this profile</summary>
      <div className="mt-2 grid gap-2">
        <div className="flex flex-wrap items-center gap-2">
          <span className="w-[90px] text-[12px] text-ink-2">Acts as</span>
          <Choices label="Acts as">
            <Choice active={d.kind === "user"} onClick={() => set({ kind: "user" })}>You (user token)</Choice>
            <Choice active={d.kind === "bot"} onClick={() => set({ kind: "bot" })}>A bot (bot token)</Choice>
          </Choices>
        </div>
        <div className="flex flex-wrap items-center gap-2">
          <span className="w-[90px] text-[12px] text-ink-2">Access</span>
          <Choices label="Access">
            <Choice active={!d.write} onClick={() => set({ write: false })}>Read only</Choice>
            <Choice active={d.write} onClick={() => set({ write: true })}>Read and write</Choice>
          </Choices>
        </div>
        <label className="flex flex-wrap items-center gap-2"><span className="w-[90px] text-[12px] text-ink-2">App name</span>
          <input className="field-input w-[240px] max-w-full" value={appName} onChange={(e) => setAppName(e.target.value)} placeholder={d.name || "Launchpad"} aria-label="Slack app name" />
        </label>
        <span className="text-[12px] text-ink-3">{d.kind === "user"
          ? "A user token sees what you see in Slack, and what it writes appears under your name. Search works only with a user token."
          : "A bot token posts under the app's name, which keeps it clear that an agent wrote it. It reads only the channels the bot was added to, and cannot search."}</span>
        {link.data && (
          <ol className="m-0 grid list-decimal gap-1 pl-5 text-[12px] text-ink-2">
            <li><a className="text-teal-ink hover:underline" href={link.data.url} target="_blank" rel="noreferrer">Open Slack<Icon name="ext" size={12} className="ml-1 inline" /></a>: {link.data.steps[0]?.replace(/^Open Slack: /, "")}</li>
            {link.data.steps.slice(1).map((s) => <li key={s}>{s}</li>)}
          </ol>
        )}
      </div>
    </details>
  );
}
