import { create } from "zustand";
type Action = { label: string; run: () => Promise<void> };
export const useSaveStatus = create<{ errors: Record<string, { text: string; actions?: Action[] }>; set: (key: string, error?: string, actions?: Action[]) => void }>((set) => ({
  errors: {},
  set: (key, error, actions) => set((s) => { const errors = { ...s.errors }; if (error) errors[key] = { text: error, actions }; else delete errors[key]; return { errors }; }),
}));
