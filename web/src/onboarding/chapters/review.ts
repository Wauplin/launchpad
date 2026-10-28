import type { Chapter } from "../tour/types";
import { meta } from "./meta";

export const review: Chapter = {
  ...meta("review"),
  learned: [],
  steps: [{ id: "review.todo", title: "Coming soon", body: "This chapter is being written." }],
};
