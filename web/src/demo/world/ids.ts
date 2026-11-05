// Fixed IDs of the demo world, so every run and every chapter looks the same.

export const WS = { acme: "demo-ws-acme", personal: "demo-ws-personal" } as const;
export const P = {
  storefront: "demo-storefront",
  billing: "demo-billing-api",
  docs: "demo-docs",
  dotfiles: "demo-dotfiles",
  scratch: "demo-tmp-csv",
  ask: "demo-ask-folder",
} as const;
export const ROOT = "/home/demo/projects";
/** Temporary folders live in the data folder, outside the folders root. */
export const TMP = "/home/demo/.local/share/launchpad/tmp";
export const ORG = "acme-labs";
export const USER = "demo-user";
