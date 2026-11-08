# NoriDrive visual identity

The folded **N** represents two copies connected by one continuous ribbon. Forest green keeps the mark calm, and the mint fold distinguishes the transfer between them. No third-party logos, stock images or external fonts are embedded.

- `icon.svg`: scalable application tile, also used as the website favicon.
- `logo.svg` / `logo-white.svg`: horizontal wordmarks for light / dark surfaces. The text uses local system font fallbacks and remains editable.
- `symbol.svg`: standalone two-tone mark on a transparent background.
- `symbol-monochrome.svg` / `symbol-white.svg`: single-color variants for compact uses.
- `icon.png`: 512px export.
- `logo.png`: transparent 1160px wordmark export.

The vector sources contain paths, shapes and editable text; no raster images are embedded. Keep at least one half of a stem's width clear around the mark. Use the app tile at small sizes; do not reduce the full wordmark to tray-icon size.

## Regenerate platform assets

Install ImageMagick, librsvg (`rsvg-convert`) and Python 3, then run from the repository root:

```sh
python3 scripts/export-branding.py
```

The script copies the canonical icon to Linux and the website, writes Windows PNG package assets at their exact sizes and creates a multi-resolution ICO (16–256px). It also exports the wordmark and the 512px icon. On Windows the executable is `magick`; on ImageMagick 6 Linux it is `convert`. These are authoring prerequisites only; normal builds use the committed graphics.
