import type { Chapter } from "../tour/types";
import { meta } from "./meta";

export const launch: Chapter = {
  ...meta("launch"),
  learned: [],
  steps: [{ id: "launch.todo", title: "Coming soon", body: "This chapter is being written." }],
};
