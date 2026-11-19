// The banner of every window while Home is in maintenance (a planned update, `lib/maintenance.ts`): from the status
// Home pushes, so every tab shows it at once; it stays while the connection drops during the restart and goes with
// the new Home's first status.
import { useStatus } from "@/lib/queries";
import { bannerText } from "@/lib/maintenance";
import { when } from "@/lib/format";
import { useConnection } from "./Connection";

export function MaintenanceBanner() {
  const m = useStatus().data?.maintenance;
  const { online } = useConnection();
  const text = bannerText(m, online);
  if (!m || !text) return null;
  return (
    <div role="status" className="flex flex-none items-center gap-2 border-b border-line bg-attn-soft px-5 py-1.5 text-[12px] text-attn">
      <span className="min-w-0 flex-1">{text}</span>
      <span className="flex-none text-ink-3">since {when(m.since)}</span>
    </div>
  );
}
