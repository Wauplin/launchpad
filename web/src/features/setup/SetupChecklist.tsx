import { externalSetup, connectionSetup } from "@/lib/deployment";
// The setup checklist (onboarding plan, section 8): detect first, never redo. Each item shows what was found and
// where; a satisfied item shows "Already set up" and never offers a fix. Missing parts offer the daemon's fixed
// command (Copy, Run in terminal), a small form or a link. Checks run when the list opens, on Check again and when
// a setup terminal exits. Embedded in /setup and Settings → Onboarding.

import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Link, useNavigate } from "@tanstack/react-router";
import { useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import type { Checklist } from "@/gen/Checklist";
import type { SetupItem } from "@/gen/SetupItem";
import type { MissingPart } from "@/gen/MissingPart";
import type { Profile } from "@/gen/Profile";
import type { Project } from "@/gen/Project";
import { Icon } from "@/components/Icon";
import { Badge, ConfirmButton, ErrorState, FilterChip, FilterChips, Loading } from "@/components/ui";
import { toast, toastError } from "@/components/Toast";
import { api } from "@/lib/api";
import { copyText } from "@/lib/clipboard";
import { nativePath, useHosts } from "@/lib/hosts";
import { useConfig, useProjects } from "@/lib/queries";
import { isDesktopShell, usePushState } from "@/lib/push";
import { useComposer } from "@/stores/composer";
import { useWorkspace } from "@/stores/workspace";
import { useAddFolder } from "@/features/projects/addFolderStore";
import { useOpenTerminal, useTerminals } from "@/features/terminals/lib";
import { NotificationsCard } from "@/features/settings/DevicesView";
import { usePatchConfig } from "@/features/settings/SettingsView";
import { useOnboarding, usePatchOnboarding } from "@/onboarding/data";
import { SUMMARY_KEY } from "./summary";

export const FIRST_PROMPT = "Explain how this repository is organized and how to run its tests.";

const STARTER_PRESETS = [
  { id: "write-tests", label: "Write tests", text: "Add or update tests that cover your change, and run the relevant tests before you finish." },
  { id: "small-commits", label: "Small commits", text: "Commit your work in small, focused steps, each with a short message that says what changed." },
  { id: "local-pr", label: "Open a Local PR when done", text: "When the work is done and committed, open a Local PR with `launchpad pr create` and a clear title and description." },
];

const PAY_CHOICES = [
  { kind: "subscription", label: "A subscription", detail: "Claude Pro/Max (Claude Code) or ChatGPT Plus/Pro (Codex)" },
  { kind: "api_key", label: "An API key", detail: "Billed per use by the agent's vendor" },
  { kind: "provider", label: "Another model provider", detail: "OpenCode, for example with the Hugging Face router" },
];

const checklistKey = (host: string | null) => ["setup-checklist", host ?? ""] as const;

function useChecklist(host: string | null) {
  const qc = useQueryClient();
  return useQuery({
    queryKey: checklistKey(host),
    queryFn: async () => {
      const c = await api.get<Checklist>(`/api/setup/checklist${host ? `?host=${encodeURIComponent(host)}` : ""}`);
      if (!host) qc.setQueryData(SUMMARY_KEY, { required_left: c.required_left });
      return c;
    },
    staleTime: 30_000,
  });
}

/** Re-checks when a setup terminal (fix, install, sign-in) exits or closes. */
function useRecheckOnSetupExit(recheck: () => void) {
  const terminals = useTerminals().data;
  const alive = useMemo(() => (terminals ?? []).filter((t) => t.setup && t.alive).map((t) => t.id), [terminals]);
  const previous = useRef<string[]>(alive);
  const latest = useRef(recheck);
  latest.current = recheck;
  useEffect(() => {
    if (previous.current.some((id) => !alive.includes(id))) latest.current();
    previous.current = alive;
  }, [alive]);
}

export function SetupChecklist() {
  const hosts = useHosts();
  const [host, setHost] = useState<string | null>(null);
  const q = useChecklist(host);
  const qc = useQueryClient();
  const recheck = () => {
    void q.refetch();
    void qc.invalidateQueries({ queryKey: ["config"] });
    void qc.invalidateQueries({ queryKey: ["projects"] });
  };
  useRecheckOnSetupExit(recheck);
  const onboarding = useOnboarding().data;
  const patchOnboarding = usePatchOnboarding();
  const dismissed = onboarding?.dismissed ?? [];
  const machine = host ?? "this computer";
  const items = q.data?.items ?? [];
  const required = items.filter((i) => i.importance === "required");
  const optional = items.filter((i) => i.importance !== "required");
  const hidden = optional.filter((i) => dismissed.includes(`setup:${i.id}`));
  const shown = optional.filter((i) => !dismissed.includes(`setup:${i.id}`));
  const ready = !!q.data && q.data.required_left === 0;
  return (
    <div className="grid gap-3">
      {hosts.length > 0 && (
        <FilterChips label="Machine">
          <FilterChip active={host === null} onClick={() => setHost(null)}>This computer</FilterChip>
          {hosts.map((h) => <FilterChip key={h.host.id} active={host === h.host.id} onClick={() => setHost(h.host.id)} disabled={h.state !== "connected"} title={h.state === "connected" ? undefined : "Not connected"}>{h.host.id}</FilterChip>)}
        </FilterChips>
      )}
      <div className="flex flex-wrap items-center gap-2">
        {q.data ? (
          ready ? (
            <h2 className="m-0 flex items-center gap-2 text-[15px] font-semibold"><span className="grid size-5 place-items-center rounded-full bg-diff-add text-gh-open"><Icon name="check" size={12} /></span>This machine is ready</h2>
          ) : (
            <h2 className="m-0 text-[15px] font-semibold">{required.length - q.data.required_left} of {required.length} required · {machine}</h2>
          )
        ) : <h2 className="m-0 text-[15px] font-semibold">Checking {machine}…</h2>}
        <button type="button" className="btn btn-sm ml-auto" disabled={q.isFetching} onClick={recheck}>{q.isFetching ? <span className="lp-spin" aria-hidden="true" /> : <Icon name="refresh" size={12} />}Check again</button>
      </div>
      {ready && <p className="m-0 text-[13px] text-ink-2">Everything Launchpad needs was found on {machine}. The optional items below add more; nothing here needs to be configured again.</p>}
      {q.error && <ErrorState error={q.error} onRetry={recheck} />}
      {!q.data && q.isFetching && <Loading what="setup checks" />}
      {q.data && <>
        <Section title="Required">{required.map((i) => <ItemCard key={i.id} item={i} host={host} recheck={recheck} />)}</Section>
        <Section title={host ? "Optional" : "Recommended and optional"}>
          {!host && <NotificationsItem />}
          {shown.map((i) => <ItemCard key={i.id} item={i} host={host} recheck={recheck} onHide={i.state === "missing" ? () => void patchOnboarding({ dismiss: [`setup:${i.id}`] }).catch(toastError) : undefined} />)}
          {hidden.length > 0 && (
            <p className="m-0 flex flex-wrap items-center gap-1.5 text-[12px] text-ink-3">Hidden: {hidden.map((i) => i.title).join(", ")}
              <button type="button" className="btn btn-sm btn-ghost" onClick={() => void patchOnboarding({ restore: hidden.map((i) => `setup:${i.id}`) }).catch(toastError)}>Show</button>
            </p>
          )}
        </Section>
        {!host && <FinishCard ready={ready} />}
      </>}
    </div>
  );
}

function Section({ title, children }: { title: string; children: ReactNode }) {
  return <section className="grid gap-2"><div className="seclabel">{title}</div>{children}</section>;
}

const STATE_BADGE = {
  ready: <Badge tone="ok" icon="check">Already set up</Badge>,
  unverified: <Badge tone="neutral" icon="check" title="Configured, but proving it works needs a push. It counts as set up.">Configured, not verified</Badge>,
  missing: <Badge tone="attn">Missing</Badge>,
};

function ItemShell({ title, importance, state, why, onHide, children }: { title: string; importance: SetupItem["importance"]; state: SetupItem["state"]; why: string; onHide?: () => void; children?: ReactNode }) {
  const done = state !== "missing";
  return (
    <section className="overflow-hidden rounded-[10px] border border-line" aria-label={title}>
      <div className="flex flex-wrap items-center gap-2 border-b border-line bg-chrome px-3.5 py-2.5">
        <span className={`grid size-5 flex-none place-items-center rounded-full ${done ? "bg-diff-add text-gh-open" : "border border-line bg-paper text-ink-3"}`} aria-hidden="true">{done && <Icon name="check" size={11} />}</span>
        <b className="text-[13px] font-semibold">{title}</b>
        {importance !== "required" && <span className="text-[11px] text-ink-3">{importance === "recommended" ? "Recommended" : "Optional"}</span>}
        <span className="ml-auto flex items-center gap-1.5">
          {STATE_BADGE[state]}
          {onHide && <button type="button" className="btn btn-sm btn-ghost" title="Hide this optional item" onClick={onHide}>Not now</button>}
        </span>
      </div>
      <div className="grid gap-2.5 p-3.5">
        <p className="m-0 text-[12px] text-ink-3">{why}</p>
        {children}
      </div>
    </section>
  );
}

function FoundList({ item }: { item: SetupItem }) {
  if (item.found.length === 0) return null;
  return (
    <dl className="m-0 grid grid-cols-[minmax(0,140px)_minmax(0,1fr)] gap-x-3 gap-y-1 text-[13px]">
      {item.found.map((f, i) => (
        <div key={i} className="contents">
          <dt className="truncate text-ink-3">{f.label}</dt>
          <dd className="m-0 min-w-0 break-words">{f.value}{f.source && <span className="text-[12px] text-ink-3"> · {f.source}</span>}</dd>
        </div>
      ))}
    </dl>
  );
}

function ItemCard({ item, host, recheck, onHide }: { item: SetupItem; host: string | null; recheck: () => void; onHide?: () => void }) {
  const missing = item.state === "missing";
  // Identity keys are one form; the profile, folder and presets steps have their own controls.
  const identity = item.missing.filter((p) => p.id === "user.name" || p.id === "user.email");
  const parts = item.missing.filter((p) => !identity.includes(p) && !p.id.startsWith("sign_in:") && !(item.id === "profile" && p.id === "create") && !(item.id === "folder" && p.id === "add") && !(item.id === "presets" && p.id === "templates"));
  return (
    <ItemShell title={item.title} importance={item.importance} state={item.state} why={item.why} onHide={onHide}>
      <FoundList item={item} />
      {item.id === "github" && <GitHubExpectations />}
      {/* A satisfied item only explains what Launchpad uses: no fix, change or switch actions. */}
      {missing && <>
        {identity.length > 0 && <IdentityForm keys={identity.map((p) => p.id)} host={host} onSaved={recheck} />}
        {item.id === "profile" && <ProfileStep item={item} host={host} onDone={recheck} />}
        {item.id === "folder" && <FolderStep item={item} host={host} onDone={recheck} />}
        {item.id === "presets" && <PresetsStep onDone={recheck} />}
        {parts.map((p) => <PartRow key={p.id} item={item.id} part={p} host={host} />)}
        {item.id === "devices" && <SettingsLink to="/settings/devices">Open Settings → Devices</SettingsLink>}
        {item.id === "hosts" && <SettingsLink to="/settings/hosts">Open Settings → Machines</SettingsLink>}
        {item.id === "huggingface" && item.missing.some((p) => p.id === "enable") && <SettingsLink to="/settings/huggingface">Open Settings → Hugging Face</SettingsLink>}
      </>}
    </ItemShell>
  );
}

function SettingsLink({ to, children }: { to: string; children: ReactNode }) {
  return <div><Link to={to} className="btn btn-sm no-underline">{children}</Link></div>;
}

/** One missing part: what is missing, and its fixed command with Copy and Run in terminal. */
function PartRow({ item, part, host }: { item: string; part: MissingPart; host: string | null }) {
  const terminal = useOpenTerminal();
  if (!host && externalSetup()) return <div className="grid gap-1.5 rounded-lg border border-line p-3"><b>{part.label}</b><a className="btn btn-sm" href={externalSetup()!} target="_top">Connect in hosting controls</a></div>;
  return (
    <div className="grid gap-1.5 rounded-lg border border-line-2 bg-chrome p-2.5">
      <div className="text-[13px] font-semibold">{part.label}</div>
      {part.detail && <div className="text-[12px] text-ink-2">{part.detail}</div>}
      {part.command && <>
        <code className="block max-h-[120px] overflow-auto rounded-md bg-paper px-2 py-1.5 font-mono text-[12px] break-all whitespace-pre-wrap">{part.command}</code>
        <div className="flex flex-wrap items-center gap-1.5">
          <button type="button" className="btn btn-sm" onClick={() => void copyText(part.command!).then(() => toast("Command copied"))}><Icon name="copy" size={12} />Copy</button>
          {part.runnable && (
            <ConfirmButton tone="publish" icon="term" consequence={part.sudo ? "Opens a visible terminal; type your password there. Launchpad never types it." : "Opens a visible terminal that runs this command. You can read and stop it."} confirmLabel="Run" onConfirm={() => terminal.setupFix(item, part.id, host)}>Run in terminal</ConfirmButton>
          )}
        </div>
      </>}
    </div>
  );
}

function IdentityForm({ keys, host, onSaved }: { keys: string[]; host: string | null; onSaved: () => void }) {
  const [name, setName] = useState("");
  const [email, setEmail] = useState("");
  const [busy, setBusy] = useState(false);
  const askName = keys.includes("user.name");
  const askEmail = keys.includes("user.email");
  const valid = (!askName || name.trim()) && (!askEmail || email.includes("@"));
  const save = async () => {
    setBusy(true);
    try {
      await api.post("/api/setup/git-identity", { host, ...(askName ? { name: name.trim() } : {}), ...(askEmail ? { email: email.trim() } : {}) });
      toast("Saved in your global Git configuration");
      onSaved();
    } catch (e) { toastError(e); } finally { setBusy(false); }
  };
  return (
    <form className="grid gap-2 rounded-lg border border-line-2 bg-chrome p-2.5" onSubmit={(e) => { e.preventDefault(); if (valid) void save(); }}>
      <div className="text-[13px] font-semibold">Your identity for commits</div>
      <div className="text-[12px] text-ink-2">Saved with git config --global. Only the missing {askName && askEmail ? "values are" : "value is"} asked; existing values are never changed.</div>
      {askName && <label className="grid gap-1 text-[12px] text-ink-2">Name<input className="field-input h-[30px] max-w-[360px]" value={name} onChange={(e) => setName(e.target.value)} autoComplete="name" /></label>}
      {askEmail && <label className="grid gap-1 text-[12px] text-ink-2">Email<input className="field-input h-[30px] max-w-[360px]" type="email" value={email} onChange={(e) => setEmail(e.target.value)} autoComplete="email" /></label>}
      <div><button type="submit" className="btn btn-sm btn-primary" disabled={!valid || busy}>Save</button></div>
    </form>
  );
}

function ProfileStep({ item, host, onDone }: { item: SetupItem; host: string | null; onDone: () => void }) {
  const qc = useQueryClient();
  const terminal = useOpenTerminal();
  const profiles = useConfig().data?.config.profiles ?? [];
  const [busy, setBusy] = useState<string | null>(null);
  const create = async (harness: string) => {
    setBusy(harness);
    try {
      const p = await api.post<Profile>("/api/setup/profile-from-login", { harness });
      toast(`Created the profile ${p.name}`);
      await qc.invalidateQueries({ queryKey: ["config"] });
      void qc.invalidateQueries({ queryKey: ["sign-in"] });
      onDone();
    } catch (e) { toastError(e); } finally { setBusy(null); }
  };
  const signIns = item.missing.filter((p) => p.id.startsWith("sign_in:"));
  return <>
    {item.logins.length > 0 && host === null && (
      <div className="grid gap-1.5">
        <div className="text-[13px] font-semibold">Signed-in logins found</div>
        {item.logins.map((l) => (
          <div key={l.harness} className="flex flex-wrap items-center gap-2 rounded-lg border border-line-2 bg-chrome p-2.5 text-[13px]">
            <span className="min-w-0 flex-1"><b>{l.name}</b>{l.detail && <span className="text-[12px] text-ink-3"> · {l.detail}</span>}</span>
            <button type="button" className="btn btn-sm btn-primary" disabled={busy !== null} onClick={() => void create(l.harness)}>Create a profile from this login</button>
          </div>
        ))}
      </div>
    )}
    {signIns.map((p) => {
      const id = p.id.slice("sign_in:".length);
      const profile = profiles.find((v) => v.id === id);
      return (
        <div key={p.id} className="flex flex-wrap items-center gap-2 rounded-lg border border-line-2 bg-chrome p-2.5 text-[13px]">
          <span className="min-w-0 flex-1"><b>{profile?.name ?? id}</b>{p.detail && <span className="text-[12px] text-ink-3"> · {p.detail}</span>}</span>
          {!host && externalSetup() ? <a className="btn btn-sm" href={connectionSetup(id)!} target="_top">Connect in hosting controls</a> : <ConfirmButton tone="publish" icon="term" consequence="Opens a visible terminal with the agent's sign-in command." confirmLabel="Sign in" onConfirm={() => terminal.profileSetup(id, host)}>Sign in</ConfirmButton>}
        </div>
      );
    })}
    {item.missing.some((p) => p.id === "create") && (
      <div className="grid gap-1.5">
        <div className="text-[13px] font-semibold">{item.logins.length > 0 ? "Or: how do you pay for agents?" : "How do you pay for agents?"}</div>
        <div className="grid gap-px rounded-lg border border-line bg-paper p-[5px]">
          {PAY_CHOICES.map((c) => (
            <Link key={c.kind} to={`/settings/profiles?new=${c.kind}` as never} className="flex min-h-[40px] flex-col justify-center rounded-md px-2.5 py-1 no-underline hover:bg-sunk">
              <span className="text-[13px] text-ink">{c.label}</span>
              <span className="text-[12px] text-ink-3">{c.detail}</span>
            </Link>
          ))}
        </div>
        <p className="m-0 text-[12px] text-ink-3">Settings → Profiles then offers Install and Sign in for the chosen agent.</p>
      </div>
    )}
  </>;
}

function FolderStep({ item, host, onDone }: { item: SetupItem; host: string | null; onDone: () => void }) {
  const qc = useQueryClient();
  const showAdd = useAddFolder((s) => s.show);
  const workspace = useWorkspace((s) => s.current);
  const [busy, setBusy] = useState<string | null>(null);
  const add = async (path: string) => {
    setBusy(path);
    try {
      const p = await api.post<Project>("/api/projects", { path: host ? nativePath(path) : path, host, workspace_id: workspace ?? undefined });
      toast(`Added ${p.name}`);
      await qc.invalidateQueries({ queryKey: ["projects"] });
      void qc.invalidateQueries({ queryKey: ["workspaces"] });
      onDone();
    } catch (e) { toastError(e); } finally { setBusy(null); }
  };
  return (
    <div className="grid gap-1.5">
      {item.suggestions.length > 0 && <div className="text-[12px] text-ink-2">Git repositories found in your folders root:</div>}
      {item.suggestions.map((path) => (
        <div key={path} className="flex items-center gap-2 rounded-lg border border-line-2 bg-chrome px-2.5 py-1.5 text-[13px]">
          <Icon name="folder" size={13} />
          <span className="min-w-0 flex-1 truncate" title={path}>{nativePath(path)}</span>
          <button type="button" className="btn btn-sm" disabled={busy !== null} onClick={() => void add(path)}><Icon name="plus" size={12} />Add</button>
        </div>
      ))}
      <div><button type="button" className="btn btn-sm" onClick={showAdd}><Icon name="plus" size={12} />Add folder…</button></div>
    </div>
  );
}

function PresetsStep({ onDone }: { onDone: () => void }) {
  const config = useConfig().data?.config;
  const patch = usePatchConfig();
  const [chosen, setChosen] = useState<string[]>([]);
  const [busy, setBusy] = useState(false);
  const save = async () => {
    if (!config) return;
    setBusy(true);
    try {
      const existing = config.presets;
      const add = STARTER_PRESETS.filter((p) => chosen.includes(p.id) && !existing.some((e) => e.id === p.id));
      await patch({ presets: [...existing, ...add] });
      toast(`Added ${add.length} ${add.length === 1 ? "preset" : "presets"}`);
      onDone();
    } catch (e) { toastError(e); } finally { setBusy(false); }
  };
  return (
    <div className="grid gap-1.5">
      {STARTER_PRESETS.map((p) => (
        <label key={p.id} className="flex items-start gap-2 rounded-lg border border-line-2 bg-chrome p-2.5 text-[13px]">
          <input type="checkbox" className="mt-0.5 accent-teal" checked={chosen.includes(p.id)} onChange={(e) => setChosen((c) => e.target.checked ? [...c, p.id] : c.filter((x) => x !== p.id))} />
          <span className="min-w-0"><b>{p.label}</b><span className="block text-[12px] text-ink-3">{p.text}</span></span>
        </label>
      ))}
      <div><button type="button" className="btn btn-sm btn-primary" disabled={!chosen.length || busy || !config} onClick={() => void save()}>Add selected presets</button></div>
    </div>
  );
}

function GitHubExpectations() {
  return (
    <div className="grid gap-1 rounded-lg border border-line-2 bg-chrome p-2.5 text-[12px] text-ink-2">
      <b className="text-[13px] text-ink">What to expect from GitHub</b>
      <span>Launchpad polls GitHub every 60 seconds (Settings → GitHub) and keeps a local cache, so pages open instantly.</span>
      <span>The status bar warns when fewer than 500 API calls are left in the hour.</span>
      <span>Nothing is posted, merged or changed on GitHub without your click.</span>
    </div>
  );
}

/** Browser notifications are per device: checked here, not by the daemon. */
function NotificationsItem() {
  const push = usePushState().data;
  const notify = useConfig().data?.config.agents.notify;
  const desktop = isDesktopShell();
  if (!push && !desktop) return null;
  const ready = notify !== false && (desktop || !!push?.subscribed);
  return (
    <ItemShell title="Notifications" importance="recommended" state={ready ? "ready" : "missing"} why="Know when an agent finishes or fails, on this device.">
      {ready ? (
        <p className="m-0 text-[13px]">{desktop ? "The desktop app shows notifications through this computer's notification center." : "On for this browser."}</p>
      ) : notify === false ? (
        <SettingsLink to="/settings/notifications">Agent notifications are off for all devices: open Settings → Notifications</SettingsLink>
      ) : <NotificationsCard />}
    </ItemShell>
  );
}

function FinishCard({ ready }: { ready: boolean }) {
  const projects = useProjects().data ?? [];
  const profiles = useConfig().data?.config.profiles ?? [];
  const start = useComposer((s) => s.start);
  const update = useComposer((s) => s.update);
  const navigate = useNavigate();
  const folder = projects.filter((p) => !p.is_temp).sort((a, b) => a.created_at - b.created_at)[0];
  const open = () => {
    if (!folder) return;
    const hadText = !!useComposer.getState().spec.text.trim() && useComposer.getState().open;
    start({ projectId: folder.id });
    if (hadText) toast("Your unsent draft was kept; its prompt was not replaced.");
    else update((s) => ({ ...s, text: FIRST_PROMPT }));
    void navigate({ to: "/agents" });
  };
  return (
    <section className="grid gap-2 rounded-[10px] border border-line p-3.5" aria-label="Launch your first real agent">
      <b className="text-[15px] font-semibold">Launch your first real agent</b>
      <p className="m-0 text-[13px] text-ink-2">A safe first task that reads but changes nothing: “{FIRST_PROMPT}”{folder ? <> It runs in <b>{folder.name}</b>.</> : null}</p>
      {!ready && <p className="m-0 text-[12px] text-ink-3">Finish the required items first{!folder ? ": add a folder" : profiles.length === 0 ? ": create a profile" : ""}.</p>}
      <div><button type="button" className="btn btn-primary" disabled={!folder || profiles.length === 0} onClick={open}><Icon name="play" size={12} />Open the composer</button></div>
    </section>
  );
}
