import { useSaveStatus } from "../lib/saveStatus";
export function SaveStatus() {
  const errors = useSaveStatus((s) => s.errors);
  if (!Object.keys(errors).length) return null;
  return <div role="alert" className="fixed left-3 right-3 top-2 z-50 max-h-[40vh] overflow-auto rounded-lg border border-attn-line bg-attn-soft p-3 text-sm text-attn">{Object.entries(errors).map(([key, error]) => <div key={key}>{error.text} {error.actions?.map((action) => <button key={action.label} className="btn btn-sm ml-2" onClick={() => void action.run().catch(() => {})}>{action.label}</button>)}</div>)}</div>;
}
