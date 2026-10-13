import { create } from "zustand";

export type Theme = "light" | "dark";
const key = "launchpad-theme";
const normalize = (value: string | null): Theme => value === "dark" ? "dark" : "light";

function apply(theme: Theme) {
  document.documentElement.dataset.theme = theme;
  // The installed app's title bar follows the top bar (--color-chrome).
  document.querySelector('meta[name="theme-color"]')?.setAttribute("content", theme === "dark" ? "#161b22" : "#f4f6f9");
}

// index.html applies the saved choice before paint. Keep working if storage is blocked.
export const useTheme = create<{ theme: Theme; toggle: () => void }>((set, get) => ({
  theme: normalize(document.documentElement.dataset.theme ?? null),
  toggle: () => {
    const theme = get().theme === "light" ? "dark" : "light";
    apply(theme);
    set({ theme });
    try { localStorage.setItem(key, theme); } catch { /* Current window still switches. */ }
  },
}));

// Other windows of this browser share the choice; other devices never receive it.
window.addEventListener("storage", (event) => {
  if (event.storageArea !== localStorage || (event.key !== key && event.key !== null)) return;
  const theme = normalize(event.newValue);
  apply(theme);
  useTheme.setState({ theme });
});
