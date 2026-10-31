// A button that opens the Space settings drawer above the app (see lib/deployment.ts): sign-ins on a Space happen there.
import { openSpaceSettings } from "@/lib/deployment";

export function SpaceSettingsButton({ profile, label = "Open Space settings", primary = false, small = true }: { profile?: string; label?: string; primary?: boolean; small?: boolean }) {
  return (
    <button type="button" className={`btn w-fit${small ? " btn-sm" : ""}${primary ? " btn-primary" : ""}`} title="Opens the Space settings panel above Launchpad" onClick={() => openSpaceSettings(profile)}>
      {label}
    </button>
  );
}
