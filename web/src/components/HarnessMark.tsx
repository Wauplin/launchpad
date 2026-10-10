import { createContext, useContext, type ReactNode } from "react";
import { useQuery } from "@tanstack/react-query";
import type { HarnessDescriptor } from "@/gen/HarnessDescriptor";
import type { HarnessKind } from "@/gen/HarnessKind";
import { api } from "@/lib/api";

export type Harness = HarnessKind;
const Registry = createContext<HarnessDescriptor[]>([]);
export function HarnessProvider({ children }: { children: ReactNode }) {
  const query = useQuery({ queryKey: ["harnesses"], queryFn: () => api.get<HarnessDescriptor[]>("/api/harnesses"), staleTime: Infinity });
  return <Registry.Provider value={query.data ?? []}>{children}</Registry.Provider>;
}
export const useHarnesses = () => useContext(Registry);
export function useHarnessInfo(id: HarnessKind | null) { return useHarnesses().find((h) => h.id === id); }
export function useHarnessName() {
  const harnesses = useHarnesses();
  return (id: HarnessKind) => harnesses.find((h) => h.id === id)?.name ?? id;
}

/** Metadata and assets come from the Rust registration; new adapters need no UI branches. */
export function HarnessMark({ harness, size = 16, className = "" }: { harness: Harness; size?: number; className?: string }) {
  const descriptor = useHarnessInfo(harness);
  if (!descriptor) return <span title={harness} aria-label={harness} className={`inline-block flex-none ${className}`} style={{ width: size, height: size }}>◇</span>;
  return <img src={`data:image/svg+xml,${encodeURIComponent(descriptor.icon_svg)}`} width={size} height={size} alt={descriptor.name} title={descriptor.name} draggable={false} className={`inline-block flex-none select-none ${className}`} />;
}
