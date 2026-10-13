// Settings → Harnesses → Sign-in: whether each harness CLI (and its variants) and the GitHub CLI are installed
// and signed in on the daemon host, with the command to run there once (over SSH on a server).

import { useQuery } from "@tanstack/react-query";
import type { SignInCheck } from "@/gen/SignInCheck";
import { HarnessMark } from "@/components/HarnessMark";
import { Icon } from "@/components/Icon";
import { toast, toastError } from "@/components/Toast";
import { api } from "@/lib/api";
import { Card } from "./SettingsView";

function state(c: SignInCheck): { label: string; tone: string } {
  if (!c.cli_found) return { label: "Not installed", tone: "text-gh-closed" };
  if (c.signed_in === true) return { label: "Signed in", tone: "text-gh-open" };
  if (c.signed_in === false) return { label: "Not signed in", tone: "text-attn" };
  return { label: "Unknown", tone: "text-ink-3" };
}

function copy(text: string) {
  navigator.clipboard.writeText(text).then(() => toast("Command copied"), toastError);
}

export function SignInCard() {
  const q = useQuery({ queryKey: ["sign-in"], queryFn: () => api.get<SignInCheck[]>("/api/setup/sign-in"), staleTime: 60_000 });
  return (
    <Card
      title="Sign-in"
      end={<button className="btn btn-sm" disabled={q.isFetching} onClick={() => void q.refetch()}><Icon name="refresh" size={12} />{q.isFetching ? "Checking…" : "Check again"}</button>}
    >
      <p className="text-[12.5px] text-ink-2">Each harness and the GitHub CLI use their own sign-in on the computer that runs the Launchpad daemon. Run a missing command there once, for example over SSH on a server. Launchpad only reads the status; it never signs in or refreshes a token.</p>
      {q.isLoading && <div className="text-xs text-ink-3">Checking…</div>}
      {q.error && <div className="text-xs text-gh-closed">{q.error.message}</div>}
      <div className="grid rounded-md border border-line">
        {(q.data ?? []).map((c) => {
          const s = state(c);
          const ok = c.cli_found && c.signed_in === true;
          return (
            <div key={`${c.id}/${c.variant ?? ""}`} className="grid gap-1 border-b border-line-2 px-3 py-2 text-[12.5px] last:border-0">
              <div className="flex flex-wrap items-center gap-2">
                {c.id === "github" ? <Icon name="pr" size={14} /> : <HarnessMark harness={c.id} size={14} />}
                <b className="font-[650]">{c.name}</b>
                {c.variant && <span className="text-xs text-ink-3">variant</span>}
                <span className={`ml-auto text-xs font-semibold ${s.tone}`}>{s.label}</span>
              </div>
              {c.detail && <div className="truncate text-xs text-ink-3" title={c.detail}>{c.detail}</div>}
              {!ok && (
                <div className="flex flex-wrap items-center gap-2 text-xs">
                  <span className="text-ink-2">{c.cli_found ? "Run on the daemon host:" : "Install it on the daemon host, then run:"}</span>
                  <code className="rounded bg-chrome px-1.5 py-0.5 font-mono">{c.command}</code>
                  <button className="iconbtn" title="Copy the command" aria-label={`Copy ${c.command}`} onClick={() => copy(c.command)}><Icon name="copy" size={12} /></button>
                </div>
              )}
            </div>
          );
        })}
      </div>
      <p className="text-xs text-ink-3">Variants with their own environment are checked with it. Git push also needs credentials on the daemon host (SSH keys or <code className="font-mono">gh auth setup-git</code>).</p>
    </Card>
  );
}
