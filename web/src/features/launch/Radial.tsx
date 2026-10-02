// Radial harness picker: every Launch button opens two round buttons next to the cursor, with only
// the Claude and Codex logos. The last harness used has a ring.

import type { MouseEvent, ReactNode } from "react";
import { create } from "zustand";
import type { HarnessKind } from "@/gen/HarnessKind";
import { HarnessMark, HARNESS_NAME } from "@/components/HarnessMark";
import { Icon } from "@/components/Icon";
import { LAST_HARNESS_KEY, useComposer, type LaunchContext } from "@/stores/composer";

type Radial = { at: { x: number; y: number } | null; ctx?: LaunchContext; show: (x: number, y: number, ctx?: LaunchContext) => void; hide: () => void };

export const useRadial = create<Radial>((set) => ({
  at: null,
  show: (x, y, ctx) => set({ at: { x, y }, ctx }),
  hide: () => set({ at: null, ctx: undefined }),
}));

function lastHarness(): HarnessKind {
  try {
    return localStorage.getItem(LAST_HARNESS_KEY) === "codex" ? "codex" : "claude";
  } catch {
    return "claude";
  }
}

export function RadialPicker() {
  const { at, ctx, hide } = useRadial();
  const start = useComposer((s) => s.start);
  if (!at) return null;
  const last = lastHarness();
  // Keep the bubbles inside the window.
  const x = Math.min(Math.max(at.x - 64, 12), window.innerWidth - 150);
  const y = Math.min(at.y + 16, window.innerHeight - 96);
  const pick = (h: HarnessKind) => {
    hide();
    start(h, ctx);
  };
  return (
    <div className="fixed inset-0 z-40" onMouseDown={hide}>
      <div className="absolute flex gap-3.5" style={{ left: x, top: y }} onMouseDown={(e) => e.stopPropagation()}>
        {(["claude", "codex"] as HarnessKind[]).map((h) => (
          <button
            key={h}
            title={HARNESS_NAME[h]}
            onClick={() => pick(h)}
            className={`grid size-[54px] place-items-center rounded-full bg-paper shadow-[0_8px_18px_-6px_rgba(23,32,43,.45),0_0_0_4px_rgba(255,255,255,.95)] transition-transform hover:scale-105 ${
              h === last ? (h === "claude" ? "outline-2 outline-offset-[5px] outline-clay" : "outline-2 outline-offset-[5px] outline-[#5b6cff]") : ""
            }`}
          >
            <HarnessMark harness={h} size={30} />
          </button>
        ))}
      </div>
    </div>
  );
}

/** A Launch button: opens the radial picker, then the composer with `ctx` added. */
export function LaunchButton({
  ctx,
  label = "Launch",
  small,
  primary = true,
  title = "Launch an agent",
  children,
}: {
  ctx?: LaunchContext;
  label?: string | null;
  small?: boolean;
  primary?: boolean;
  title?: string;
  children?: ReactNode;
}) {
  const show = useRadial((s) => s.show);
  const onClick = (e: MouseEvent) => {
    e.stopPropagation();
    show(e.clientX, e.clientY, ctx);
  };
  return (
    <button className={`btn ${small ? "btn-sm" : ""} ${primary ? "btn-primary" : ""}`} title={title} onClick={onClick}>
      <Icon name="play" size={small ? 12 : 14} />
      {label}
      {children}
    </button>
  );
}
