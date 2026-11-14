// The visible demo frame in the top bar: "Demo · nothing is real" and Exit demo (the purple stripe is CSS, `.lp-demo`).

import { exitDemo } from "@/demo/mode";

/** The phone's demo frame: a strip over the top bar. `?demo=phone` (screenshots and videos) goes without it. */
export function PhoneDemoStrip() {
  return (
    <div className="flex flex-none items-center gap-2 border-b border-line bg-demo-soft px-3 py-1">
      <span className="min-w-0 flex-1 truncate text-[13px] font-semibold text-demo" role="status">● Demo · nothing is real</span>
      <button type="button" className="btn btn-sm" onClick={() => exitDemo()}>Exit demo</button>
    </div>
  );
}

export function DemoChip() {
  return (
    <span className="flex items-center gap-1.5" data-tour="demo.chip">
      <span className="rounded-full bg-demo-soft px-2 py-0.5 text-[12px] font-semibold whitespace-nowrap text-demo" role="status">● Demo · nothing is real</span>
      <button type="button" className="btn btn-sm" onClick={() => exitDemo()}>Exit demo</button>
    </span>
  );
}
