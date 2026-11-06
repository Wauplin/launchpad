import { delegatedAccess, externalSetup } from "@/lib/deployment";
import { SpaceSettingsButton } from "@/components/SpaceSettingsButton";
// Settings → Hugging Face: Home’s login, backup and trace copies, or the hf CLI of this computer.

import { useQuery } from "@tanstack/react-query";
import { Icon } from "@/components/Icon";
import { ErrorState, Loading } from "@/components/ui";
import { api } from "@/lib/api";
import { ago } from "@/lib/format";
import { externalLinkProps } from "@/lib/platform";
import { useStatus } from "@/lib/queries";
import type { SignInCheck } from "@/gen/SignInCheck";
import { CliCheckRow } from "./ProfilesView";
import { Card, ConfigError, SettingsPage } from "./SettingsView";

type LoginInfo = { check: SignInCheck; account: string | null };
type HomeBackup = { destination: string | null; last_complete_at: number | null; error: string | null; credentials_encrypted: boolean; traces_error: string | null };

const INSTALL_URL = "https://huggingface.co/docs/huggingface_hub/en/guides/cli";

export function HuggingFaceView() {
  const role = useStatus().data?.role;
  if (!role) return <Loading what="Home settings" />;
  return role === "home" ? <HomeHuggingFace /> : <CliView />;
}

function HomeHuggingFace() {
  const login = useQuery({ queryKey: ["home-hf-login"], queryFn: () => api.get<LoginInfo>("/api/setup/huggingface"), refetchInterval: 60_000 });
  const backup = useQuery({ queryKey: ["home-backup"], queryFn: () => api.get<HomeBackup>("/api/home/backup"), refetchInterval: 30_000 });
  const bucket = backup.data?.destination?.startsWith("hf://buckets/") ? backup.data.destination.slice("hf://buckets/".length) : null;
  return <SettingsPage tour="settings.trace-sync" title="Hugging Face" intro="Home requires a Hugging Face login and a private backup bucket. Backup is always on.">
    <Card title="Hugging Face login">
      {login.isPending ? <Loading /> : login.isError ? <ErrorState error={login.error} /> : <span className="text-[13px] text-ink-2">{login.data?.check.signed_in ? `Signed in${login.data.account ? ` as ${login.data.account}` : ""}` : "Sign-in required"}</span>}
      {delegatedAccess() ? <a href="/_home/setup" className="btn btn-sm w-fit">Hugging Face sign-in</a> : <span className="text-[12px] text-ink-3">Run hf auth login on Home to connect the account.</span>}
    </Card>
    <Card title="Required Home backup">
      <p className="m-0 text-[13px] text-ink-2">The private bucket stores Home’s database and session records, the vault, Git branch bundles, plain folders and assets. Backup cannot be turned off.</p>
      {backup.data && <span className={`text-[13px] ${backup.data.credentials_encrypted ? "text-gh-open" : "text-attn"}`}>{backup.data.credentials_encrypted ? "Credentials (vault secrets and Home’s own logins) are encrypted with your credential key, kept as the LP_HOME_KEY secret of this Space. Other data is stored as plain files." : "Credentials are not encrypted yet: finish Home setup to turn encryption on."}</span>}
      {backup.isPending ? <Loading /> : backup.isError ? <ErrorState error={backup.error} /> : backup.data?.error ? <ErrorState error={backup.data.error} /> : <span className="text-[13px] text-ink-2">{backup.data?.last_complete_at ? `Last complete backup ${ago(backup.data.last_complete_at)}` : "Waiting for the first complete backup"}</span>}
      <span className="text-[12px] text-ink-3">The bucket mount is managed by the Home deployment.</span>
    </Card>
    <Card title="Traces" end={bucket && <a className="btn btn-sm no-underline" {...externalLinkProps(`https://huggingface.co/buckets/${bucket}`)}><Icon name="ext" size={12} />Open bucket</a>}>
      <p className="m-0 text-[13px] text-ink-2">After each backup, Home copies the original conversation files of every agent (Claude Code JSONL, Codex rollout, OpenCode export) to the bucket, one file per trace, with a <code className="text-xs">launchpad.json</code> next to them. The trace viewer of the bucket opens them.</p>
      <code className="min-w-0 max-w-full whitespace-normal break-all font-mono text-xs">&lt;workspace&gt;/traces/&lt;folder&gt;/YYYYMMDD_HHMM_&lt;session ID&gt;/</code>
      {backup.data?.traces_error && <ErrorState error={backup.data.traces_error} />}
    </Card>
  </SettingsPage>;
}

function CliView() {
  const q = useQuery({ queryKey: ["hf-login"], queryFn: () => api.get<LoginInfo>("/api/setup/huggingface"), refetchInterval: 60_000 });
  const c = q.data?.check;
  return (
    <SettingsPage tour="settings.trace-sync" title="Hugging Face" intro="The Hugging Face CLI of this computer. Home copies agent traces to its backup bucket.">
      <ConfigError />
      {externalSetup() && <SpaceSettingsButton small={false} label="Connect Hugging Face in Space settings" />}
      <Card title="Hugging Face CLI" end={<button className="btn btn-sm" disabled={q.isFetching} onClick={() => void q.refetch()}><Icon name="refresh" size={12} />Check again</button>}>
        {q.isPending ? <Loading /> : q.isError ? <ErrorState error={q.error} onRetry={() => void q.refetch()} /> : c && (
          <CliCheckRow check={c} icon="huggingface" command={c.cli_found ? c.command : null}>
            {!c.cli_found && <a className="text-xs text-teal-ink underline" {...externalLinkProps(INSTALL_URL)}>Installation instructions</a>}
          </CliCheckRow>
        )}
      </Card>
      <Card title="Traces">
        <p className="m-0 text-[13px] text-ink-2">A runner sends the conversation of each agent to Home at the end of every turn. Home copies the original files to its backup bucket, one file per trace, under <code className="text-xs">&lt;workspace&gt;/traces/&lt;folder&gt;/</code>.</p>
      </Card>
    </SettingsPage>
  );
}
