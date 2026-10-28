// Every prerecorded scenario, and how a launch prompt or a message picks one.

import { registerScenarios } from "../player";
import { askOther, darkMode, followUp, pdfSpeed } from "./ambient";
import askToday from "./askToday";
import checkout from "./checkout";
import lineNotes from "./lineNotes";
import localPr from "./localPr";
import quota from "./quota";
import rateLimit from "./rateLimit";

registerScenarios([checkout, rateLimit, lineNotes, localPr, askToday, quota, darkMode, pdfSpeed, followUp, askOther]);

const RULES: [RegExp, string][] = [
  [/review notes|line notes|address .*notes/i, "line-notes"],
  [/rate.?limit|invoices?|throttl|429/i, "rate-limit"],
  [/#\s?42|issue|local pr|pull request|webhook|docs?\b|document/i, "local-pr"],
  [/quota|split|refactor|modules?/i, "quota"],
  [/checkout|flaky|test/i, "checkout"],
];

/** The closest scenario for a launch prompt (keyword match, default `checkout`). */
export function pickScenario(prompt: string): string {
  return RULES.find(([re]) => re.test(prompt))?.[1] ?? "checkout";
}

/** The scenario for a message sent to an existing session. */
export function pickReply(text: string, assistant: boolean): string {
  if (assistant) return /needs? me|today|what should|todo|to do|attention/i.test(text) ? "ask-today" : "ask-other";
  return /review notes|line notes/i.test(text) ? "line-notes" : "follow-up";
}
