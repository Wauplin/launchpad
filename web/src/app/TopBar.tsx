import { useRouterState } from "@tanstack/react-router";
import { Icon } from "@/components/Icon";

const TITLES: [RegExp, string][] = [
  [/^\/$/, "Inbox"],
  [/^\/agents/, "Agents"],
  [/^\/projects/, "Projects"],
  [/^\/settings/, "Settings"],
];

export function TopBar() {
  const path = useRouterState({ select: (s) => s.location.pathname });
  const title = TITLES.find(([re]) => re.test(path))?.[1] ?? "";
  return (
    <header className="flex h-[46px] flex-none items-center gap-3.5 border-b border-line bg-chrome pr-2.5 pl-3.5">
      <div className="flex items-center gap-2 font-bold tracking-[0.01em]">
        <span className="grid size-[22px] place-items-center rounded-md bg-teal">
          <svg viewBox="0 0 12 12" width="12" height="12">
            <path d="M6 1.5 10.5 10H1.5z" fill="#fff" />
          </svg>
        </span>
        Launchpad
      </div>
      <div className="flex items-center gap-1.5 text-ink-2">
        <b className="font-semibold text-ink">{title}</b>
      </div>
      <button className="btn btn-primary ml-auto" title="Launch an agent">
        <Icon name="play" size={14} />
        Launch
      </button>
    </header>
  );
}
