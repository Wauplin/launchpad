import type { Terminal } from "@xterm/xterm";

/** Keep resets in xterm's asynchronous parser queue, after old output and before the snapshot. */
export function terminalOutput(terminal: Pick<Terminal, "write" | "resize">, acknowledge: (epoch: number, bytes: number) => void, ready: () => void) {
  let epoch = 0;
  let replaying = true;
  return {
    get replaying() { return replaying; },
    reset(next: number, rows: number, cols: number) {
      epoch = next;
      replaying = true;
      // CAN cancels an unfinished escape sequence; RIS resets parser, modes and both screens.
      // Use bytes so an incomplete UTF-8 character is also flushed before RIS clears the screen.
      terminal.write(Uint8Array.of(0x18, 0x1b, 0x63), () => terminal.resize(cols, rows));
    },
    /** Continues the previous stream after a reconnect: no reset, the missing bytes follow at the PTY size. */
    resume(next: number, rows: number, cols: number) {
      epoch = next;
      replaying = true;
      terminal.write("", () => terminal.resize(cols, rows));
    },
    data(data: Uint8Array) {
      const generation = epoch;
      terminal.write(data, () => acknowledge(generation, data.byteLength));
    },
    ready() {
      const generation = epoch;
      terminal.write("", () => {
        if (generation !== epoch) return;
        replaying = false;
        ready();
      });
    },
  };
}
