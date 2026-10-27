import { create } from "zustand";

type Toast = { id: number; text: string; tone: "info" | "error"; action?: { label: string; run: () => void } };

type ToastStore = { toasts: Toast[]; push: (t: Omit<Toast, "id">) => void; dismiss: (id: number) => void };

let next = 1;

export const useToasts = create<ToastStore>((set, get) => ({
  toasts: [],
  push: (t) => {
    const id = next++;
    set({ toasts: [...get().toasts, { ...t, id }] });
    setTimeout(() => get().dismiss(id), t.tone === "error" ? 8000 : 5000);
  },
  dismiss: (id) => set({ toasts: get().toasts.filter((t) => t.id !== id) }),
}));

export const toast = (text: string, action?: Toast["action"]) => useToasts.getState().push({ text, tone: "info", action });
export const toastError = (e: unknown) => useToasts.getState().push({ text: e instanceof Error ? e.message : String(e), tone: "error" });

export function Toasts() {
  const { toasts, dismiss } = useToasts();
  return (
    <div className="pointer-events-none fixed bottom-10 left-1/2 z-50 grid -translate-x-1/2 gap-2">
      {toasts.map((t) => (
        <div
          key={t.id}
          className={`pointer-events-auto flex max-w-[560px] items-center gap-3 rounded-lg px-3.5 py-2 text-[13px] shadow-[0_10px_30px_-10px_rgba(23,32,43,.5)] ${
            t.tone === "error" ? "bg-danger text-white" : "bg-graphite text-white"
          }`}
        >
          <span className="min-w-0 break-words">{t.text}</span>
          {t.action && (
            <button
              className="font-semibold text-[#8FD3C8] hover:underline"
              onClick={() => {
                t.action!.run();
                dismiss(t.id);
              }}
            >
              {t.action.label}
            </button>
          )}
          <button className="text-white/60 hover:text-white" title="Dismiss" onClick={() => dismiss(t.id)}>
            ✕
          </button>
        </div>
      ))}
    </div>
  );
}
