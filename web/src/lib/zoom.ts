export const ZOOM_LEVELS = [100, 110, 120, 135, 150];
export const zoomScale = (position: number) => (ZOOM_LEVELS[Math.max(0, Math.min(4, position))] ?? 120) / 100;
