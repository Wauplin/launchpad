// Settings → Workspace → Telegram: the workspace's own Telegram bot (its @BotFather token, write-only on Home) and
// the owner's chat bound to it. Plain text goes to the workspace's Ask target, /<bot> to its bots, and only this
// workspace's notifications reach the chat.

import { useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import type { TelegramStatus } from "@/gen/TelegramStatus";
import { ActionButton } from "@/components/ActionButton";
import { Icon } from "@/components/Icon";
import { MenuItem } from "@/components/Popover";
import { toast, toastError } from "@/components/Toast";
import { Badge, ConfirmButton, ErrorState, Loading, MenuSection, MoreMenu, SettingRow, Token } from "@/components/ui";
import { api } from "@/lib/api";
import { actionConfirm, actionPath, runAction } from "@/lib/actions";
import { copyText } from "@/lib/clipboard";
import { when } from "@/lib/format";
import { Card } from "./SettingsView";

/** The workspace's Telegram state; `["channels", …]` follows the `channels` bus topic. */
export const useTelegram = (workspace: string | null | undefined) => useQuery({
  queryKey: ["channels", "telegram", workspace],
  queryFn: () => api.get<TelegramStatus>(actionPath("telegram.set", { workspace: workspace! })),
  enabled: !!workspace, staleTime: 30_000, retry: false,
});

export function TelegramCard({ workspace, name }: { workspace: string; name: string }) {
  const qc = useQueryClient();
  const q = useTelegram(workspace);
  const params = { workspace };
  const set = (next: TelegramStatus) => qc.setQueryData(["channels", "telegram", workspace], next);
  const refresh = () => qc.invalidateQueries({ queryKey: ["channels"] });
  const [token, setToken] = useState("");
  const [busy, setBusy] = useState(false);
  const act = (f: () => Promise<unknown>) => { setBusy(true); void f().catch(toastError).finally(() => setBusy(false)); };
  const t = q.data;
  const badge = t?.bound ? <Badge tone="ok" icon="check">Bound to {t.chat_name ?? "your chat"}</Badge> : t?.configured ? <Badge tone="attn">Not bound</Badge> : undefined;
  return (
    <Card title="Telegram" end={badge} tour="settings.telegram">
      {q.isLoading ? <Loading /> : q.error ? <ErrorState error={q.error} /> : t && <>
        {t.error && <ErrorState error={t.error} />}
        {!t.configured ? (
          <SettingRow label="Bot token" hint={<>Each workspace has its own Telegram bot. In Telegram, open <a href="https://t.me/BotFather" target="_blank" rel="noreferrer">@BotFather</a>, send <code>/newbot</code> and paste the token it gives you. Home checks it with Telegram.</>}>
            <span className="flex flex-wrap gap-2">
              <input className="field-input min-w-[260px] flex-1 font-mono text-[12px]" type="password" aria-label="Telegram bot token" placeholder="123456:ABC…" value={token} onChange={(e) => setToken(e.target.value)} autoComplete="off" />
              <button className="btn btn-sm btn-primary" disabled={busy || !token.trim()} onClick={() => act(() => runAction<TelegramStatus>("telegram.token", params, { token }).then((s) => { set(s); setToken(""); toast("Saved on Home"); }))}>Save on Home</button>
            </span>
          </SettingRow>
        ) : <>
          <div className="flex flex-wrap items-center gap-2 text-[13px]"><span>Bot</span><b>@{t.bot_username}</b></div>
          {t.code && t.link && (
            <div className="grid gap-2 rounded-card border border-line-2 p-2.5 text-[13px]">
              <span>{t.bound ? "To bind another chat" : "To bind your chat"}, open the link on your phone and press <b>Start</b>, or send <code>/start {t.code}</code> to @{t.bot_username}. The first chat that sends this code is bound; every other chat is ignored.</span>
              <span className="flex flex-wrap items-center gap-2">
                <Token>{t.code}</Token>
                <a className="btn btn-sm no-underline" href={t.link} target="_blank" rel="noreferrer"><Icon name="ext" size={12} />Open in Telegram</a>
                <button className="btn btn-sm" onClick={() => void copyText(t.link!).then(() => toast("Link copied"), toastError)}><Icon name="copy" size={12} />Copy link</button>
                {t.code_expires_at && <span className="text-[12px] text-ink-3">Valid until {when(t.code_expires_at)}</span>}
              </span>
            </div>
          )}
          {!t.code && !t.bound && <button className="btn btn-sm w-fit" disabled={busy} onClick={() => act(() => runAction<TelegramStatus>("telegram.code", params).then(set))}>Show a binding code</button>}
          {t.bound && (
            <SettingRow label="Notifications" hint={`Sessions of ${name} only: completed turns, failures, unexpected interruptions, automatic quota and cost stops, automatic resumption. Never code or messages.`}>
              <span className="grid gap-1 text-[13px]">
                <label className="flex items-center gap-2"><input type="checkbox" className="accent-teal" checked={t.notify} disabled={busy} onChange={(e) => act(() => runAction<TelegramStatus>("telegram.set", params, { notify: e.target.checked }).then(set))} />Send notifications to this chat</label>
                <label className="flex items-center gap-2"><input type="checkbox" className="accent-teal" checked={t.titles} disabled={busy || !t.notify} onChange={(e) => act(() => runAction<TelegramStatus>("telegram.set", params, { titles: e.target.checked }).then(set))} />Include session titles</label>
              </span>
            </SettingRow>
          )}
          {/* One visible button (Send test message); Bind another chat and Delete Telegram setup in ⋯. */}
          <div className="flex flex-wrap items-center gap-1">
            {t.bound && <ActionButton name="telegram.test" params={params} onDone={() => toast("Test message sent")} />}
            <MoreMenu label="More Telegram actions" width={280}>{(close) => <>
              {t.bound && !t.code && <MenuItem icon={<Icon name="plus" size={14} />} label="Bind another chat" detail="Shows a new binding code" disabled={busy} onClick={() => { close(); act(() => runAction<TelegramStatus>("telegram.code", params).then(set)); }} />}
              <MenuSection><ConfirmButton icon="trash" consequence={actionConfirm("telegram.remove") ?? ""} confirmLabel="Delete" onConfirm={() => runAction("telegram.remove", params).then(() => { close(); return refresh(); }, toastError)}>Delete Telegram setup</ConfirmButton></MenuSection>
            </>}</MoreMenu>
          </div>
        </>}
      </>}
    </Card>
  );
}
