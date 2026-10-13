import assert from "node:assert/strict";
import { test } from "node:test";
import headless from "@xterm/headless";
import { terminalOutput } from "../src/features/terminals/output.ts";

const { Terminal } = headless;
const encoder = new TextEncoder();
const line = (terminal, row = 0) => terminal.buffer.active.getLine(row)?.translateToString(true);

async function replay(terminal, chunks, { rows = 6, cols = 30, epoch = 1 } = {}) {
  const acknowledgements = [];
  let ready;
  const done = new Promise((resolve) => { ready = resolve; });
  const output = terminalOutput(terminal, (epoch, bytes) => acknowledgements.push({ epoch, bytes }), ready);
  output.reset(epoch, rows, cols);
  for (const chunk of chunks) output.data(typeof chunk === "string" ? encoder.encode(chunk) : chunk);
  assert.equal(output.replaying, true);
  output.ready();
  await done;
  assert.equal(output.replaying, false);
  return acknowledgements;
}

test("carriage-return progress replaces the current line; erase removes old suffixes", async () => {
  const terminal = new Terminal({ allowProposedApi: true });
  try {
    await replay(terminal, ["Downloading 1000 bytes", "\r\x1b[2KDone\r\n", "next"]);
    assert.equal(line(terminal), "Done");
    assert.equal(line(terminal, 1), "next");
  } finally { terminal.dispose(); }
});

test("split UTF-8 and split escape sequences preserve bytes and colors", async () => {
  const terminal = new Terminal({ allowProposedApi: true });
  try {
    const data = encoder.encode("\x1b[31mHéllo 世界\x1b[0m");
    const acks = await replay(terminal, [...data].map((byte) => Uint8Array.of(byte)));
    assert.equal(line(terminal), "Héllo 世界");
    assert.equal(terminal.buffer.active.getLine(0).getCell(0).getFgColor(), 1);
    assert.equal(acks.reduce((total, ack) => total + ack.bytes, 0), data.byteLength);
  } finally { terminal.dispose(); }
});

test("alternate screen and cursor redraws restore the normal screen", async () => {
  const terminal = new Terminal({ allowProposedApi: true });
  try {
    await replay(terminal, ["shell", "\x1b[?1049h\x1b[Hworking", "\x1b[H\x1b[2Kdone", "\x1b[?1049l"]);
    assert.equal(terminal.buffer.active.type, "normal");
    assert.equal(line(terminal), "shell");
  } finally { terminal.dispose(); }
});

test("resync follows queued output, cancels a partial CSI and restores snapshot dimensions", async () => {
  const terminal = new Terminal({ allowProposedApi: true });
  try {
    terminal.write("old output\x1b[?1049h\x1b[");
    const acks = await replay(terminal, ["fresh"], { rows: 4, cols: 12, epoch: 2 });
    assert.equal(terminal.rows, 4);
    assert.equal(terminal.cols, 12);
    assert.equal(terminal.buffer.active.type, "normal");
    assert.equal(line(terminal), "fresh");
    assert.deepEqual(acks, [{ epoch: 2, bytes: 5 }]);
  } finally { terminal.dispose(); }
});

test("resync discards an unfinished UTF-8 character from the previous stream", async () => {
  const terminal = new Terminal({ allowProposedApi: true });
  try {
    terminal.write(Uint8Array.of(0xf0, 0x9f));
    await replay(terminal, ["fresh 世界"]);
    assert.equal(line(terminal), "fresh 世界");
  } finally { terminal.dispose(); }
});

test("rapid progress bursts end on the final frame without accumulating lines", async () => {
  const terminal = new Terminal({ allowProposedApi: true });
  try {
    await replay(terminal, Array.from({ length: 10000 }, (_, i) => `\r\x1b[2KProgress ${i}`));
    assert.equal(line(terminal), "Progress 9999");
    assert.equal(terminal.buffer.active.cursorY, 0);
  } finally { terminal.dispose(); }
});

test("resume continues the screen without a reset", async () => {
  const terminal = new Terminal({ allowProposedApi: true });
  const acks = [];
  try {
    await replay(terminal, ["\x1b[32mabc"]);
    let ready;
    const done = new Promise((resolve) => { ready = resolve; });
    const output = terminalOutput(terminal, (epoch, bytes) => acks.push({ epoch, bytes }), ready);
    output.resume(2, 6, 30);
    output.data(encoder.encode("def"));
    output.ready();
    await done;
    assert.equal(line(terminal), "abcdef");
    assert.equal(terminal.buffer.active.getLine(0).getCell(4).getFgColor(), 2);
    assert.deepEqual(acks, [{ epoch: 2, bytes: 3 }]);
  } finally { terminal.dispose(); }
});

test("ACK and ready callbacks keep their epoch across an overlapping resync", async () => {
  const terminal = new Terminal({ allowProposedApi: true });
  const acks = [];
  try {
    let ready;
    const done = new Promise((resolve) => { ready = resolve; });
    let readyCount = 0;
    const output = terminalOutput(terminal, (epoch, bytes) => acks.push({ epoch, bytes }), () => { readyCount++; ready(); });
    output.reset(1, 6, 30);
    output.data(encoder.encode("stale"));
    output.ready();
    output.reset(2, 6, 30);
    output.data(encoder.encode("current"));
    output.ready();
    await done;
    assert.deepEqual(acks, [{ epoch: 1, bytes: 5 }, { epoch: 2, bytes: 7 }]);
    assert.equal(line(terminal), "current");
    assert.equal(readyCount, 1);
  } finally { terminal.dispose(); }
});
