import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useVirtualizer } from "@tanstack/react-virtual";
import { memo, useCallback, useEffect, useRef, useState } from "react";
import { api } from "@/lib/api";
import { toastError } from "@/components/Toast";
import { Card, SettingsPage } from "./SettingsView";

type Device = { id: string; name: string; created_at: number; expires_at: number; current: boolean };
type Pair = { url: string; svg: string; expires_at: number };

export function DevicesView() {
  const qc = useQueryClient();
  const devices = useQuery({ queryKey: ["devices"], queryFn: () => api.get<Device[]>("/api/devices"), refetchInterval: 5000 });
  const [pair, setPair] = useState<Pair | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [now, setNow] = useState(Date.now());
  useEffect(() => { const timer = setInterval(() => setNow(Date.now()), 1000); return () => clearInterval(timer); }, []);
  const scroll = useRef<HTMLDivElement>(null);
  const rows = devices.data ?? [];
  const virtualizer = useVirtualizer({ count: rows.length, getScrollElement: () => scroll.current, estimateSize: () => 72, useFlushSync: false });
  const createPair = async () => {
    setBusy("pair");
    try { setPair(await api.post<Pair>("/api/devices/pair")); } catch (error) { toastError(error); }
    finally { setBusy(null); }
  };
  const revoke = useCallback(async (device: Device) => {
    setBusy(device.id);
    try {
      await api.delete(`/api/devices/${device.id}`);
      if (device.current) { location.replace("/auth"); return; }
      await qc.invalidateQueries({ queryKey: ["devices"] });
    } catch (error) { toastError(error); }
    finally { setBusy(null); }
  }, [qc]);
  const remaining = pair ? Math.max(0, Math.ceil((pair.expires_at * 1000 - now) / 1000)) : 0;
  return <SettingsPage>
    <Card title="Pair a device" end={<button className="btn btn-sm btn-primary" disabled={busy !== null} onClick={() => void createPair()}>Create pairing link</button>}>
      <p className="text-sm text-ink-2">Scan the QR code with your phone, or open the link in another browser. Give that browser a name to finish pairing. Each link works once and expires after five minutes.</p>
      {pair && (remaining > 0 ? <div className="grid justify-items-start gap-2">
        <img alt="Scan to pair Launchpad" className="h-64 w-64 max-w-full" src={`data:image/svg+xml;base64,${btoa(pair.svg)}`} />
        <a className="text-sm text-teal-ink underline break-all" href={pair.url} target="_blank" rel="noopener noreferrer">Open pairing link</a>
        <button className="btn btn-sm" onClick={() => void navigator.clipboard.writeText(pair.url).catch(toastError)}>Copy pairing link</button>
        <span className="text-xs text-ink-3" role="status">Expires in {Math.floor(remaining / 60)}:{String(remaining % 60).padStart(2, "0")} · single use</span>
      </div> : <p className="text-sm text-ink-3" role="status">Pairing link expired. Create a new link.</p>)}
    </Card>
    <Card title="Paired browsers">
      <p className="text-sm text-ink-2">Revoke signs out that browser and disconnects its event and terminal viewers. Agents and terminals keep running on the host.</p>
      {devices.error && <p className="text-sm text-attn" role="alert">{devices.error.message}</p>}
      {!devices.data ? <p className="text-sm text-ink-3">Loading…</p> : !rows.length ? <p className="text-sm text-ink-3">No paired browsers.</p> :
        <div ref={scroll} className="overflow-y-auto" style={{ height: Math.min(rows.length * 72, 360) }}>
          <div style={{ height: virtualizer.getTotalSize(), position: "relative" }}>
            {virtualizer.getVirtualItems().map((item) => {
              const device = rows[item.index]!;
              return <DeviceRow key={device.id} device={device} height={item.size} top={item.start} busy={busy !== null} onRevoke={revoke} />;
            })}
          </div>
        </div>}
    </Card>
  </SettingsPage>;
}

const DeviceRow = memo(function DeviceRow({ device, height, top, busy, onRevoke }: { device: Device; height: number; top: number; busy: boolean; onRevoke: (device: Device) => Promise<void> }) {
  return <div className="absolute left-0 top-0 flex w-full items-center gap-3 border-b border-line-2" style={{ height, transform: `translateY(${top}px)` }}>
    <div className="min-w-0 flex-1"><div className="truncate text-sm font-semibold">{device.name}{device.current && <span className="ml-2 text-xs font-normal text-ink-3">This browser</span>}</div>
      <div className="text-xs text-ink-3">Paired {new Date(device.created_at * 1000).toLocaleDateString()} · expires {new Date(device.expires_at * 1000).toLocaleDateString()}</div></div>
    <button className="btn btn-sm" disabled={busy} onClick={() => void onRevoke(device)}>{device.current ? "Sign out" : "Revoke"}</button>
  </div>;
});
