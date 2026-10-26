// Text size of the side panel, per browser profile; independent of Launchpad's own zoom setting.

export const TEXT_SIZES = [100, 110, 120, 135, 150] as const;
const KEY = "launchpad-extension-text-size";
const DEFAULT = 120;

export function textSize(): number {
  const saved = Number(localStorage.getItem(KEY));
  return (TEXT_SIZES as readonly number[]).includes(saved) ? saved : DEFAULT;
}

export function applyTextSize(size = textSize()) {
  localStorage.setItem(KEY, String(size));
  document.documentElement.style.zoom = String(size / 100);
  // Chrome scales viewport units with zoom: full-height layouts divide by it.
  document.documentElement.style.setProperty("--x-zoom", String(size / 100));
}
