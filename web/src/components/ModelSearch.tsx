// Searchable model list for profiles with their own provider (`models_url`), inside a Popover.

import { useState } from "react";
import type { ModelInfo } from "@/gen/ModelInfo";
import { MenuItem } from "./Popover";
import { splitModel } from "./ProviderPicker";

const LIMIT = 50;

export function ModelSearch({ models, loading, error, active, onPick }: { models: ModelInfo[]; loading: boolean; error: string | null; active: string | null; onPick: (id: string) => void }) {
  const [text, setText] = useState("");
  const query = text.trim();
  const words = query.toLowerCase().split(/\s+/).filter(Boolean);
  const matches = models.filter((m) => words.every((w) => `${m.id} ${m.description}`.toLowerCase().includes(w)));
  // A short, unvirtualized list: refine the search to reach the others.
  const shown = matches.slice(0, LIMIT);
  const activeBase = active ? splitModel(active).base : null;
  return (
    <>
      <input autoFocus className="field-input mb-1 h-[30px] text-xs" aria-label="Search models" placeholder="Search, or type a model ID such as org/model:provider" value={text} onChange={(e) => setText(e.target.value)} />
      {loading && <div className="px-2 py-1 text-xs text-ink-3">Loading models…</div>}
      {error && <div className="px-2 py-1 text-xs text-attn">Could not load the model list: {error}</div>}
      {query && !query.includes(" ") && (query.includes("/") || !matches.length) && !models.some((m) => m.id === query) && <MenuItem label={<>Use <code className="font-mono">{query}</code></>} detail="The model ID typed above" onClick={() => onPick(query)} />}
      {shown.map((m) => <MenuItem key={m.id} label={m.display_name || m.id} detail={m.description || undefined} active={activeBase === m.id} onClick={() => onPick(m.id)} />)}
      {matches.length > LIMIT && <div className="px-2 py-1 text-xs text-ink-3">{matches.length - LIMIT} more models: refine the search.</div>}
      {!loading && !error && models.length > 0 && !matches.length && <div className="px-2 py-1 text-xs text-ink-3">No model matches.</div>}
    </>
  );
}
