// Settings → Help → Onboarding: the tours, the chapter list with replay, and the setup checklist. The tour and
// the demo exist only in the desktop layout; phones see the setup checklist only.

import { Link } from "@tanstack/react-router";
import { Icon } from "@/components/Icon";
import { SettingsPage, Card } from "@/features/settings/SettingsView";
import { enterDemo } from "@/demo/mode";
import { usePhone } from "@/lib/phone";
import { CHAPTER_META } from "./chapters/meta";
import { useOnboarding, usePatchOnboarding } from "./data";

export function OnboardingView() {
  const phone = usePhone();
  const seen = useOnboarding().data?.chapters_seen ?? [];
  const patch = usePatchOnboarding();
  const play = (entry: string) => { void patch({ welcome: "tour" }).catch(() => {}); enterDemo(entry); };
  return (
    <SettingsPage title="Onboarding" intro={phone ? "Get this machine ready for real work." : "Learn Launchpad on demo data. Nothing runs for real and nothing is saved, except which chapters you finished."}>
      {!phone && (
        <Card title="Tours">
          <div className="flex flex-wrap gap-2">
            <button type="button" className="btn btn-demo" onClick={() => play("essentials:layout")}><Icon name="play" size={12} />Essentials tour · about 3 min</button>
            <button type="button" className="btn" onClick={() => play("full:layout")}>Full tour · about 10 min</button>
            <button type="button" className="btn" onClick={() => enterDemo("")}>Explore the demo freely</button>
          </div>
          <p className="m-0 text-[12px] text-ink-3">The demo replaces Launchpad in this tab until you exit it: agents replay prerecorded runs, GitHub actions are simulated, and leaving reloads the app.</p>
        </Card>
      )}
      {!phone && (
        <Card title="Chapters">
          <ol className="m-0 grid list-none gap-0.5 p-0">
            {CHAPTER_META.map((c, i) => {
              const done = seen.includes(c.id);
              return (
                <li key={c.id} className="flex items-center gap-2.5 rounded-md px-1 py-1 text-[13px]">
                  <span className={`grid size-5 place-items-center rounded-full text-[11px] ${done ? "bg-demo text-white" : "bg-sunk text-ink-3"}`} aria-label={done ? "Finished" : undefined}>{done ? <Icon name="check" size={11} /> : i + 1}</span>
                  <span className="min-w-0 flex-1 truncate">{c.title}{c.essentials && <span className="ml-2 text-[11px] text-ink-3">Essentials</span>}</span>
                  <span className="text-[12px] text-ink-3 tabular-nums">{c.time}</span>
                  <button type="button" className="btn btn-sm" onClick={() => enterDemo(c.id)}>{done ? "Replay" : "Play"}</button>
                </li>
              );
            })}
          </ol>
        </Card>
      )}
      <Card title="Setup" end={<Link to={"/setup" as never} className="btn btn-sm no-underline">Open</Link>}>
        <p className="m-0 text-[13px] text-ink-2">The setup checklist checks GitHub CLI, Git, agent profiles and a first folder on this machine, and shows only what is still missing.</p>
      </Card>
    </SettingsPage>
  );
}
