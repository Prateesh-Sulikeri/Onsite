// Bundles src/*.js into ../ui/vendor/ (the app itself has no build step).
//  - editor.js: one plain script, loaded by index.html.
//  - whiteboard/: ES modules with code splitting, imported the first time the Whiteboard tab opens.
//    Excalidraw's rarely used parts (Mermaid import, etc.) stay in chunks that load only if used.
// Excalidraw's fonts are copied next to it; the CJK font (Xiaolai, ~200 files) is left out.
import { build } from "esbuild";
import { cpSync, existsSync, mkdirSync, readdirSync, rmSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

const out = fileURLToPath(new URL("../ui/vendor/", import.meta.url));
rmSync(out, { recursive: true, force: true });
mkdirSync(out, { recursive: true });

const common = {
  bundle: true,
  minify: true,
  target: "chrome120",
  legalComments: "linked",
  conditions: ["production"],
  define: { "process.env.NODE_ENV": '"production"', "process.env.IS_PREACT": '"false"' },
  logLevel: "info",
};

await build({ ...common, format: "iife", entryPoints: ["src/editor.js"], outfile: join(out, "editor.js") });
await build({
  ...common,
  format: "esm",
  splitting: true,
  entryPoints: ["src/whiteboard.js"],
  outdir: join(out, "whiteboard"),
  chunkNames: "chunks/[name]-[hash]",
  loader: { ".woff2": "file", ".ttf": "file" },
  assetNames: "assets/[name]-[hash]",
});

// Excalidraw fetches fonts from window.EXCALIDRAW_ASSET_PATH + "fonts/<Family>/<file>".
const pkg = "node_modules/@excalidraw/excalidraw/dist/prod/fonts";
if (!existsSync(pkg)) throw new Error(`Excalidraw fonts not found at ${pkg}`);
for (const dir of readdirSync(pkg)) {
  if (dir === "Xiaolai") continue;
  cpSync(join(pkg, dir), join(out, "excalidraw", "fonts", dir), { recursive: true });
}
