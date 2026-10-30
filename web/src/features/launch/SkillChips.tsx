import type { SkillRef } from "@/gen/SkillRef";
import { Icon } from "@/components/Icon";
export function SkillChips({ skills = [], disabled, onRemove }: { skills?: SkillRef[]; disabled?: boolean; onRemove: (path: string) => void }) {
  return skills.map((s) => <span key={s.path} title={s.path} className="inline-flex min-w-0 max-w-full items-center gap-1 rounded-md border border-line bg-chrome pl-2 text-xs text-ink-2"><Icon name="file" size={12} /><span className="truncate">/{s.name}</span><button className="iconbtn shrink-0" aria-label={`Remove skill ${s.name}`} disabled={disabled} onClick={() => onRemove(s.path)}><Icon name="x" size={12} /></button></span>);
}
