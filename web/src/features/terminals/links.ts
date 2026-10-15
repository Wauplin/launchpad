/** Terminal output is untrusted, including destinations hidden behind OSC 8 labels. */
export function terminalLinkUrl(uri: string): string | null {
  try {
    const url = new URL(uri);
    return url.protocol === "http:" || url.protocol === "https:" ? url.href : null;
  } catch { return null; }
}

export function terminalLinkModifier(event: Pick<MouseEvent, "ctrlKey" | "metaKey" | "altKey" | "shiftKey" | "button">, mac: boolean): boolean {
  return event.button === 0 && !event.altKey && !event.shiftKey && (mac ? event.metaKey : event.ctrlKey);
}
