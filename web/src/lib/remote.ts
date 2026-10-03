import { create } from "zustand";
import { persist } from "zustand/middleware";

export const useRemote = create<{ enabled: boolean; set: (enabled: boolean) => void }>()(persist((set) => ({
  enabled: false,
  set: (enabled) => set({ enabled }),
}), { name: "launchpad-remote-browser" }));

if (new URLSearchParams(location.search).get("remote") === "1") useRemote.getState().set(true);
