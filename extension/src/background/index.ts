// Service worker: the only part of the extension that talks to Home or Hugging Face. It holds the credentials, the
// connection state, the event stream and the toolbar badge, and answers the panel and content scripts through the
// runtime messages of shared/messages.ts.

import { HEARTBEAT_ALARM, startEvents, streaming } from "./events";
import { REFRESH_ALARM, refresh } from "./hf";
import { current, load } from "./home";
import { listen } from "./ports";
import { route } from "./router";
import { lockStorage } from "./store";

const ready = (async () => {
  await lockStorage();
  if (await load()) startEvents();
})().catch((e) => console.warn("start", e));

// Listeners right away, so the event that woke the worker reaches them; they answer once the state is loaded.
route(ready);
listen(current, ready);

chrome.runtime.onInstalled.addListener(() => {
  void chrome.sidePanel.setPanelBehavior({ openPanelOnActionClick: true });
});

chrome.alarms.onAlarm.addListener((alarm) => {
  if (alarm.name === REFRESH_ALARM) void refresh().catch((e) => console.warn("token refresh", e));
  if (alarm.name === HEARTBEAT_ALARM) void ready.then(() => { if (current().home && current().phase !== "revoked" && !streaming()) startEvents(); });
});
