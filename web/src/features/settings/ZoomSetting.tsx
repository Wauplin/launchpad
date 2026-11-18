import { SettingRow } from "@/components/ui";
import { DEFAULT_ZOOM, ZOOM_LEVELS, useZoom } from "@/lib/zoom";

export function ZoomSetting() {
  const { zoom, setZoom } = useZoom();
  return (
    <SettingRow label="Zoom" hint="Scales the whole interface." scope="device">
      <label className="grid w-full gap-2">
        <span className="font-semibold">{ZOOM_LEVELS[zoom]}%{zoom === DEFAULT_ZOOM ? " (default)" : ""}</span>
        <input aria-label="Interface zoom" type="range" min={0} max={4} step={1} value={zoom} onChange={(e) => setZoom(Number(e.target.value))} className="w-full accent-teal" />
        <span className="flex justify-between text-[12px] text-ink-2">{ZOOM_LEVELS.map((z) => <span key={z}>{z}%</span>)}</span>
      </label>
    </SettingRow>
  );
}
