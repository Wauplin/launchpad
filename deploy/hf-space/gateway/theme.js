// Applied before paint: the same device-local theme and text size as the Launchpad app (same origin, same keys).
try {
  if (localStorage.getItem("launchpad-theme") === "dark") document.documentElement.dataset.theme = "dark";
  // `launchpad-zoom` holds a position in [100, 110, 120, 135, 150] %, like web/src/lib/zoom.ts. Phones stay at 100%.
  const zoom = [100, 110, 120, 135, 150][Number(localStorage.getItem("launchpad-zoom") ?? 2)] ?? 120;
  document.documentElement.style.setProperty("--zoom", matchMedia("(max-width: 719px)").matches ? "1" : String(zoom / 100));
} catch { /* Storage can be unavailable in some embedded contexts. */ }
