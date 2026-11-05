// Channels part of the demo world: a Slack channel with two agent drafts in the Outbox (one to send, one sent),
// a bound Telegram chat and one incoming hook to the Maintainer bot. No URL, token or secret exists.

import type { Channel } from "@/gen/Channel";
import type { Hook } from "@/gen/Hook";
import type { OutboxMessage } from "@/gen/OutboxMessage";
import type { TelegramStatus } from "@/gen/TelegramStatus";
import { clock } from "../daemon";
import { BOT } from "./bots";

export function channelsWorld() {
  const channels: Channel[] = [{ id: "demo-ch-team", name: "team-chat", kind: "slack", host: "hooks.slack.com", created_at: clock.ago(60 * 24 * 3) }];
  const outbox: OutboxMessage[] = [
    {
      id: "demo-ob-release", channel_id: "demo-ch-team", channel: "team-chat", kind: "slack", title: "storefront 2.4 is ready for QA",
      body: "The saved-carts branch is merged on main. QA can start on staging after the 14:00 deploy. Known issue: the cart badge does not update until a page reload (fix in review).",
      session_id: "demo-s-bot-maintainer", author: "Maintainer", created_at: clock.ago(25), status: "draft", sent_at: null, error: null,
    },
    {
      id: "demo-ob-ci", channel_id: "demo-ch-team", channel: "team-chat", kind: "slack", title: null,
      body: "CI is green again on storefront: the flaky checkout test now waits for the payment request.",
      session_id: "demo-s-checkout", author: "Claude Code", created_at: clock.ago(60 * 20), status: "sent", sent_at: clock.ago(60 * 19), error: null,
    },
  ];
  const telegram: TelegramStatus = { configured: true, bot_username: "acme_launchpad_bot", bound: true, chat_name: "@demo-user", code: null, link: null, code_expires_at: null, notify: true, titles: false, error: null };
  const hooks: Hook[] = [{ id: "demo-hook-ci", name: "Nightly CI", target: { kind: "bot", id: BOT }, created_at: clock.ago(60 * 24 * 2), last_at: clock.ago(60 * 9), count: 4, last_session: "demo-s-bot-maintainer" }];
  return { channels, outbox, telegram, hooks };
}
