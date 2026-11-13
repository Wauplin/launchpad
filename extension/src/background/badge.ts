// The toolbar badge (F4): Today's count of the extension's workspace, amber when something waits for you, grey "…"
// while Home starts or does not answer, nothing when not connected. No Chrome notifications.

import type { ConnectionState } from "../shared/messages";

const AMBER = "#9a6700";
const GREY = "#6e7781";

let shown = "";

export function renderBadge(state: ConnectionState) {
  let text = "";
  let color = GREY;
  let title = "Launchpad";
  const ws = state.workspaces.find((w) => w.id === state.workspace)?.name;
  switch (state.phase) {
    case "online":
      if (state.today) { text = state.today > 99 ? "99+" : String(state.today); color = AMBER; }
      title = state.today === null ? "Launchpad" : `Launchpad${ws ? ` · ${ws}` : ""}: ${state.today ? `${state.today} waiting for you` : "nothing waiting for you"}`;
      break;
    case "connecting":
    case "starting":
      text = "…"; title = "Launchpad: Home is starting"; break;
    case "offline":
      text = "…"; title = "Launchpad: Home does not answer"; break;
    default:
      title = "Launchpad: not connected";
  }
  const key = `${text}|${color}|${title}`;
  if (key === shown) return;
  shown = key;
  void chrome.action.setBadgeText({ text });
  void chrome.action.setBadgeBackgroundColor({ color });
  void chrome.action.setBadgeTextColor?.({ color: "#ffffff" });
  void chrome.action.setTitle({ title });
}
