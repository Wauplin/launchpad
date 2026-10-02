import claudeLogo from "@/assets/claude.svg";
import codexLogo from "@/assets/codex.svg";

export type Harness = "claude" | "codex";

export const HARNESS_NAME: Record<Harness, string> = { claude: "Claude Code", codex: "Codex" };

/** The official logo of a harness. Used everywhere a harness appears, without a text label. */
export function HarnessMark({ harness, size = 16, className = "" }: { harness: Harness; size?: number; className?: string }) {
  return (
    <img
      src={harness === "claude" ? claudeLogo : codexLogo}
      width={size}
      height={size}
      alt={HARNESS_NAME[harness]}
      title={HARNESS_NAME[harness]}
      draggable={false}
      className={`inline-block flex-none select-none ${className}`}
    />
  );
}
