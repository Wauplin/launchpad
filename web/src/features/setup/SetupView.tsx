// /setup: Home's setup page, on both shells. A runner's setup is its own window (features/runner).

import { PageHeader } from "@/components/ui";
import { usePhone } from "@/lib/phone";
import { PhoneHeader } from "@/mobile/PhoneHeader";
import { phoneBack } from "@/mobile/back";
import { HomeSetup } from "./HomeSetup";
import { useSetupSummary } from "./summary";

const INTRO = "From an empty Home to your first agent, step by step.";

export function SetupView() {
  const phone = usePhone();
  const left = useSetupSummary().data?.required_left;
  const meta = left == null ? undefined : left ? `${left} required step${left === 1 ? "" : "s"} left` : "Every required step is done";
  return (
    <div className="flex h-full min-h-0 flex-col">
      {phone ? <PhoneHeader title="Setup" subtitle={meta} onBack={() => phoneBack("/more")} details={<p className="m-0 text-[13px] text-ink-2">{INTRO}</p>} /> : <PageHeader title="Setup" meta={meta} intro={INTRO} />}
      <div className="min-h-0 flex-1 overflow-y-auto">
        <div className="phone-page-content grid max-w-page grid-cols-[minmax(0,1fr)] gap-4 px-5 py-5">
          <HomeSetup />
        </div>
      </div>
    </div>
  );
}
