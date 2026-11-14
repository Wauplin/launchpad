// Settings → GitHub: Home's GitHub login and GitHub's rate limits as of Home's last call. Launchpad reads GitHub only
// when something asks (a publication, a suggestion, Clone from GitHub); github.com shows notifications and items.

import { formatReset } from "@/lib/format";
import { useGitHubLimits } from "@/lib/githubLimits";
import { GitHubCard } from "./ProfilesView";
import { Card, ConfigError, SettingsPage } from "./SettingsView";

export function GitHubView() {
  const limits = useGitHubLimits();
  const entries = Object.entries(limits.data ?? {});
  return (
    <SettingsPage title="GitHub" intro="The GitHub account Home uses for Clone from GitHub, publication and suggestions. Notifications, issues and pull requests open on github.com.">
      <ConfigError />
      <GitHubCard />
      <Card title="Rate limits" tour="settings.github-limits">
        {entries.length === 0
          ? <span className="text-[12px] text-ink-3">{limits.isLoading ? "Loading…" : "Not read yet: Home has not called GitHub since it started."}</span>
          : entries.map(([k, v]) => <div key={k} className="text-[13px]">GitHub {k}: {v.remaining} calls left, resets {formatReset(v.reset_at)}</div>)}
      </Card>
    </SettingsPage>
  );
}
