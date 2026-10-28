// /setup: the setup checklist page, on both shells. Settings → Onboarding embeds the same checklist.

import { useNavigate } from "@tanstack/react-router";
import { PageHeader } from "@/components/ui";
import { usePhone } from "@/lib/phone";
import { PhoneHeader } from "@/mobile/PhoneHeader";
import { SetupChecklist } from "./SetupChecklist";

const META = "Launchpad checks what this machine already has and offers only what is missing";

export function SetupView() {
  const phone = usePhone();
  const navigate = useNavigate();
  return (
    <div className="h-full overflow-y-auto">
      {phone ? <PhoneHeader title="Setup" onBack={() => void navigate({ to: "/settings/onboarding" as never })} /> : <PageHeader title="Setup" meta={META} />}
      <div className="phone-page-content mx-auto grid max-w-[820px] grid-cols-[minmax(0,1fr)] gap-4 p-6">
        {phone && <p className="m-0 text-[12px] text-ink-3">{META}.</p>}
        <SetupChecklist />
      </div>
    </div>
  );
}
