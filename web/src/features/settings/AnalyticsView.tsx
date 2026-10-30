import { useState } from "react";
import { AgentActivity } from "./AgentActivity";
import { SettingsPage } from "./SettingsView";

const dayKey = (date: Date) => `${date.getFullYear()}-${date.getMonth()}-${date.getDate()}`;

export function AnalyticsView() {
  const [month, setMonth] = useState(() => new Date(new Date().getFullYear(), new Date().getMonth(), 1));
  const [selected, setSelected] = useState(() => dayKey(new Date()));
  const move = (delta: number) => {
    const date = new Date(month.getFullYear(), month.getMonth() + delta, 1);
    setMonth(date);
    setSelected(dayKey(date));
  };

  return <SettingsPage title="Usage" intro="Agent activity, per month.">
    <AgentActivity month={month} selected={selected} onSelect={setSelected} onMove={move} onToday={() => { const today = new Date(); setMonth(new Date(today.getFullYear(), today.getMonth(), 1)); setSelected(dayKey(today)); }} />
  </SettingsPage>;
}
