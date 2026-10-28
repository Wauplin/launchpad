// Short looping replays that keep the running world sessions alive, and a generic reply to follow-up messages.

import { at, edit, final, say, scenario, think, tool } from "../player";

const toggleDiff = `@@ -1,9 +1,17 @@
 import { useTheme } from "../theme";
+import { MoonIcon, SunIcon } from "./icons";

 export function Header() {
-  return <header className="site-header"><Logo /><CartButton /></header>;
+  const { theme, toggle } = useTheme();
+  return (
+    <header className="site-header">
+      <Logo />
+      <button aria-label="Dark mode" aria-pressed={theme === "dark"} onClick={toggle}>
+        {theme === "dark" ? <SunIcon /> : <MoonIcon />}
+      </button>
+      <CartButton />
+    </header>
+  );
 }
`;

export const darkMode = scenario({
  id: "loop-dark-mode",
  folder: "storefront",
  harness: "claude",
  title: "Add a dark-mode toggle to the header",
  timeline: [
    at(0.8, think("Checking how the theme tokens are applied before wiring the toggle.")),
    at(1.5, tool("grep", "rg -n \"data-theme\" src", { seconds: 0.8, output: "src/theme.ts:14:  document.documentElement.dataset.theme = theme;\nsrc/styles/tokens.css:3::root[data-theme=\"dark\"] {" })),
    at(1.5, tool("read", "src/components/Header.tsx", { seconds: 0.6, output: "export function Header() {\n  return <header className=\"site-header\"><Logo /><CartButton /></header>;\n}" })),
    at(1.8, edit("src/components/Header.tsx", toggleDiff)),
    at(1.5, tool("bash", "pnpm test header", { seconds: 3.5, output: " ✓ tests/header.test.tsx (6 tests) 0.81s\n\n Test Files  1 passed (1)\n      Tests  6 passed (6)" })),
    at(1.5, think("Checking contrast of the header links in the dark theme.")),
    at(1.5, tool("bash", "pnpm exec playwright test header.spec.ts --project=chromium", { seconds: 5, output: "Running 4 tests using 2 workers\n  ✓ header › toggles dark mode (1.2s)\n  ✓ header › remembers the choice (0.9s)\n  ✓ header › keeps contrast in dark mode (1.4s)\n  ✓ header › keyboard focus is visible (0.7s)\n\n  4 passed (4.6s)" })),
  ],
});

export const pdfSpeed = scenario({
  id: "loop-pdf",
  folder: "billing-api",
  harness: "opencode",
  title: "Speed up invoice PDF rendering",
  timeline: [
    at(0.8, think("Profiling where the PDF renderer spends its time.")),
    at(1.5, tool("bash", "cargo bench --bench pdf_render", { seconds: 6, output: "pdf_render/one_page    time:   [41.2 ms 41.9 ms 42.7 ms]\npdf_render/ten_pages   time:   [388 ms 395 ms 402 ms]\npdf_render/fonts_load  time:   [33.8 ms 34.1 ms 34.5 ms]" })),
    at(1.5, think("Font loading is 80% of a one-page render. Caching the parsed fonts in a OnceLock should remove most of it.")),
    at(1.5, edit("src/pdf/fonts.rs", `@@ -8,7 +8,10 @@ use printpdf::IndirectFontRef;
-pub fn load_fonts(doc: &PdfDocumentReference) -> Fonts {
-    Fonts::parse(include_bytes!("../../assets/Inter.ttf"))
+static PARSED: std::sync::OnceLock<ParsedFonts> = std::sync::OnceLock::new();
+
+pub fn load_fonts(doc: &PdfDocumentReference) -> Fonts {
+    let parsed = PARSED.get_or_init(|| ParsedFonts::parse(include_bytes!("../../assets/Inter.ttf")));
+    parsed.attach(doc)
 }
`)),
    at(1.5, tool("bash", "cargo bench --bench pdf_render", { seconds: 6, output: "pdf_render/one_page    time:   [7.9 ms 8.1 ms 8.4 ms]\n                       change: [-81.0% -80.6% -80.1%] (p = 0.00 < 0.05)\n                       Performance has improved." })),
  ],
});

/** A reply to a message that matches no other scenario. */
export const followUp = scenario({
  id: "follow-up",
  folder: "storefront",
  harness: "claude",
  title: "Follow-up",
  timeline: [
    at(0.5, think("Reading the message and the current state of the folder.")),
    at(0.8, tool("bash", "git status --short", { seconds: 0.5, output: " M src/checkout/submit.ts\n M tests/checkout.test.ts" })),
    at(0.8, say("Looking at the related code before I answer.")),
    at(1.0, tool("bash", "pnpm test", { seconds: 2.5, output: " Test Files  9 passed (9)\n      Tests  187 passed (187)" })),
    final("Done. This is a demo agent, so it replays a short prerecorded answer: in a real run, the agent would work on your message in this folder and report what it changed and checked."),
  ],
});

/** The assistant's reply to a question other than "what needs me today". */
export const askOther = scenario({
  id: "ask-other",
  folder: "storefront",
  harness: "claude",
  title: "Ask Launchpad",
  timeline: [
    at(0.4, think("Reading the workspace state to answer.")),
    at(0.8, tool("bash", "launchpad sessions list --workspace Acme", { seconds: 0.8, output: "[…]" })),
    final("In the demo, Ask Launchpad answers with prerecorded replies. Try **What needs me today?** to see it propose a launch you can start with one click."),
  ],
});
