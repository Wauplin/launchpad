// Applied before paint: the same device-local theme choice as the Launchpad app (same origin).
try {
  if (localStorage.getItem("launchpad-theme") === "dark") document.documentElement.dataset.theme = "dark";
} catch { /* Storage can be unavailable in some embedded contexts. */ }
