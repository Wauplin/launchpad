// Desktop entry: the shell with Today and Agents in the same chunk, so the first page needs no extra request.
import type { Entry } from "./router";
import { Shell } from "./Shell";
import { AgentsView } from "@/features/agents/AgentsView";
import { TodayView } from "@/features/today/TodayView";

export const entry: Entry = { Root: Shell, TodayView, AgentsView };
