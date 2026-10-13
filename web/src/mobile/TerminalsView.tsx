// Terminals page. On the phone it sits above the full-page terminal dock (MobileShell); the desktop keeps
// terminals in the bottom dock, so this page only opens it.

import { useEffect } from "react";
import { isPhone } from "@/lib/phone";
import { useLayout } from "@/stores/layout";

export function TerminalsView() {
  const setDock = useLayout((s) => s.setDock);
  useEffect(() => { if (!isPhone) setDock({ open: true }); }, [setDock]);
  if (isPhone) return null;
  return <div className="grid h-full place-items-center p-8 text-center text-ink-2">Terminals open in the dock at the bottom of the window.</div>;
}
