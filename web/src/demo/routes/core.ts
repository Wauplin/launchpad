// Status, configuration, workspaces, folders and harness metadata.

import { route } from "../daemon";
import { world } from "../world";

route("GET", "/api/status", () => world.status);
route("GET", "/api/config", () => ({ config: world.config, error: null, remote: false }));
route("GET", "/api/workspaces", () => world.workspaces);
route("GET", "/api/projects", () => world.projects);
route("POST", "/api/log", () => ({ ok: true }));
