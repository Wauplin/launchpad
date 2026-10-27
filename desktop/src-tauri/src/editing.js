// WebKitGTK has native editing history but lacks GTK key bindings for undo/redo.
// Keep this in the Linux shell: browsers and the other desktop engines bind these
// already. Use the editor's history (and input events), never replace field values.
window.addEventListener("keydown", (event) => {
  // Bubble after page handlers, so editors can keep their own shortcuts/history.
  if (event.defaultPrevented || event.isComposing || !event.ctrlKey || event.metaKey || event.altKey) return;
  const key = event.key.toLowerCase();
  const command = key === "z" ? (event.shiftKey ? "redo" : "undo") : key === "y" && !event.shiftKey ? "redo" : null;
  if (!command) return;

  const field = document.activeElement;
  // xterm uses a textarea for process input: Ctrl+Z must still reach the PTY.
  if (!field || field.closest(".xterm")) return;
  const textField = field instanceof HTMLTextAreaElement || (field instanceof HTMLInputElement && ["text", "search", "url", "tel", "email", "password"].includes(field.type));
  if (!textField || field.disabled || field.readOnly) return;

  event.preventDefault();
  document.execCommand(command);
});
