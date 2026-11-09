# Launchpad icon

The selected sculpted launch artwork is an ivory folded arrow with a mint core and coral exhaust on a teal rounded tile. It was generated with imagegen and selected on 2026-10-02. Transparency is preserved.

- `source.png`: the unchanged 1254 × 1254 generated master.
- `16x16.png` through `1024x1024.png`: PNG exports.
- `launchpad.webp`: a lossless 512 × 512 WebP export.
- `../../web/src/assets/launchpad.png`: the 64px web export, bundled by Vite and used as the browser favicon and the top-left logo in the app.

Regenerate all exports with `python3 packaging/generate-icons.py` (requires Pillow). Keep the master unchanged; derive sizes from it rather than from a smaller export. The installable web app's icons are in `web/public/icons/`. An already open browser tab may need a reload to display the new favicon.
