# Harness marks

- `claude.svg`: the Claude logo from Anthropic's Claude Code VS Code extension (`resources/claude-logo.svg`, version 2.1.287), unchanged.
- `codex.svg`: the Codex app mark with the gradient of the official app icon. The vector shape comes from `@lobehub/icons-static-svg` 1.95.1 (`icons/codex.svg`, MIT); the Codex VS Code extension ships a PNG.
- `opencode.svg`: a simple original geometric O glyph for this integration, not an official upstream logo.

The Rust descriptor embeds its asset and returns it through `/api/harnesses`. React uses that descriptor rather than keeping a separate harness-to-asset table.
