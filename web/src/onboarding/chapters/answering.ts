// Chapter 3: agent groups, questions, approval cards, replies and the state-dependent primary button.

import type { Chapter } from "../tour/types";
import { world } from "@/demo/world";
import { anchor, clickAnchor, waitFor } from "./helpers";
import { meta } from "./meta";

const ID = "demo-s-invoices";
const answers = () => (world.events[ID] ?? []).filter((e) => e.body.kind === "ask_answered").length;
let start = 0;
const firstButton = async (name: string) => { (await waitFor(() => anchor(name)?.querySelector<HTMLButtonElement>("button:not(:disabled)")))?.click(); };

export const answering: Chapter = {
  ...meta("answering"),
  learned: ["Needs you, Running, Quota paused and Done group your agents", "Agents ask questions; you answer with a click", "In Ask for approval mode, commands wait for your decision", "One primary button follows the state: Stop, Answer, Archive or Resume"],
  setup: () => { start = answers(); },
  steps: [
    {
      id: "answering.groups", route: "/agents", anchor: "agents.list",
      title: "Agents, grouped by what they need",
      body: "Needs you: questions and finished runs you have not seen. Running: working now. Quota paused: waiting for subscription quota. Done: seen and waiting for your next message.",
    },
    {
      id: "answering.question", route: `/agents?session=${ID}`, anchor: "session.question",
      title: "An agent asks you",
      body: "Agents can stop and ask. The question shows here and the agent is in Needs you until you answer. You can also type another answer in the message box.",
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
      body: "When the agent is Done, send the next message here. Add context attaches read-only folders, GitHub links or another agent’s trace. Presets add your usual instructions with one click. Enter sends; Shift+Enter adds a line.",
      enter: () => clickAnchor("session.views", "trace"),
    },
    {
      id: "answering.primary", anchor: "session.primary",
      title: "One primary button",
      body: "It follows the state: Stop while running, Answer for a question, Review changes or Archive when Done, Resume after a stop. Rows in the list also offer Stop, Archive and Mark unread.",
    },
  ],
};
