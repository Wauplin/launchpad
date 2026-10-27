import { useQuery } from "@tanstack/react-query";
import { Loading, SettingRow, useSaveState } from "@/components/ui";
import { api } from "@/lib/api";
import { useConfig } from "@/lib/queries";
import type { SignInCheck } from "@/gen/SignInCheck";
import { Card, ConfigError, SavedInput, SettingsPage, usePatchConfig } from "./SettingsView";

export function HuggingFaceView() {
  const config = useConfig();
  const patch = usePatchConfig();
  const { state, error, run } = useSaveState();
  const q = useQuery({ queryKey: ["trace-backup"], queryFn: () => api.get<{ check: SignInCheck; error: string | null }>("/api/setup/trace-backup"), refetchInterval: 60_000 });
  const c = q.data?.check;
  const backup = config.data?.config.trace_backup;
  return (
    <SettingsPage title="Hugging Face" intro="Automatically save agent traces to a Hugging Face bucket from this computer.">
      <ConfigError />
      <Card title="Hugging Face CLI" end={<button className="btn" disabled={q.isFetching} onClick={() => void q.refetch()}>Check again</button>}>
        {q.isPending && <Loading />}
        {q.isError && <p role="alert">Could not check the CLI. Try Check again.</p>}
        {c && <>
          <p className="m-0 text-[13px]">{!c.cli_found ? "hf is not installed." : c.signed_in ? "Signed in" : "Sign-in check failed"}{c.detail && ` · ${c.detail}`}</p>
          {!c.cli_found && <p className="m-0 text-[13px]">Install the CLI on the computer running Launchpad: <a href="https://huggingface.co/docs/huggingface_hub/en/guides/cli" target="_blank" rel="noreferrer">installation instructions</a>.</p>}
          {!c.signed_in && <p className="m-0 text-[13px]">Run <code>hf auth login</code> there, then verify with <code>hf auth whoami</code> and Check again. Use a current CLI with <code>hf buckets</code> support.</p>}
        </>}
      </Card>
      <Card title="Trace backup">
        {!backup ? <Loading /> : <>
          <SettingRow label="Save traces automatically" hint="Off by default. Includes existing sessions, prompts, tool output and attachments. New buckets are private; existing buckets keep their visibility." scope="all" state={state} error={error}>
            <input type="checkbox" aria-label="Save traces automatically" checked={backup.enabled} onChange={(e) => void run(() => patch({ trace_backup_enabled: e.target.checked }))} />
          </SettingRow>
          <SettingRow label="Bucket" hint="A bucket name in your signed-in account, or namespace/name." scope="all">
            <SavedInput label="Bucket name" value={backup.bucket} onSave={(v) => patch({ trace_backup_bucket: v.trim() })} />
          </SettingRow>
          <SettingRow label="Folder" hint="Defaults to this machine’s name. Use a different folder for each machine." scope="all">
            <SavedInput label="Folder name" value={backup.folder} onSave={(v) => patch({ trace_backup_folder: v.trim() })} />
          </SettingRow>
          <p className="m-0 break-all text-[12px] text-ink-3">{backup.folder}/&lt;agent type&gt;/&lt;session ID&gt;/events-&lt;first sequence&gt;.jsonl</p>
          <p className="m-0 text-[12px] text-ink-3">Checks for new events every minute. Upload failures retry automatically and never block agents. Local traces remain available. Turning this off leaves uploaded files in the bucket.</p>
          {backup.enabled && q.data?.error && <p className="m-0 text-[13px] text-danger-ink" role="alert">{q.data.error}</p>}
        </>}
      </Card>
    </SettingsPage>
  );
}
