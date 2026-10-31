// Interface zoom, chosen per device (browser local storage): a phone and a laptop on the same daemon need different sizes.

import { create } from "zustand";
import { isPhone } from "./phone";

export const ZOOM_LEVELS = [100, 110, 120, 135, 150];
export const DEFAULT_ZOOM = 2;
export const zoomScale = (position: number) => (ZOOM_LEVELS[Math.max(0, Math.min(4, position))] ?? 120) / 100;

const key = "launchpad-zoom";
const normalize = (value: string | null) => {
  const n = Number(value);
  return value !== null && Number.isInteger(n) && n >= 0 && n <= 4 ? n : DEFAULT_ZOOM;
};
const read = () => { try { return normalize(localStorage.getItem(key)); } catch { return DEFAULT_ZOOM; } };

export const useZoom = create<{ zoom: number; setZoom: (zoom: number) => void }>((set) => ({
  zoom: read(),
  setZoom: (zoom) => {
    set({ zoom });
    try { localStorage.setItem(key, String(zoom)); } catch { /* This window still changes. */ }
  },
}));

// Other windows of this browser follow; other devices keep their own zoom.
window.addEventListener("storage", (event) => {
  if (event.storageArea === localStorage && (event.key === key || event.key === null)) useZoom.setState({ zoom: read() });
});

/** Demo recordings (videos, screenshots) render the normal layout in a larger viewport at a higher zoom. */
export const recordingZoom = () => (window.__lpDemo ? window.__lpRecordingZoom : undefined);

/** Scale of the root `zoom` in this window: the phone shell always uses 100%. */
export const useZoomScale = () => {
  const zoom = useZoom((z) => z.zoom);
  return recordingZoom() ?? (isPhone ? 1 : zoomScale(zoom));
};
