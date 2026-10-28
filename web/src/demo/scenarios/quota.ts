// Chapter 9: quota pause and continue. The run stops at Quota paused when included quota runs out and continues by
// itself once the demo clock passes the reset (`clock.fastForward`), like the real quota protection.

import { at, edit, final, quotaPause, scenario, think, tool } from "../player";

/** Minutes until the Claude 5-hour window resets in the demo (shared with the status quota readings). */
export const QUOTA_RESET_MINUTES = 47;

const modDiff = `@@ -1,4 +1,8 @@
-pub mod service;
+pub mod service;
+pub mod totals;
+pub mod numbering;
+pub mod pdf;
+pub use service::InvoiceService;
`;

const totalsDiff = `@@ -0,0 +1,18 @@
+use crate::money::Cents;
+use super::LineItem;
+
+/// Net, tax and gross totals of an invoice, rounded per line (see docs/tax-rounding.md).
+pub fn totals(lines: &[LineItem]) -> (Cents, Cents, Cents) {
+    let net: Cents = lines.iter().map(|l| l.net()).sum();
+    let tax: Cents = lines.iter().map(|l| l.tax_rounded()).sum();
+    (net, tax, net + tax)
+}
`;

const numberingDiff = `@@ -0,0 +1,12 @@
+/// Invoice numbers: \`INV-<year>-<sequence>\`, the sequence restarting every year.
+pub fn next_number(year: i32, last: Option<&str>) -> String {
+    let seq = last
+        .and_then(|n| n.strip_prefix(&format!("INV-{year}-")))
+        .and_then(|s| s.parse::<u32>().ok())
+        .unwrap_or(0);
+    format!("INV-{year}-{:05}", seq + 1)
+}
`;

export default scenario({
  id: "quota",
  folder: "billing-api",
  harness: "claude",
  title: "Split the invoice service into modules",
  timeline: [
    at(0.4, think("src/invoices/service.rs is 1,400 lines. I will move totals, numbering and PDF rendering into their own modules without changing behavior.")),
    at(0.8, tool("bash", "wc -l src/invoices/*.rs", { seconds: 0.5, output: "  1412 src/invoices/service.rs\n    38 src/invoices/mod.rs\n  1450 total" })),
    at(0.8, edit("src/invoices/mod.rs", modDiff)),
    at(0.8, edit("src/invoices/totals.rs", totalsDiff, "add")),
    quotaPause(QUOTA_RESET_MINUTES),
    at(0.6, think("Quota is available again. Continuing with the numbering module.")),
    at(0.8, edit("src/invoices/numbering.rs", numberingDiff, "add")),
    at(0.8, tool("bash", "cargo test invoices", { seconds: 2.4, output: "running 58 tests\n…\ntest result: ok. 58 passed; 0 failed; 0 ignored; finished in 1.92s" })),
    final(`**The invoice service is split into modules; behavior is unchanged.**

- \`src/invoices/totals.rs\` and \`src/invoices/numbering.rs\` hold the code moved out of \`service.rs\`.
- \`cargo test invoices\`: **58 passed**.

The run paused once when the included 5-hour quota ran out and continued by itself after the reset; no paid usage was used.

### Remaining work
- PDF rendering is still in \`service.rs\`; it can move to \`pdf.rs\` in a follow-up.`),
  ],
});
