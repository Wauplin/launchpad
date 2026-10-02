# Launchpad icon

The selected sculpted launch artwork is an ivory folded arrow with a mint core and coral exhaust on a teal rounded tile. It was generated with imagegen and selected on 2026-10-02. Transparency is preserved.

- `source.png`: the unchanged 1254 × 1254 generated master.
- `16x16.png` through `1024x1024.png`: PNG exports for desktop and browser use.
- `launchpad.webp`: a lossless 512 × 512 WebP export.
- `../launchpad.svg`: a self-contained SVG wrapper around the 1024px PNG, not vector artwork.
- `../../desktop/src-tauri/icons/`: the 32px, 128px, 256px (`128x128@2x.png`) and 512px (`icon.png`) PNGs referenced by Tauri and the full installer.
- `../../web/src/assets/launchpad.png`: the 64px web export, bundled by Vite and used as the browser favicon and the top-left logo in the app.

Regenerate all exports with `python3 packaging/generate-icons.py` (requires Pillow). Keep the master unchanged; derive sizes from it rather than from a smaller export. Run `packaging/install-icons.sh` to update the installed Ubuntu icon and refresh the icon cache without restarting the app or daemon. An already open browser tab may need a reload to display the new favicon.
