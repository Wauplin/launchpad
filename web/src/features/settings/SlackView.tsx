// Settings → Slack: the Slack profiles agents use with `launchpad slack --profile <handle>`, and which bots of this
// workspace get each one (their `[tools]` in bot.toml, `slack:<handle>`).

import { useQueryClient } from "@tanstack/react-query";
import type { Bot } from "@/gen/Bot";
import type { SlackProfile } from "@/gen/SlackProfile";
import { Empty, ErrorState, Loading } from "@/components/ui";
import { toastError } from "@/components/Toast";
import { runAction } from "@/lib/actions";
import { useBots } from "@/lib/bots";
import { useSlackProfiles } from "@/lib/slack";
import { defaultOn, slackKey, useLayers } from "@/lib/toolDefaults";
import { useScope } from "@/lib/workspace";
import { Card, SettingsPage } from "./SettingsView";
import { SlackProfiles } from "./SlackSetup";

export function SlackView() {
  const profiles = useSlackProfiles();
  const n = profiles.data?.length ?? 0;
  return (
    <SettingsPage
      title="Slack"
      meta={profiles.data ? `${n} profile${n === 1 ? "" : "s"}` : undefined}
      intro="Agents use Slack with launchpad slack --profile <handle>, the same commands as slackline. Home runs each call with the profile's token: agents and runners never hold it. A profile can do what its token allows, in the name the token acts in (you, or the app's bot). Sessions get a profile only when it is on by default or you switch it on for a launch, a bot or a default."
    >
      <Card title="Profiles" tour="settings.slack">
        {profiles.isLoading ? <Loading what="Slack profiles" /> : profiles.error ? <ErrorState error={profiles.error} /> : <SlackProfiles />}
      </Card>
      {n > 0 && <BotAccess profiles={profiles.data ?? []} />}
    </SettingsPage>
  );
}

/** Which bots of this workspace get each profile, changed in place (the bot's `[tools]`). */
function BotAccess({ profiles }: { profiles: SlackProfile[] }) {
  const { workspace } = useScope();
  const bots = (useBots().data ?? []).filter((b) => b.workspace_id === workspace.id);
  return (
    <Card title={`Bots of ${workspace.name || "this workspace"}`}>
      {bots.length === 0 ? <Empty icon="info">No bot in this workspace.</Empty> : (
        <div className="overflow-x-auto">
          <table className="w-full border-collapse text-[13px]">
            <thead>
              <tr className="text-left text-[12px] text-ink-3">
                <th className="py-1 pr-3 font-normal">Bot</th>
                {profiles.map((p) => <th key={p.handle} className="py-1 pr-3 font-normal" title={p.description}>{p.name}</th>)}
              </tr>
            </thead>
            <tbody>{bots.map((b) => <BotRow key={b.id} bot={b} profiles={profiles} />)}</tbody>
          </table>
        </div>
      )}
      <span className="text-[12px] text-ink-3">A bot gets a profile from its own settings, or from the profile's default. Other agents follow the profile's default, the workspace's and folder's default tools, and the composer's Tools.</span>
    </Card>
  );
}

function BotRow({ bot, profiles }: { bot: Bot; profiles: SlackProfile[] }) {
  const qc = useQueryClient();
  const own = bot.settings.tools ?? {};
  const layers = useLayers(bot.workspace_id, bot.settings.folder ?? null, undefined, true);
  const set = async (key: string, v: string) => {
    const tools = { ...own };
    if (v === "") delete tools[key];
    else tools[key] = v === "on";
    try {
      await runAction("bot.update", { id: bot.id }, { set: { tools } });
      await qc.invalidateQueries({ queryKey: ["bots"] });
    } catch (e) {
      toastError(e);
    }
  };
  return (
    <tr className="border-t border-line-2">
      <td className="py-1.5 pr-3">{bot.settings.name}</td>
      {profiles.map((p) => {
        const key = slackKey(p.handle);
        const current = own[key] as boolean | undefined;
        const inherited = defaultOn(layers, key, p.default_on);
        return (
          <td key={p.handle} className="py-1.5 pr-3">
            <select className="field-input w-auto" aria-label={`${p.name} for ${bot.settings.name}`} value={current === undefined ? "" : current ? "on" : "off"} onChange={(e) => void set(key, e.target.value)}>
              <option value="">{inherited ? "Default (on)" : "Default (off)"}</option>
              <option value="on">On</option>
              <option value="off">Off</option>
            </select>
          </td>
        );
      })}
    </tr>
  );
}
