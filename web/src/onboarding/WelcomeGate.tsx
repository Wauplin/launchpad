// First launch (desktop layout, real mode only): the welcome dialog on a fresh install, otherwise a one-time
// banner announcing the tour. The answer lives in the onboarding record, so a second browser does not ask again.

import { Link, useNavigate } from "@tanstack/react-router";
import { useState } from "react";
import { Icon, type IconName } from "@/components/Icon";
import { toastError } from "@/components/Toast";
import { enterDemo } from "@/demo/mode";
import { useOnboarding, usePatchOnboarding } from "./data";

export function WelcomeGate() {
  const q = useOnboarding();
  const patch = usePatchOnboarding();
  const o = q.data;
  if (!o || o.welcome) return null;
  if (o.fresh_install) return <WelcomeDialog />;
  if (o.dismissed.includes("banner")) return null;
  return (
    <div role="region" aria-label="Tour announcement" className="flex flex-none flex-wrap items-center gap-2 border-b border-line-2 bg-demo-soft px-3.5 py-1.5 text-[13px] text-demo">
      <Icon name="spark" size={14} />
      <span className="min-w-0 flex-1">New: an interactive tour of Launchpad on demo data.</span>
      <Link to="/settings/onboarding" className="btn btn-sm btn-demo no-underline" onClick={() => void patch({ dismiss: ["banner"] }).catch(toastError)}>Settings → Onboarding</Link>
      <button type="button" className="btn btn-sm btn-ghost" onClick={() => void patch({ dismiss: ["banner"] }).catch(toastError)}>Dismiss</button>
    </div>
  );
}

function Choice({ icon, title, text, action, onClick, primary }: { icon: IconName; title: string; text: string; action: string; onClick: () => void; primary?: boolean }) {
  return (
    <div className="flex items-center gap-3 rounded-[10px] border border-line p-3">
      <span className="grid size-8 flex-none place-items-center rounded-full bg-demo-soft text-demo"><Icon name={icon} size={14} /></span>
      <span className="min-w-0 flex-1"><b className="block text-[13px] font-semibold">{title}</b><span className="text-[12px] text-ink-2">{text}</span></span>
      <button type="button" className={`btn btn-sm ${primary ? "btn-demo" : ""}`} onClick={onClick}>{action}</button>
    </div>
  );
}

function WelcomeDialog() {
  const patch = usePatchOnboarding();
  const navigate = useNavigate();
  const [busy, setBusy] = useState(false);
  const answer = (welcome: "tour" | "skip" | "later", then: () => void) => {
    setBusy(true);
    void patch({ welcome }).then(then).catch((e) => { setBusy(false); toastError(e); });
  };
  return (
    <div className="fixed inset-0 z-[70] grid place-items-center bg-[rgb(10_15_25/0.45)] p-4">
      <div role="dialog" aria-modal="true" aria-labelledby="lp-welcome" className="w-[520px] max-w-full rounded-xl border border-line border-t-[3px] border-t-demo bg-paper p-5 shadow-[0_20px_50px_-15px_rgba(10,15,25,.6)]" aria-busy={busy}>
        <h2 id="lp-welcome" className="m-0 text-[18px] font-semibold">Welcome to Launchpad</h2>
        <p className="mt-1.5 mb-3 text-[13px] text-ink-2">Launchpad starts coding agents, keeps an eye on them and helps you review their work and your GitHub notifications. Take a quick tour on demo data first?</p>
        <div className="grid gap-2">
          <Choice icon="play" primary title="Essentials tour · about 3 min" text="Launch an agent, answer it, review its work. Demo data only." action="Start" onClick={() => answer("tour", () => enterDemo("essentials:layout"))} />
          <Choice icon="agents" title="Full tour · about 10 min" text="Adds GitHub, Ask, terminals, workspaces, automation and settings." action="Start" onClick={() => answer("tour", () => enterDemo("full:layout"))} />
          <Choice icon="sliders" title="Skip the tour" text="Go to the setup checklist. The tour stays in Settings → Onboarding." action="Set up" onClick={() => answer("skip", () => void navigate({ to: "/setup" as never }))} />
        </div>
        <div className="mt-3 flex items-center gap-2 text-[12px] text-ink-3">
          <span className="flex-1">Nothing in the tour runs for real or is saved.</span>
          <button type="button" className="btn btn-sm btn-ghost" onClick={() => answer("later", () => {})}>Not now</button>
        </div>
      </div>
    </div>
  );
}
