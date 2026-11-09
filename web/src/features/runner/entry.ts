// A runner's own window: one status page, no shell, sidebar or other pages (see main.tsx).
import type { Entry } from "@/app/router";
import { RunnerApp } from "./RunnerPage";

const Nothing = () => null;

export const entry: Entry = { Root: RunnerApp, TodayView: Nothing, AgentsView: Nothing };
