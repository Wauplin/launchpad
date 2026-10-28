import type { Chapter } from "../tour/types";
import { meta } from "./meta";

export const settings: Chapter = {
  ...meta("settings"),
  learned: [],
  steps: [{ id: "settings.todo", title: "Coming soon", body: "This chapter is being written." }],
};
