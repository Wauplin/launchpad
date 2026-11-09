// /setup: the setup checklist page, on both shells. Settings → Onboarding embeds the same checklist.

import { useNavigate } from "@tanstack/react-router";
import { PageHeader } from "@/components/ui";
import { usePhone } from "@/lib/phone";
import { useStatus } from "@/lib/queries";
import { PhoneHeader } from "@/mobile/PhoneHeader";
import { SetupChecklist } from "./SetupChecklist";
import { useSetupSummary } from "./summary";

const INTRO = "Launchpad checks what this machine already has and offers only what is missing. Fixes run only when you click them, in a terminal you can watch.";
const HOME_INTRO = "From an empty Home to your first agent, step by step.";

export function SetupView() {
  const phone = usePhone();
  const navigate = useNavigate();
  const intro = useStatus().data?.role === "home" ? HOME_INTRO : INTRO;
  const left = useSetupSummary().data?.required_left;
  const meta = left == null ? undefined : left ? `${left} required step${left === 1 ? "" : "s"} left` : "Every required step is done";
  return (
    <div className="flex h-full min-h-0 flex-col">
      {phone ? <PhoneHeader title="Setup" subtitle={meta} onBack={() => void navigate({ to: "/settings/onboarding" as never })} details={<p className="m-0 text-[13px] text-ink-2">{intro}</p>} /> : <PageHeader title="Setup" meta={meta} intro={intro} />}
      <div className="min-h-0 flex-1 overflow-y-auto">
        <div className="phone-page-content grid max-w-page grid-cols-[minmax(0,1fr)] gap-4 px-5 py-5">
          <SetupChecklist />
        </div>
      </div>
    </div>
  );
}
