// Icon sprite, copied from docs/design/sketches.html. Render <IconSprite /> once, then <Icon name="…" />.

export type IconName =
  | "issue"
  | "issue-done"
  | "pr"
  | "pr-draft"
  | "pr-merged"
  | "pr-local"
  | "inbox"
  | "agents"
  | "folder"
  | "sliders"
  | "check"
  | "x"
  | "bell-off"
  | "mail"
  | "play"
  | "refresh"
  | "ext"
  | "copy"
  | "plus"
  | "branch"
  | "term"
  | "stop"
  | "chev"
  | "search"
  | "file"
  | "comment"
  | "commit"
  | "eye"
  | "grip"
  | "pencil"
  | "trash"
  | "up"
  | "send"
  | "alert"
  | "xcircle"
  | "okcircle"
  | "window"
  | "archive"
  | "clock"
  | "clip";

export function IconSprite() {
  return (
    <svg width="0" height="0" style={{ position: "absolute" }} aria-hidden="true">
      <defs>
    <symbol id="i-clip" viewBox="0 0 16 16"><path d="M13 7.5 8 12.5a3.2 3.2 0 0 1-4.5-4.5l5.3-5.3a2.1 2.1 0 0 1 3 3L6.6 10.9a1 1 0 0 1-1.5-1.5L10 4.5" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round"/></symbol>
    <symbol id="i-clock" viewBox="0 0 16 16"><circle cx="8" cy="8" r="6" fill="none" stroke="currentColor" strokeWidth="1.5"/><path d="M8 4v4l3 2" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round"/></symbol>
    <symbol id="i-archive" viewBox="0 0 16 16"><path d="M3 6v7h10V6M2 3h12v3H2zM6 9h4" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinejoin="round"/></symbol>
    <symbol id="i-issue" viewBox="0 0 16 16"><circle cx="8" cy="8" r="6.25" fill="none" stroke="currentColor" strokeWidth="1.5"/><circle cx="8" cy="8" r="1.6" fill="currentColor"/></symbol>
    <symbol id="i-issue-done" viewBox="0 0 16 16"><circle cx="8" cy="8" r="6.25" fill="none" stroke="currentColor" strokeWidth="1.5"/><path d="M5.4 8.2l1.8 1.8 3.4-3.6" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round"/></symbol>
    <symbol id="i-pr" viewBox="0 0 16 16"><g fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round"><circle cx="4" cy="3.5" r="1.75"/><circle cx="4" cy="12.5" r="1.75"/><circle cx="12" cy="12.5" r="1.75"/><path d="M4 5.25v5.5"/><path d="M12 10.75V6.5a2 2 0 0 0-2-2H7.5"/><path d="M9 3 7.5 4.5 9 6"/></g></symbol>
    <symbol id="i-pr-draft" viewBox="0 0 16 16"><g fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round"><circle cx="4" cy="3.5" r="1.75"/><circle cx="4" cy="12.5" r="1.75"/><circle cx="12" cy="12.5" r="1.75"/><path d="M4 5.25v5.5"/></g><circle cx="12" cy="8" r="1" fill="currentColor"/><circle cx="12" cy="4.25" r="1" fill="currentColor"/></symbol>
    <symbol id="i-pr-merged" viewBox="0 0 16 16"><g fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round"><circle cx="4" cy="3.5" r="1.75"/><circle cx="4" cy="12.5" r="1.75"/><circle cx="12" cy="8.5" r="1.75"/><path d="M4 5.25v5.5"/><path d="M4 5.25c0 2.5 2.2 3.25 6.25 3.25"/></g></symbol>
    <symbol id="i-pr-local" viewBox="0 0 16 16"><g fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round"><circle cx="4" cy="3.5" r="1.75"/><circle cx="4" cy="12.5" r="1.75"/><circle cx="12" cy="12.5" r="1.75"/><path d="M4 5.6v4.8" strokeDasharray="1.6 1.6"/><path d="M12 10.75V6.5a2 2 0 0 0-2-2H7.5" strokeDasharray="1.6 1.6"/></g></symbol>
    <symbol id="i-inbox" viewBox="0 0 16 16"><g fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinejoin="round"><path d="M2 9.5 3.6 3.6a1 1 0 0 1 1-.85h6.8a1 1 0 0 1 1 .85L14 9.5v3a1 1 0 0 1-1 1H3a1 1 0 0 1-1-1z"/><path d="M2 9.5h3.5l1 1.5h3l1-1.5H14"/></g></symbol>
    <symbol id="i-agents" viewBox="0 0 16 16"><g fill="none" stroke="currentColor" strokeWidth="1.5"><rect x="2" y="2" width="5" height="5" rx="1.2"/><rect x="9" y="2" width="5" height="5" rx="1.2"/><rect x="2" y="9" width="5" height="5" rx="1.2"/><rect x="9" y="9" width="5" height="5" rx="1.2"/></g></symbol>
    <symbol id="i-folder" viewBox="0 0 16 16"><path d="M2 4.5a1 1 0 0 1 1-1h3.2l1.5 1.5H13a1 1 0 0 1 1 1v6a1 1 0 0 1-1 1H3a1 1 0 0 1-1-1z" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinejoin="round"/></symbol>
    <symbol id="i-sliders" viewBox="0 0 16 16"><g fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round"><path d="M2.5 4h6.5M12 4h1.5M2.5 8h1.5M7 8h6.5M2.5 12h7.5M13 12h.5"/><circle cx="10.5" cy="4" r="1.5"/><circle cx="5.5" cy="8" r="1.5"/><circle cx="11.5" cy="12" r="1.5"/></g></symbol>
    <symbol id="i-check" viewBox="0 0 16 16"><path d="M3.5 8.5 6.5 11.5 12.5 5" fill="none" stroke="currentColor" strokeWidth="1.7" strokeLinecap="round" strokeLinejoin="round"/></symbol>
    <symbol id="i-x" viewBox="0 0 16 16"><path d="M4.5 4.5l7 7M11.5 4.5l-7 7" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round"/></symbol>
    <symbol id="i-bell-off" viewBox="0 0 16 16"><g fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round"><path d="M4.5 10.5V7a3.5 3.5 0 0 1 6.2-2.2"/><path d="M11.5 7.2v3.3l1 1.5h-9"/><path d="M6.5 13.5a1.5 1.5 0 0 0 3 0"/><path d="M2.5 2.5l11 11"/></g></symbol>
    <symbol id="i-mail" viewBox="0 0 16 16"><g fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinejoin="round"><rect x="2" y="3.5" width="12" height="9" rx="1.5"/><path d="M2.5 4.5 8 8.5l5.5-4"/></g></symbol>
    <symbol id="i-play" viewBox="0 0 16 16"><path d="M5 3.2v9.6a.6.6 0 0 0 .9.5l7.4-4.8a.6.6 0 0 0 0-1L5.9 2.7a.6.6 0 0 0-.9.5z" fill="currentColor"/></symbol>
    <symbol id="i-refresh" viewBox="0 0 16 16"><g fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round"><path d="M13 8a5 5 0 1 1-1.5-3.55"/><path d="M13 2.5v3h-3"/></g></symbol>
    <symbol id="i-ext" viewBox="0 0 16 16"><g fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round"><path d="M9 2.5h4.5V7"/><path d="M13.5 2.5 7.5 8.5"/><path d="M12 9.5v3a1 1 0 0 1-1 1H3.5a1 1 0 0 1-1-1V5a1 1 0 0 1 1-1h3"/></g></symbol>
    <symbol id="i-copy" viewBox="0 0 16 16"><g fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinejoin="round"><rect x="5.5" y="5.5" width="8" height="8" rx="1.5"/><path d="M10.5 5.5v-2a1 1 0 0 0-1-1h-6a1 1 0 0 0-1 1v6a1 1 0 0 0 1 1h2"/></g></symbol>
    <symbol id="i-plus" viewBox="0 0 16 16"><path d="M8 3.5v9M3.5 8h9" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round"/></symbol>
    <symbol id="i-branch" viewBox="0 0 16 16"><g fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round"><circle cx="4.5" cy="3.5" r="1.5"/><circle cx="4.5" cy="12.5" r="1.5"/><circle cx="11.5" cy="5" r="1.5"/><path d="M4.5 5v6"/><path d="M11.5 6.5c0 3-7 2-7 4.5"/></g></symbol>
    <symbol id="i-term" viewBox="0 0 16 16"><g fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round"><rect x="1.75" y="2.75" width="12.5" height="10.5" rx="1.5"/><path d="M4.5 6.5 6.5 8.5 4.5 10.5"/><path d="M8 10.5h3.5"/></g></symbol>
    <symbol id="i-stop" viewBox="0 0 16 16"><rect x="4" y="4" width="8" height="8" rx="1.5" fill="currentColor"/></symbol>
    <symbol id="i-chev" viewBox="0 0 16 16"><path d="M4.5 6.5 8 10l3.5-3.5" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round"/></symbol>
    <symbol id="i-search" viewBox="0 0 16 16"><g fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round"><circle cx="7" cy="7" r="4.5"/><path d="M10.5 10.5 13.5 13.5"/></g></symbol>
    <symbol id="i-file" viewBox="0 0 16 16"><g fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinejoin="round"><path d="M4 1.75h5l3 3v9.5H4z"/><path d="M9 1.75v3h3"/></g></symbol>
    <symbol id="i-comment" viewBox="0 0 16 16"><path d="M2.5 3.5a1 1 0 0 1 1-1h9a1 1 0 0 1 1 1v7a1 1 0 0 1-1 1H7.2L4.5 13.6v-2.1h-1a1 1 0 0 1-1-1z" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinejoin="round"/></symbol>
    <symbol id="i-commit" viewBox="0 0 16 16"><g fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round"><circle cx="8" cy="8" r="2.5"/><path d="M1.5 8h4M10.5 8h4"/></g></symbol>
    <symbol id="i-eye" viewBox="0 0 16 16"><g fill="none" stroke="currentColor" strokeWidth="1.5"><path d="M1.5 8s2.4-4.5 6.5-4.5S14.5 8 14.5 8 12.1 12.5 8 12.5 1.5 8 1.5 8z"/><circle cx="8" cy="8" r="2"/></g></symbol>
    <symbol id="i-grip" viewBox="0 0 16 16"><g fill="currentColor"><circle cx="6" cy="4" r="1.1"/><circle cx="10" cy="4" r="1.1"/><circle cx="6" cy="8" r="1.1"/><circle cx="10" cy="8" r="1.1"/><circle cx="6" cy="12" r="1.1"/><circle cx="10" cy="12" r="1.1"/></g></symbol>
    <symbol id="i-pencil" viewBox="0 0 16 16"><path d="M10.5 2.75l2.75 2.75-7.5 7.5H3v-2.75z" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinejoin="round"/></symbol>
    <symbol id="i-trash" viewBox="0 0 16 16"><g fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round"><path d="M2.75 4.5h10.5"/><path d="M6.25 4.5V3h3.5v1.5"/><path d="M4.25 4.5l.6 9h6.3l.6-9"/></g></symbol>
    <symbol id="i-up" viewBox="0 0 16 16"><path d="M8 13V3.5M4 7.5l4-4 4 4" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round"/></symbol>
    <symbol id="i-send" viewBox="0 0 16 16"><path d="M2.5 8 13.5 2.5 10.5 13.5 8 9z" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinejoin="round"/></symbol>
    <symbol id="i-alert" viewBox="0 0 16 16"><g fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round"><path d="M8 2 14.5 13.5h-13z"/><path d="M8 6.5v3"/></g><circle cx="8" cy="11.6" r=".9" fill="currentColor"/></symbol>
    <symbol id="i-xcircle" viewBox="0 0 16 16"><circle cx="8" cy="8" r="6.25" fill="currentColor"/><path d="M5.8 5.8l4.4 4.4M10.2 5.8l-4.4 4.4" stroke="#fff" strokeWidth="1.5" strokeLinecap="round"/></symbol>
    <symbol id="i-okcircle" viewBox="0 0 16 16"><circle cx="8" cy="8" r="6.25" fill="currentColor"/><path d="M5.3 8.2l1.8 1.8 3.6-3.7" fill="none" stroke="#fff" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round"/></symbol>
    <symbol id="i-window" viewBox="0 0 16 16"><g fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinejoin="round"><rect x="1.75" y="2.75" width="12.5" height="10.5" rx="1.5"/><path d="M1.75 5.75h12.5"/></g></symbol>
      </defs>
    </svg>
  );
}

export function Icon({ name, size = 16, className = "" }: { name: IconName; size?: number; className?: string }) {
  return (
    <svg width={size} height={size} className={`inline-block flex-none ${className}`} aria-hidden="true">
      <use href={`#i-${name}`} />
    </svg>
  );
}
