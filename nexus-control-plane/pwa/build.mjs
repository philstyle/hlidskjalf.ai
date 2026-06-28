import * as esbuild from "esbuild";
import { readFileSync, writeFileSync, cpSync, mkdirSync, rmSync } from "fs";
import { basename, dirname, resolve } from "path";
import { fileURLToPath } from "url";

const __dirname = dirname(fileURLToPath(import.meta.url));
const srcdir = resolve(__dirname, "src");
const iconsdir = resolve(__dirname, "icons");
const outdir = resolve(__dirname, "../src-tauri/pwa-dist");

// Clean + create output dir
rmSync(outdir, { recursive: true, force: true });
mkdirSync(outdir, { recursive: true });

const result = await esbuild.build({
  entryPoints: [
    resolve(srcdir, "app.js"),
    resolve(srcdir, "app.css"),
  ],
  bundle: true,
  minify: true,
  entryNames: "[name]-[hash]",
  outdir,
  metafile: true,
  loader: { ".woff2": "file" },
});

// Extract hashed filenames from metafile
let jsFile = "";
let cssFile = "";
for (const output of Object.keys(result.metafile.outputs)) {
  const name = basename(output);
  if (name.endsWith(".js")) jsFile = name;
  if (name.endsWith(".css")) cssFile = name;
}

// Read index.html template and replace placeholders
let html = readFileSync(resolve(srcdir, "index.html"), "utf-8");
html = html.replace("__JS__", jsFile);
html = html.replace("__CSS__", cssFile);
writeFileSync(resolve(outdir, "index.html"), html);

// Copy static assets
cpSync(resolve(srcdir, "manifest.json"), resolve(outdir, "manifest.json"));
cpSync(resolve(srcdir, "sw.js"), resolve(outdir, "sw.js"));
cpSync(iconsdir, resolve(outdir, "icons"), { recursive: true });

console.log(`PWA built → ${outdir}/`);
console.log(`  index.html, ${jsFile}, ${cssFile}, manifest.json, icons/`);
