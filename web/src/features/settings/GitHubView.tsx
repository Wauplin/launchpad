// Settings → GitHub: Home's GitHub login, how often the inbox is polled, and the repositories with
// notifications but no folder. Those belong to no workspace: no Inbox, count or badge shows them. Add to clones one
// into a workspace; the owner checks the others on GitHub.

import { memo, useEffect, useMemo, useRef, useState } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { Icon } from "@/components/Icon";
import { MenuItem, Popover } from "@/components/Popover";
import { Empty, Loading, RowActions, SettingRow, useSaveState } from "@/components/ui";
import { useInbox } from "@/features/inbox/data";
import { openExternal } from "@/lib/platform";
import { usePhone } from "@/lib/phone";
import { ago } from "@/lib/format";
import { useConfig, useProjects } from "@/lib/queries";
import { useWorkspaces } from "@/lib/workspace";
import { useAddFolder } from "@/features/projects/addFolderStore";
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
      <ReposWithoutFolder />
    </SettingsPage>
  );
}

type Orphan = { repo: string; count: number; last: number };

// Fixed heights for the virtualizer: Add to ▾ and ⋯ stay on the line, also on the phone.
const ROW = 52;
const PHONE_ROW = 64;

const OrphanRow = memo(function OrphanRow({ r, phone }: { r: Orphan; phone: boolean }) {
  const workspaces = useWorkspaces().data ?? [];
  // One visible action (Add to ▾); Open on GitHub in ⋯.
  const add = <Popover viewport align="right" width={240} trigger={({ open, toggle }) => <button className="btn btn-sm" aria-expanded={open} onClick={toggle}>Add to<Icon name="chev" size={12} /></button>}>
    {(close) => <>
      <div className="seclabel px-2 pt-1 pb-0.5">Add {r.repo} to</div>
      {workspaces.map((w) => <MenuItem key={w.id} label={w.name} onClick={() => { close(); useAddFolder.getState().show({ repo: r.repo, workspaceId: w.id }); }} />)}
    </>}
  </Popover>;
  return (
    <div className={`group flex items-center gap-2 border-b border-line-2 px-3 text-[13px] ${phone ? "h-[64px]" : "h-[52px]"}`}>
      <span className="grid min-w-0 flex-1">
        <b className="truncate font-mono text-[12px]">{r.repo}</b>
        <span className="text-[12px] text-ink-3">{r.count} notification{r.count === 1 ? "" : "s"} · {ago(r.last)}</span>
      </span>
      <RowActions label={`More actions for ${r.repo}`} width={phone ? 0 : 104} main={add} more={(close) => <MenuItem icon={<Icon name="ext" size={14} />} label="Open on GitHub" onClick={() => { close(); openExternal(`https://github.com/${r.repo}`); }} />} />
    </div>
  );
});

/** Repositories with notifications and no folder in any workspace, latest activity first. */
function ReposWithoutFolder() {
  const phone = usePhone();
  const row = phone ? PHONE_ROW : ROW;
  const inbox = useInbox();
  const projects = useProjects().data;
  const orphans = useMemo(() => {
    const known = new Set((projects ?? []).flatMap((p) => [p.gh_repo, p.gh_upstream]).filter((r): r is string => !!r).map((r) => r.toLowerCase()));
    const by = new Map<string, Orphan>();
    for (const t of inbox.data?.threads ?? []) {
      if (known.has(t.repo.toLowerCase())) continue;
      const at = Date.parse(t.updated_at) || 0;
      const o = by.get(t.repo) ?? { repo: t.repo, count: 0, last: 0 };
      o.count++;
      o.last = Math.max(o.last, at);
      by.set(t.repo, o);
    }
    return [...by.values()].sort((a, b) => b.last - a.last);
  }, [inbox.data, projects]);
  const ref = useRef<HTMLDivElement>(null);
  const v = useVirtualizer({ count: orphans.length, getScrollElement: () => ref.current, estimateSize: () => row, overscan: 6, useFlushSync: false });
  return (
    <Card title="Repositories without a folder" tour="settings.github-repos">
      <span className="text-[12px] text-ink-3">They have GitHub notifications for you but no folder in any workspace, so no Inbox shows them. Add one to a workspace to see its items there, or check them on GitHub.</span>
      {inbox.isLoading ? <Loading what="notifications" /> : orphans.length === 0 ? <Empty icon="pr">Every repository with notifications has a folder.</Empty> : (
        <div ref={ref} className="overflow-y-auto rounded-control border border-line" style={{ maxHeight: (phone ? 4 : 8) * row + 2 }}>
          <div className="relative" style={{ height: v.getTotalSize() }}>
            {v.getVirtualItems().map((it) => <div key={orphans[it.index].repo} className="absolute inset-x-0" style={{ top: it.start }}><OrphanRow r={orphans[it.index]} phone={phone} /></div>)}
          </div>
        </div>
      )}
    </Card>
  );
}
