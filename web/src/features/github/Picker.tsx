import { isPhone } from "@/lib/phone";
// Checkbox pickers for labels and reviewers, filled from the repository data already in cache.db.

import { useQuery } from "@tanstack/react-query";
import { useVirtualizer } from "@tanstack/react-virtual";
import { useMemo, useRef, useState, type ReactNode } from "react";
import { Popover } from "@/components/Popover";
import { api } from "@/lib/api";
import type { Label } from "./types";

export type RepoMeta = { labels: Label[]; reviewers: string[] };
export function useRepoMeta(repo: string) {
  return useQuery({ queryKey: ["repo-meta", repo], queryFn: () => api.get<RepoMeta>(`/api/repo-meta/${repo}`), staleTime: 300_000 });
}

/** A searchable checkbox list. `allowNew` offers the typed text when it is not in the list. */
export function Picker({ label, options, selected, onChange, allowNew, render, disabled }: { label: string; options: string[]; selected: string[]; onChange: (next: string[]) => void; allowNew?: string; render?: (name: string) => ReactNode; disabled?: boolean }) {
  return (
    <Popover width={250} trigger={({ open, toggle }) => <button className="btn btn-sm mt-2 w-full justify-between" disabled={disabled} aria-expanded={open} onClick={toggle}>{label} <span>▾</span></button>}>
      {() => <List options={options} selected={selected} onChange={onChange} allowNew={allowNew} render={render} />}
    </Popover>
  );
}

function List({ options, selected, onChange, allowNew, render }: { options: string[]; selected: string[]; onChange: (next: string[]) => void; allowNew?: string; render?: (name: string) => ReactNode }) {
  const [query, setQuery] = useState("");
  const all = useMemo(() => [...new Set([...selected, ...options])], [options, selected]);
  const q = query.trim().toLowerCase();
  const rows = q ? all.filter((o) => o.toLowerCase().includes(q)) : all;
  const ref = useRef<HTMLDivElement>(null);
  const v = useVirtualizer({ count: rows.length, getScrollElement: () => ref.current, estimateSize: () => isPhone ? 44 : 28, overscan: 6, useFlushSync: false });
  const toggle = (name: string) => onChange(selected.includes(name) ? selected.filter((s) => s !== name) : [...selected, name]);
  return (
    <div className="grid gap-1">
      <input className="field-input h-7" aria-label="Search" placeholder="Search…" value={query} onChange={(e) => setQuery(e.target.value)} />
      {allowNew && q && !all.some((o) => o.toLowerCase() === q) && <button className="rounded px-2 py-1 text-left text-[12px] text-teal-ink hover:bg-select" onClick={() => { toggle(query.trim()); setQuery(""); }}>{allowNew} “{query.trim()}”</button>}
      <div ref={ref} className="overflow-auto" style={{ height: Math.min(240, rows.length * (isPhone ? 44 : 28)) }}>
        <div className="relative" style={{ height: v.getTotalSize() }}>
          {v.getVirtualItems().map((r) => {
            const name = rows[r.index];
            return (
              <label key={name} className={`absolute left-0 flex ${isPhone ? "h-11" : "h-7"} w-full cursor-pointer items-center gap-2 rounded px-1.5 text-[12px] hover:bg-select`} style={{ top: r.start }}>
                <input type="checkbox" checked={selected.includes(name)} onChange={() => toggle(name)} />
                <span className="min-w-0 truncate">{render ? render(name) : name}</span>
              </label>
            );
          })}
        </div>
      </div>
      {!rows.length && <p className="m-0 px-1.5 text-[12px] text-ink-3">{all.length ? "No match." : "Nothing cached for this repository yet."}</p>}
    </div>
  );
}
