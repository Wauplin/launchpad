// Settings → GitHub: the GitHub CLI of this computer and how often the inbox is polled.

import { Loading, SettingRow, useSaveState } from "@/components/ui";
import { useConfig } from "@/lib/queries";
import { useEffect, useState } from "react";
import { GitHubCard } from "./ProfilesView";
import { Card, ConfigError, SettingsPage, usePatchConfig } from "./SettingsView";

export function GitHubView() {
  const q = useConfig();
  const patch = usePatchConfig();
  const { state, error, run } = useSaveState();
  const [text, setText] = useState<string | null>(null);
  useEffect(() => { if (q.data) setText(String(q.data.config.github.poll_interval_seconds)); }, [q.data]);
  const commit = () => {
    const n = Math.max(15, Math.floor(Number(text) || 0));
    setText(String(n));
    if (q.data && n !== q.data.config.github.poll_interval_seconds) void run(() => patch({ github_poll_interval_seconds: n }));
  };
  return (
    <SettingsPage title="GitHub" intro="The GitHub account used by the Inbox, issues, PRs and publication.">
      <ConfigError />
      <GitHubCard />
      <Card title="Inbox">
        {text === null ? <Loading /> : (
          <SettingRow label="Check every" hint="Seconds between notification checks (at least 15)." scope="all" state={state} error={error}>
            <input type="number" min={15} aria-label="Seconds between checks" className="field-input w-24" value={text} onChange={(e) => setText(e.target.value)} onBlur={commit} onKeyDown={(e) => { if (e.key === "Enter") commit(); }} />
            <span className="text-[12px] text-ink-3">seconds</span>
          </SettingRow>
        )}
      </Card>
    </SettingsPage>
  );
}
