import assert from "node:assert/strict";
import { test } from "node:test";
import { pastedImages } from "../src/lib/attachments.ts";

function paste({ files = [], items = [], types = [], text = "" } = {}) {
  return {
    clipboardData: { files, items, types, getData: (type) => type === "text/plain" ? text : "" },
    prevented: false,
    preventDefault() { this.prevented = true; },
  };
}

test("DOM image files and items attach once without reading the native clipboard", () => {
  globalThis.window = { __launchpadReadClipboardImage: () => { throw Error("Unexpected native read"); } };
  const image = new File(["image bytes"], "shot.png", { type: "image/png" });
  const item = { kind: "file", type: "image/png", getAsFile: () => image };
  for (const data of [{ files: [image], items: [item] }, { items: [item] }]) {
    const event = paste(data);
    assert.deepEqual(pastedImages(event), [image]);
    assert.equal(event.prevented, true);
  }
});

test("empty WebKit screenshot events use the native PNG without suppressing native paste", async () => {
  globalThis.window = { __launchpadReadClipboardImage: async () => "data:image/png;base64,AQID" };
  for (const types of [[], ["Files"], ["image/png"]]) {
    const event = paste({ types });
    const files = await pastedImages(event);
    assert.equal(files.length, 1);
    assert.equal(files[0].type, "image/png");
    assert.deepEqual([...new Uint8Array(await files[0].arrayBuffer())], [1, 2, 3]);
    assert.equal(event.prevented, false);
  }
});

test("text remains native even when WebKit omits clipboard types", () => {
  globalThis.window = { __launchpadReadClipboardImage: () => { throw Error("Unexpected native read"); } };
  for (const data of [{ text: "clipboard text" }, ...["text/plain", "text/html", "text/uri-list"].map((type) => ({ types: [type] }))]) {
    const event = paste(data);
    assert.equal(pastedImages(event), null);
    assert.equal(event.prevented, false);
  }
});

test("empty clipboards, native errors and browsers without the bridge", async () => {
  globalThis.window = { __launchpadReadClipboardImage: async () => null };
  assert.deepEqual(await pastedImages(paste()), []);
  window.__launchpadReadClipboardImage = async () => { throw Error("Clipboard unavailable"); };
  await assert.rejects(pastedImages(paste()), /Clipboard unavailable/);
  globalThis.window = {};
  const event = paste();
  assert.equal(pastedImages(event), null);
  assert.equal(event.prevented, false);
});
