// Content script of https://github.com/* (isolated world): the GitHub layer. It holds no credentials and reaches Home
// only through the worker (`request` and `subscribe` of shared/messages.ts). Built as one classic script
// (vite.content.config.ts): content scripts cannot load modules. The parts live in content/github/: page kinds
// (page.ts), anchors (anchors.ts, the one registry), soft navigation (navigation.ts), Shadow DOM widgets (widgets.ts)
// and the features (repoBadge.ts…), run by main.ts. It never clicks or submits GitHub's own controls.

import { start } from "./github/main";

start();
