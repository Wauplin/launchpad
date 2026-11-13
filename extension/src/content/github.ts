// Content script of https://github.com/* (isolated world): the GitHub layer. It holds no credentials and reaches Home
// only through the worker (`request` and `subscribe` of shared/messages.ts). Built as one classic script
// (vite.content.config.ts): content scripts cannot load modules. The page kinds, anchors and widgets come next.

export {};
