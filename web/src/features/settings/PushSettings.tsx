// Settings → Devices → Phone notifications: the Firebase service-account key Home wakes the Android app with
// (write-only on Home). A wake-up carries no text: the phone reads its notifications from Home itself.

import { useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import type { PushStatus } from "@/gen/PushStatus";
import { ActionButton } from "@/components/ActionButton";
import { toast, toastError } from "@/components/Toast";
import { Badge, ConfirmButton, ErrorState, Loading, MenuSection, MoreMenu, SettingRow } from "@/components/ui";
import { api } from "@/lib/api";
import { actionConfirm, runAction } from "@/lib/actions";
import { ago } from "@/lib/format";
import { Card } from "./SettingsView";

export function PushCard() {
  const qc = useQueryClient();
  const q = useQuery({ queryKey: ["phone-push"], queryFn: () => api.get<PushStatus>("/api/phone/push"), staleTime: 30_000, retry: false });
  const set = (next: PushStatus) => qc.setQueryData(["phone-push"], next);
  const [key, setKey] = useState("");
  const [busy, setBusy] = useState(false);
  const p = q.data;
  const save = () => {
    setBusy(true);
    void runAction<PushStatus>("push.key", {}, { key }).then((s) => { set(s); setKey(""); toast("Saved on Home"); }, toastError).finally(() => setBusy(false));
  };
  const badge = !p ? undefined : !p.configured ? <Badge tone="attn">No key</Badge> : p.phones > 0 ? <Badge tone="ok" icon="check">{p.phones === 1 ? "1 phone" : `${p.phones} phones`}</Badge> : <Badge>No phone yet</Badge>;
  return (
    <Card title="Phone notifications" end={badge}>
      {q.isLoading ? <Loading /> : q.error ? <ErrorState error={q.error} /> : p && <>
        <p className="m-0 text-[13px] text-ink-2">Home wakes the Android app through Firebase Cloud Messaging when something waits for you. A wake-up carries no title and no text: the phone then reads its notifications from Home.</p>
        {p.last_error && <ErrorState error={p.last_error} />}
        {!p.configured ? (
          <SettingRow label="Service-account key" hint="In the Firebase console of your own project: Project settings → Service accounts → Generate new private key. Paste the whole JSON file. Home keeps it in its vault and never shows it again.">
            <span className="grid gap-2">
              <textarea className="field-input min-h-[96px] w-full font-mono text-[12px]" aria-label="Firebase service-account key (JSON)" placeholder={'{ "type": "service_account", "project_id": "…", … }'} value={key} onChange={(e) => setKey(e.target.value)} autoComplete="off" spellCheck={false} />
              <button className="btn btn-sm btn-primary w-fit" disabled={busy || !key.trim()} onClick={save}>Save on Home</button>
            </span>
          </SettingRow>
        ) : <>
          <div className="grid gap-1 text-[13px]">
            <span>Firebase project <b>{p.project_id}</b></span>
            <span className="break-all text-[12px] text-ink-3">{p.client_email}</span>
            <span className="text-[12px] text-ink-3">{p.last_sent_at ? `Last wake-up ${ago(p.last_sent_at)}` : "No wake-up sent yet"}{p.phones === 0 && " · sign in on the phone and allow notifications"}</span>
          </div>
          {/* One visible button (Send a test wake-up); Delete the key in ⋯. */}
          <div className="flex flex-wrap items-center gap-1">
            <ActionButton name="push.test" onDone={() => { toast("Test wake-up sent"); void q.refetch(); }} />
            <MoreMenu label="More phone notification actions" width={280}>{(close) => <>
              <MenuSection><ConfirmButton icon="trash" consequence={actionConfirm("push.remove") ?? ""} confirmLabel="Delete" onConfirm={() => runAction<PushStatus>("push.remove").then((s) => { close(); set(s); }, toastError)}>Delete the key</ConfirmButton></MenuSection>
            </>}</MoreMenu>
          </div>
        </>}
      </>}
    </Card>
  );
}
