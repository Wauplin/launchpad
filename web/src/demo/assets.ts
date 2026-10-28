// Download and image links (`daemonUrl`) in demo mode: data URLs built from the world, never a daemon request.

export function demoAsset(path: string): string {
  return `data:text/plain;charset=utf-8,${encodeURIComponent(`Demo: ${path.split("?")[0]} is not available in the demo.`)}`;
}
