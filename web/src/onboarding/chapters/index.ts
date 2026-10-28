// Ordered chapter registry (like the harness registrations). Essentials chapters form the short track; the full
// track plays every chapter in this order. Shown times are owner decisions (about 3 and 10 minutes) and do not
// change with the content.

import type { Chapter, Track } from "../tour/types";
import { layout } from "./layout";
import { launch } from "./launch";
import { answering } from "./answering";
import { review } from "./review";
import { github } from "./github";
import { ask } from "./ask";
import { terminals } from "./terminals";
import { folders } from "./folders";
import { automation } from "./automation";
import { settings } from "./settings";

export const CHAPTERS: Chapter[] = [layout, launch, answering, review, github, ask, terminals, folders, automation, settings];

export const chapter = (id: string) => CHAPTERS.find((c) => c.id === id);
export const trackChapters = (track: Track) => CHAPTERS.filter((c) => track === "full" || c.essentials);
/** The time shown to users for a track. */
export const trackTime = (track: Track) => (track === "essentials" ? "about 3 min" : "about 10 min");
