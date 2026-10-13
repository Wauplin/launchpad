// Desktop entry: the shell with Agents and Inbox in the same chunk, so the first page needs no extra request.
import type { Entry } from "./router";
import { Shell } from "./Shell";
import { AgentsView } from "@/features/agents/AgentsView";
import { InboxView } from "@/features/inbox/InboxView";

export const entry: Entry = { Root: Shell, AgentsView, InboxView };
