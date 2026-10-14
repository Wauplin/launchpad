// Phone key bar state, shared with the emulators without loading xterm.

/** Emulators by terminal id (the phone key bar types into the shown one). `input` is xterm's `Terminal.input`. */
export const terminalInputs = new Map<string, { input: (data: string, wasUserInput?: boolean) => void; focus: () => void; paste: (text: string) => void; getSelection: () => string; selectAll: () => void; arrow: (direction: "A" | "B" | "C" | "D") => void }>();
/** Sticky Ctrl, applied to the next letter typed in any terminal. */
export const keyBar = { ctrl: false, changed: () => {} };
