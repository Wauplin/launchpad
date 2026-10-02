// Shiki in a Web Worker (ARCHITECTURE.md, section 8.1): the code renders as plain text first and gets
// its colors when this answers. Grammars load on demand, one chunk per language.

import { createHighlighterCore } from "shiki/core";
import { createJavaScriptRegexEngine } from "shiki/engine/javascript";
import { bundledLanguages } from "shiki/langs";
import githubLight from "shiki/themes/github-light.mjs";

/** Per line: [text, color, fontStyle] for each token. */
export type Tokens = [string, string | undefined, number | undefined][][];

const highlighter = createHighlighterCore({ themes: [githubLight], langs: [], engine: createJavaScriptRegexEngine({ forgiving: true }) });

self.onmessage = async (e: MessageEvent<{ id: number; code: string; lang: string }>) => {
  const { id, code, lang } = e.data;
  try {
    const h = await highlighter;
    if (!h.getLoadedLanguages().includes(lang)) {
      const load = bundledLanguages[lang as keyof typeof bundledLanguages];
      if (!load) throw new Error(`no grammar for ${lang}`);
      await h.loadLanguage(load);
    }
    const lines = h.codeToTokensBase(code, { lang, theme: "github-light" });
    const tokens: Tokens = lines.map((l) => l.map((t) => [t.content, t.color, t.fontStyle]));
    self.postMessage({ id, tokens });
  } catch (err) {
    self.postMessage({ id, error: String(err) });
  }
};
