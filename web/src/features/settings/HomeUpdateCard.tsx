// Settings → Home and runners, on Home: the version its Space installs, Check for updates and Update Home
// (launchpad_setup::home_update).

import { useQueryClient } from "@tanstack/react-query";
import type { HomeUpdate } from "@/gen/HomeUpdate";
import { ActionButton } from "@/components/ActionButton";
import { Loading } from "@/components/ui";
import { ago, when } from "@/lib/format";
import { UpdateHomeButton, useHomeUpdate, versionLabel } from "./HomeUpdate";
import { Card } from "./SettingsView";

function Pending({ u }: { u: HomeUpdate }) {
  const p = u.pending;
  if (!p) return null;
  const done = u.running === p.commit;
  const asked = p.runners_updated.length;
  const runners = !p.update_runners ? "Runners stay as they are: update them below." : done
    ? `${asked ? `${asked} ${asked === 1 ? "runner" : "runners"} asked to update. ` : ""}Runners with another build that connect until ${when(p.requested_at + 86_400_000)} update too.`
    : "The runners update once Home runs it.";
  return <span className="text-[12px] text-ink-2"><b>Update to {versionLabel(u, p.version)}</b> started {ago(p.requested_at)}{done ? ": Home runs it." : "."} {runners}</span>;
}

/** Settings → Home and runners, on Home: the installed version, Check for updates and Update Home. A Home that
 * cannot update itself (not on a Space) shows one line at the bottom of the page instead (`bottom`). */
export function HomeUpdateCard({ bottom = false }: { bottom?: boolean }) {
  const qc = useQueryClient();
  const q = useHomeUpdate();
  const u = q.data;
  const check = <ActionButton<HomeUpdate> name="home.check-update" onDone={(r) => qc.setQueryData(["home-update"], r)} />;
  if (bottom) return u?.unsupported ? <p className="m-0 text-[12px] text-ink-3">Home updates: {u.unsupported}</p> : null;
  if (!u) return <Card title="Home updates"><Loading what="Home updates" /></Card>;
  if (u.unsupported) return null;
  const failed = u.stage && ["BUILD_ERROR", "RUNTIME_ERROR", "CONFIG_ERROR", "NO_APP_FILE"].includes(u.stage);
  return (
    <Card title="Home updates" tour="settings.home-update" end={check}>
      <span className="text-[13px] text-ink-2">
        Home's Space installs <b>{u.deployed ? versionLabel(u, u.deployed) : "an unknown version"}</b> ({u.channel === "main" ? "CI packages of main" : "GitHub releases"}) and runs build <span className="font-mono text-[12px]">{u.running.slice(0, 8)}</span>.
        {u.checked_at ? ` Checked ${ago(u.checked_at)}; Home checks every hour.` : " Home checks every hour."}
      </span>
      {u.error && <span className="text-[12px] text-gh-closed">Check failed: {u.error}</span>}
      {failed && u.pending && u.running !== u.pending.commit && <span className="text-[12px] text-gh-closed">The Space stopped in {u.stage}: Home still runs its previous build. Read the Space's logs on the Hub, then update again or run deploy.sh.</span>}
      {u.building && <span className="text-[12px] font-semibold text-teal-ink">The Space builds the update ({u.stage ?? "starting"}): Home restarts with it in a few minutes.</span>}
      <Pending u={u} />
      {u.latest && u.available && !u.building ? (
        <div className="grid gap-2 rounded-card bg-attn-soft px-3 py-2 text-[12px] text-ink-2">
          <span>
            <b className="text-attn">{versionLabel(u, u.latest.version)} is available</b> ({ago(u.latest.at)}{u.latest.behind != null ? `, ${u.latest.behind} newer ${u.latest.behind === 1 ? "commit" : "commits"} on main` : ""}). <a href={u.latest.url} target="_blank" rel="noreferrer" className="underline">{u.channel === "main" ? "Changes" : "Release notes"}</a>
          </span>
          <span>The Space rebuilds and restarts: Home is unavailable for a few minutes. Agents keep running on their runners.</span>
          <UpdateHomeButton u={u} />
        </div>
      ) : !u.error && !u.building && u.latest && <span className="text-[12px] text-ink-3">Home is up to date.</span>}
    </Card>
  );
}
