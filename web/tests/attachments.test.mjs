import assert from "node:assert/strict";
import { test } from "node:test";
import { attachmentSrc, pastedImages } from "../src/lib/attachments.ts";

function paste({ files = [], items = [], types = [], text = "" } = {}) {
  return {
    clipboardData: { files, items, types, getData: (type) => type === "text/plain" ? text : "" },
    prevented: false,
    preventDefault() { this.prevented = true; },
  };
}

test("DOM image files and items attach once", () => {
  const image = new File(["image bytes"], "shot.png", { type: "image/png" });
  const item = { kind: "file", type: "image/png", getAsFile: () => image };
  for (const data of [{ files: [image], items: [item] }, { items: [item] }]) {
    const event = paste(data);
    assert.deepEqual(pastedImages(event), [image]);
    assert.equal(event.prevented, true);
  }
});

test("text and empty clipboards keep the native paste", () => {
  for (const data of [{}, { text: "clipboard text" }, ...["text/plain", "text/html", "text/uri-list"].map((type) => ({ types: [type] }))]) {
    const event = paste(data);
    assert.equal(pastedImages(event), null);
    assert.equal(event.prevented, false);
  }
});

test("image sources: upload references become daemon URLs, data URLs stay", () => {
  globalThis.window ??= {};
  const hash = "a".repeat(64);
  assert.equal(attachmentSrc(`upload:image/png;${hash}`), `/api/uploads/${hash}?type=image%2Fpng`);
  assert.equal(attachmentSrc("data:image/png;base64,AAAA"), "data:image/png;base64,AAAA");
  assert.equal(attachmentSrc("/api/x.png"), "/api/x.png");
  assert.equal(attachmentSrc("upload:image/png;bad"), "upload:image/png;bad");
});
