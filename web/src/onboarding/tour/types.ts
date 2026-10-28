// Tour data: a chapter is a list of steps, each one idea with at most 40 words. Steps are data so the anchor
// check (`pnpm test:tour`) can read them without rendering anything.

export type Track = "essentials" | "full";

export type Step = {
  /** `<chapter>.<name>`, unique. */
  id: string;
  /** Route to show before the step (opened with the router; search params allowed). */
  route?: string;
  /** `data-tour` attribute of the element to highlight; none centers the card. */
  anchor?: string;
  /** Narrows the anchor to the element whose `data-tour-id` matches (rows of virtualized lists). */
  anchorId?: string;
  title: string;
  body: string;
  /**
   * A "Try it" step: the instruction shown in purple. The step advances when `done` returns true (polled), or
   * when the user clicks inside the anchor if `done` is missing. "Do it for me" runs `doIt`.
   */
  tryIt?: { text: string; done?: () => boolean; doIt?: () => unknown | Promise<unknown> };
  /** The session whose prerecorded run the step watches: the card offers 1×, 2× and Skip to end while it plays. */
  run?: () => string | null;
  /** Runs when the step is shown (prepare the world, open a panel). */
  enter?: () => unknown | Promise<unknown>;
  /** Release where the step first appeared (What's new, later). */
  since?: string;
};

export type Chapter = {
  id: string;
  title: string;
  /** Shown time, as "m:ss". */
  time: string;
  essentials: boolean;
  /** What the recap card lists. */
  learned: string[];
  /** Builds the chapter's starting state after the world reset (start scenarios, select sessions). */
  setup?: () => unknown | Promise<unknown>;
  steps: Step[];
};
