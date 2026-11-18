// Chapter: folios, the reports agents write as Markdown and Launchpad renders: the Folios page, the reader with its
// typed blocks, your marks and comments, Send feedback, and a session's Folio tab.

import type { Chapter } from "../tour/types";
import { meta } from "./meta";

export const folios: Chapter = {
  ...meta("folios"),
  learned: ["Agents present reports, plans and audits as folios: Markdown with typed blocks that Launchpad renders and versions", "You mark items Do, Later, No or ?, pick options and comment right on the page; marks are saved on Home at once", "Send feedback sends them in one short message to the folio's bot thread or the agent that wrote it", "A session that wrote a folio has a Folio tab"],
  steps: [
    {
      id: "folios.page", route: "/folios", anchor: "folios.page",
      title: "Folios",
      body: "A folio is a report, plan, audit or sketchbook that an agent writes as Markdown with typed blocks (roadmaps, findings, decisions…). Launchpad renders it with one design, keeps every rev and archives it after 30 quiet days. Each row shows its kind, who wrote it and what waits for you.",
    },
    {
      id: "folios.reader", route: "/folios/roadmap-winter", anchor: "folio.reader",
      title: "The reader",
      body: "The contents on the left, the page in the middle. Typed blocks take your marks: Do, Later, No or ? on each roadmap item, a pick on each decision, ticks on checklists. Every mark is saved on Home at once, so it follows you to the phone.",
    },
    {
      id: "folios.comments", route: "/folios/roadmap-winter?panel=comments", anchor: "folio.comments",
      title: "Comments",
      body: "Select text, or use Comment on a section or an item: the comment keeps the quote, so it survives edits. Agents reply under it and resolve it with the rev that addressed it.",
    },
    {
      id: "folios.send", route: "/folios/roadmap-winter", anchor: "folio.send",
      title: "Send feedback",
      body: "Nothing is sent until you click Send feedback. It goes to the folio's bot thread, else the session that wrote it, or a new thread of a bot with the folio's agents as context. The agent reads your marks with launchpad folio feedback in a few lines.",
    },
    {
      id: "folios.session", route: "/agents?session=demo-s-docs&view=folio", anchor: "session.folio",
      title: "The Folio tab",
      body: "A session that wrote a folio shows it in its Folio tab, with the same marks and comments. ⤢ opens it full screen.",
    },
  ],
};
