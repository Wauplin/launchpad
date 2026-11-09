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
