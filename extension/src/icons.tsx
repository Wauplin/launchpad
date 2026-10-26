// Line icons of the side panel (24 px grid, current color).

const paths = {
  send: <path d="M12 19V5M5 12l7-7 7 7" strokeWidth="2.4" />,
  stop: <rect x="7" y="7" width="10" height="10" rx="2" fill="currentColor" stroke="none" />,
  plus: <path d="M12 5v14M5 12h14" strokeWidth="2" />,
  agent: <><rect x="4" y="8" width="16" height="11" rx="3" /><path d="M12 8V5M9 13h.01M15 13h.01" /></>,
  eye: <><path d="M2 12s3.5-7 10-7 10 7 10 7-3.5 7-10 7S2 12 2 12z" /><circle cx="12" cy="12" r="3" /></>,
  hand: <path d="M8 13V5.5a1.5 1.5 0 013 0V12M11 11.5V4a1.5 1.5 0 013 0v7.5M14 11.5V6a1.5 1.5 0 013 0v8c0 4-2.5 7-6.5 7-3 0-4.5-1.5-6-4l-1.5-3a1.5 1.5 0 012.5-1.5L8 15" />,
  folder: <path d="M3 7a2 2 0 012-2h4l2 2h8a2 2 0 012 2v8a2 2 0 01-2 2H5a2 2 0 01-2-2z" />,
  chev: <path d="M7 10l5 5 5-5" strokeWidth="2" />,
  list: <path d="M8 6h13M8 12h13M8 18h13M3.5 6h.01M3.5 12h.01M3.5 18h.01" />,
  edit: <path d="M12 20h9M16.5 3.5a2.1 2.1 0 013 3L7 19l-4 1 1-4z" />,
  more: <><circle cx="5" cy="12" r="1.6" fill="currentColor" /><circle cx="12" cy="12" r="1.6" fill="currentColor" /><circle cx="19" cy="12" r="1.6" fill="currentColor" /></>,
  back: <path d="M15 6l-6 6 6 6" strokeWidth="2" />,
  ext: <path d="M14 4h6v6M20 4l-9 9M18 14v5a1 1 0 01-1 1H5a1 1 0 01-1-1V7a1 1 0 011-1h5" />,
  check: <path d="M5 12l5 5 9-10" strokeWidth="2.6" />,
  sum: <path d="M4 6h16M4 10h16M4 14h10M4 18h7" />,
  cmp: <><rect x="3" y="4" width="7" height="16" rx="1.5" /><rect x="14" y="4" width="7" height="16" rx="1.5" /></>,
  q: <><circle cx="12" cy="12" r="9" /><path d="M9.5 9.5a2.5 2.5 0 015 .5c0 2-2.5 2-2.5 4M12 17h.01" /></>,
  gear: <><circle cx="12" cy="12" r="3" /><path d="M12 2v3M12 19v3M4.9 4.9l2.1 2.1M17 17l2.1 2.1M2 12h3M19 12h3M4.9 19.1L7 17M17 7l2.1-2.1" /></>,
  warn: <><path d="M12 3l10 18H2z" /><path d="M12 10v5M12 18h.01" strokeWidth="2" /></>,
  lock: <><rect x="5" y="11" width="14" height="10" rx="2" /><path d="M8 11V8a4 4 0 018 0v3" /></>,
  close: <path d="M6 6l12 12M18 6L6 18" strokeWidth="2" />,
  archive: <><rect x="3" y="4" width="18" height="5" rx="1" /><path d="M5 9v10a1 1 0 001 1h12a1 1 0 001-1V9M10 13h4" /></>,
  image: <><rect x="3" y="4" width="18" height="16" rx="2" /><circle cx="9" cy="10" r="2" /><path d="M21 16l-5-5-9 9" /></>,
  quote: <path d="M7 7h4v4c0 3-1.5 5-4 6M14 7h4v4c0 3-1.5 5-4 6" />,
  globe: <><circle cx="12" cy="12" r="9" /><path d="M3 12h18M12 3c3 3.5 3 14.5 0 18M12 3c-3 3.5-3 14.5 0 18" /></>,
  plug: <path d="M9 7V3M15 7V3M6 7h12v4a6 6 0 01-12 0zM12 17v4" />,
};
export type IconName = keyof typeof paths;

export function Icon({ name, size = 18, className = "" }: { name: IconName; size?: number; className?: string }) {
  return <svg className={className} width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">{paths[name]}</svg>;
}

/** The site icon of a tab, or its first letter. */
export function Favicon({ url, title, src, size = 16 }: { url: string; title: string; src?: string; size?: number }) {
  if (src && /^(https?|data):/.test(src)) return <img className="x-fav" src={src} width={size} height={size} alt="" />;
  let hue = 0;
  for (const c of url) hue = (hue * 31 + c.charCodeAt(0)) % 360;
  return <span className="x-fav x-fav-letter" style={{ width: size, height: size, background: `hsl(${hue} 55% 42%)` }}>{(title.trim()[0] ?? "?").toUpperCase()}</span>;
}
