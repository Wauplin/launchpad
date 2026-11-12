// Where a viewport popover goes: under its anchor, or above when there is more room there, kept on screen.

export type Rect = { left: number; right: number; top: number; bottom: number };
export type View = { x: number; y: number; w: number; h: number };

/** Viewport pixels in, CSS pixels out (divided by the root zoom `scale`); null when the anchor left the view. */
export function placePanel(anchor: Rect, view: View, panel: { width: number; scrollHeight: number }, align: "left" | "right", scale: number) {
  const { x, y, w, h } = view;
  if (anchor.bottom < y || anchor.top > y + h) return null;
  const below = Math.max(0, y + h - anchor.bottom - 12);
  const above = Math.max(0, anchor.top - y - 12);
  const height = Math.min(panel.scrollHeight, 480) * scale;
  const upwards = below < height && above > below;
  const maxHeight = Math.min(480 * scale, upwards ? above : below);
  return {
    // DOM rectangles use viewport pixels; fixed CSS offsets inherit the root zoom.
    left: Math.max(x + 8, Math.min(align === "left" ? anchor.left : anchor.right - panel.width, x + w - panel.width - 8)) / scale,
    top: (upwards ? Math.max(y + 8, anchor.top - 4 - Math.min(height, maxHeight)) : anchor.bottom + 4) / scale,
    maxHeight: maxHeight / scale,
    maxWidth: Math.max(0, w - 16) / scale,
  };
}
