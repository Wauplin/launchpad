// Chapter 3: agent groups, questions, approval cards, replies and the state-dependent primary button.

import type { Chapter } from "../tour/types";
import { world } from "@/demo/world";
import { useVoice } from "@/lib/voice/dictation";
import { anchor, clickAnchor, waitFor } from "./helpers";
import { meta } from "./meta";

const ID = "demo-s-invoices";
const answers = () => (world.events[ID] ?? []).filter((e) => e.body.kind === "ask_answered").length;
let start = 0;
const firstButton = async (name: string) => { (await waitFor(() => anchor(name)?.querySelector<HTMLButtonElement>("button:not(:disabled)")))?.click(); };

export const answering: Chapter = {
  ...meta("answering"),
  learned: ["Agents groups your runs: Running, Quota paused, Done and more; questions and unseen results also wait in Today", "Agents ask questions; you answer with a click", "In Ask for approval mode, commands wait for your decision", "One primary button follows the state: Stop, Answer, Archive or Resume", "Dictate in any message box with the mic: words land at the caret, and nothing is sent until you send it"],
  setup: () => { start = answers(); },
  steps: [
    {
      id: "answering.groups", route: "/agents", anchor: "agents.list",
      title: "Agents, grouped by state",
      body: "Agents shows what runs and what ran. Running: working now. Quota paused: waiting for subscription quota. Done: waiting for your next message. Runs you have not seen are bold; they also wait in Today until you open them.",
    },
    {
      id: "answering.topics", route: `/agents?session=${ID}`, anchor: "session.topics",
      title: "Keep related sessions together",
      body: "Add a topic to collect related work across folders. Type to find or create one. A topic name opens its history; × removes this session, with Undo. Topics stay with archived sessions too.",
    },
    {
      id: "answering.question", route: `/agents?session=${ID}`, anchor: "session.question",
      title: "An agent asks you",
      body: "Agents can stop and ask. The question shows here and on its Today card, where one click answers it. You can also type another answer in the message box.",
      tryIt: { text: "pick an answer.", done: () => answers() > start, doIt: () => firstButton("session.question") },
    },
    {
      id: "answering.approval", anchor: "session.approval",
      title: "Approval cards",
      body: "This agent runs in Ask for approval mode: commands wait for your decision. Only these buttons approve; a typed message never grants a permission.",
      tryIt: { text: "approve cargo test.", done: () => answers() > start + 1, doIt: () => firstButton("session.approval") },
    },
    {
      id: "answering.reply", anchor: "session.reply",
      title: "Reply",
      body: "Send the next message here once the agent is Done. @, / and # work as in the composer; the + icon attaches folders, GitHub links, traces or presets (your usual instructions). Enter sends; Shift+Enter adds a line.",
      enter: () => clickAnchor("session.views", "trace"),
    },
    {
      id: "answering.voice", anchor: "session.mic",
      title: "Dictate",
      body: "Tap the mic and speak: your words show live, then land at the caret. Tap again, or stay quiet, to stop; Stop & send sends at once. Ctrl+Space works in every message box. In the demo, a scripted voice speaks.",
      tryIt: { text: "click the mic.", done: () => useVoice.getState().phase === "listening", doIt: () => clickAnchor("session.mic") },
    },
    {
      id: "answering.primary", anchor: "session.primary",
      title: "One primary button",
      body: "It follows the state: Stop while running, Answer for a question, Merge when the branch can go into its base safely, else Review changes or Archive when Done, Resume after a stop. The ⋯ of each row in the list offers Stop, Archive, Mark unread and Pin.",
    },
  ],
};
