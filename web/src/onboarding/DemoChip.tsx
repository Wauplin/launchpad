// The visible demo frame in the top bar: "Demo · nothing is real" and Exit demo (the purple stripe is CSS, `.lp-demo`).

import { exitDemo } from "@/demo/mode";

export function DemoChip() {
  return (
    <span className="flex items-center gap-1.5" data-tour="demo.chip">
      <span className="rounded-full bg-demo-soft px-2 py-0.5 text-[12px] font-semibold whitespace-nowrap text-demo" role="status">● Demo · nothing is real</span>
      <button type="button" className="btn btn-sm" onClick={() => exitDemo()}>Exit demo</button>
    </span>
  );
}
