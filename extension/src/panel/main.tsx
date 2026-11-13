// Side panel. It never talks to Home or Hugging Face itself: it shows the worker's connection state (`subscribe`) and
// sends requests (`request`, shared/messages.ts). Its own small UI with Launchpad's color tokens.

import "../styles.css";
import { useEffect, useState } from "react";
import { createRoot } from "react-dom/client";
import type { ConnectionState } from "../shared/messages";
import { subscribe } from "../shared/messages";
import { applyTextSize } from "../textSize";
import { Connected } from "./Connected";
import { Address, FindHome, Revoked, Welcome } from "./Connect";
import { Settings } from "./Settings";

export type Screen = "main" | "address" | "settings";

function Root() {
  const [state, setState] = useState<ConnectionState | null>(null);
  const [screen, setScreen] = useState<Screen>("main");
  useEffect(() => subscribe([], { state: setState }), []);
  if (!state) return <div className="x-app"><div className="x-center"><span className="x-spin" /></div></div>;
  if (screen === "settings") return <Settings state={state} back={() => setScreen("main")} />;
  if (screen === "address") return <Address state={state} back={() => setScreen("main")} />;
  const open = (s: Screen) => setScreen(s);
  if (state.home && state.phase === "revoked") return <Revoked state={state} open={open} />;
  if (state.home) return <Connected state={state} open={open} />;
  if (state.user) return <FindHome state={state} open={open} />;
  return <Welcome state={state} open={open} />;
}

applyTextSize();
document.documentElement.classList.add("x-panel");
if (matchMedia("(prefers-color-scheme: dark)").matches) document.documentElement.dataset.theme = "dark";
createRoot(document.getElementById("root")!).render(<Root />);
