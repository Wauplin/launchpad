import { managedTraceDestination, externalSetup } from "@/lib/deployment";
// Settings → Hugging Face: the hf CLI of this computer and the optional trace backup to a bucket.

import { useQuery } from "@tanstack/react-query";
import { Icon } from "@/components/Icon";
import { ErrorState, Loading, SettingRow, useSaveState } from "@/components/ui";
import { api } from "@/lib/api";
import { formatAge } from "@/lib/format";
import { externalLinkProps } from "@/lib/platform";
import { useConfig } from "@/lib/queries";
import type { SignInCheck } from "@/gen/SignInCheck";
import { CliCheckRow } from "./ProfilesView";
import { Card, ConfigError, SavedInput, SettingsPage, usePatchConfig } from "./SettingsView";

type BackupInfo = { check: SignInCheck; account: string | null; error: string | null; last_upload: number | null; last_sync?: number | null };

const INSTALL_URL = "https://huggingface.co/docs/huggingface_hub/en/guides/cli";

export function HuggingFaceView() {
  const config = useConfig();
  const patch = usePatchConfig();
  const { state, error, run } = useSaveState();
  const q = useQuery({ queryKey: ["trace-backup"], queryFn: () => api.get<BackupInfo>("/api/setup/trace-backup"), refetchInterval: 60_000 });
  const c = q.data?.check;
  const ready = c ? c.cli_found && c.signed_in === true : null;
  const backup = config.data?.config.trace_backup;
  const namespace = backup?.bucket.includes("/") ? backup.bucket.split("/")[0] : q.data?.account;
  const bucket = backup && (backup.bucket.includes("/") ? backup.bucket : `${namespace ?? "<account>"}/${backup.bucket}`);
  const toggle = (on: boolean) => run(async () => { await patch({ trace_backup_enabled: on }); await q.refetch(); });
  return (
    <SettingsPage tour="settings.trace-sync" title="Hugging Face" intro="Sync agent traces between Launchpad instances through a Hugging Face bucket, using the CLI of this computer.">
      <ConfigError />
      {externalSetup() && <a className="btn w-fit no-underline" href={externalSetup()!} target="_top">Connect in hosting controls</a>}
      <Card title="Hugging Face CLI" end={<button className="btn btn-sm" disabled={q.isFetching} onClick={() => void q.refetch()}><Icon name="refresh" size={12} />Check again</button>}>
        {q.isPending ? <Loading /> : q.isError ? <ErrorState error={q.error} onRetry={() => void q.refetch()} /> : c && (
          <CliCheckRow check={c} icon="huggingface" command={c.cli_found ? c.command : null}>
            {!c.cli_found && <a className="text-xs text-teal-ink underline" {...externalLinkProps(INSTALL_URL)}>Installation instructions</a>}
          </CliCheckRow>
        )}
      </Card>
      <Card title="Trace sync" end={backup?.enabled && namespace && bucket && <a className="btn btn-sm no-underline" {...externalLinkProps(`https://huggingface.co/buckets/${bucket}`)}><Icon name="ext" size={12} />Open bucket</a>}>
        {!backup ? <Loading /> : <>
          <SettingRow label="Sync traces" hint="Off by default. Exports local sessions, prompts, tool output and attachments. Imports traces from other instances for registered folders with matching names." scope="all" state={state} error={error}>
            <label className="flex items-center gap-2 text-[13px]">
              <input type="checkbox" className="accent-teal" checked={backup.enabled} disabled={!backup.enabled && ready === false} onChange={(e) => void toggle(e.target.checked)} />
              Sync traces automatically
            </label>
            {!backup.enabled && ready === false && <span className="text-[12px] text-ink-3">Install the CLI and sign in first.</span>}
          </SettingRow>
          <SettingRow label="Bucket" hint="A bucket of your account, or namespace/name. A missing bucket is created private; an existing bucket keeps its visibility." scope="all">
            {managedTraceDestination() ? <code className="text-xs">{backup.bucket} · Managed by this host</code> : <SavedInput label="Bucket" className="font-mono text-xs" value={backup.bucket} onSave={(v) => patch({ trace_backup_bucket: v.trim() })} />}
          </SettingRow>
          <SettingRow label="Prefix" hint="Optional folder inside the bucket, such as traces. Changing it uploads to the new location and keeps existing copies." scope="all">
            {managedTraceDestination() ? <code className="text-xs">{backup.prefix || "Bucket root"} · Managed by this host</code> : <SavedInput label="Prefix" className="font-mono text-xs" value={backup.prefix} onSave={(v) => patch({ trace_backup_prefix: v.trim() })} />}
          </SettingRow>
          <SettingRow label="Destination" hint="Grouped by folder and sorted by session start time (UTC) across all workspaces and machines, with original harness trace files, launchpad.json and launchpad-events-<first sequence>.jsonl sidecars.">
            <code className="min-w-0 max-w-full whitespace-normal break-all font-mono text-xs">hf://buckets/{bucket}/{backup.prefix ? `${backup.prefix}/` : ""}&lt;folder name&gt;/YYYYMMDD_HHMM_&lt;session UUID without hyphens&gt;/</code>
          </SettingRow>
          {backup.enabled && (
            <SettingRow label="Status" hint="Uploads local traces and imports new or changed traces every minute. Failures retry automatically and never block agents.">
              {q.data?.error ? <ErrorState className="w-full" error={q.data.error} /> : <span className="text-[13px] text-ink-2">{q.data?.last_upload ? `Last upload ${formatAge(q.data.last_upload)} ago` : "Nothing uploaded since Launchpad started"}</span>}
              {q.data?.last_sync && <span className="text-[12px] text-ink-3">Last import check {formatAge(q.data.last_sync)} ago</span>}
            </SettingRow>
          )}
          <p className="m-0 text-[12px] text-ink-3">Folders with the same name share a backup location across workspaces and machines. Renaming a folder reuploads its traces to the new location; you can clean up the old copies in your bucket. Turning backup off or archiving a session leaves uploaded files in the bucket. Missing or cleaned-up native trace files cannot be reconstructed. Imported traces are read only; use New agent here to follow up in the local folder. Workspaces, credentials, profiles, settings and worktrees remain separate. Imported traces are never re-exported.</p>
        </>}
      </Card>
    </SettingsPage>
  );
}
