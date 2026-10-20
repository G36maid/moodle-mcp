// Generates sitemap.xml from the pages that were actually built, so it can
// never drift from the deploy (the hand-maintained copy once listed a page
// that no longer existed and 404'd to crawlers).
//
// Usage (from the repo root, e.g. in the Pages workflow):
//   node site/scripts/generate-sitemap.mjs --site https://moodle.ntnu.cc \
//     --astro site/dist --book docs/book --out _site/sitemap.xml
//
// Astro emits the landing page, mdBook emits the docs tree under /docs/.
// lastmod comes from the last commit touching each page's source file; a
// page whose source can't be located falls back to the newest commit date,
// and the whole run is deterministic so regenerating is diff-clean.

import { execFileSync } from "node:child_process";
import { readdirSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const args = process.argv.slice(2);
function flag(name) {
  const i = args.indexOf(`--${name}`);
  if (i === -1 || !args[i + 1]) throw new Error(`missing --${name} <value>`);
  return args[i + 1];
}

const siteUrl = flag("site").replace(/\/$/, "");
const astroDir = resolve(flag("astro"));
const bookDir = resolve(flag("book"));
const outPath = resolve(flag("out"));
const repoRoot = resolve(fileURLToPath(new URL("../..", import.meta.url)));

function htmlFiles(dir, prefix = "") {
  const out = [];
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const rel = prefix ? `${prefix}/${entry.name}` : entry.name;
    if (entry.isDirectory()) out.push(...htmlFiles(join(dir, entry.name), rel));
    else if (entry.name.endsWith(".html")) out.push(rel);
  }
  return out;
}

// mdBook's print/404 variants are for humans and error handling, not indexes;
// Astro copies public/ verbatim into dist/, dragging the Search Console
// verification stub along with it — none of them are pages.
const EXCLUDED = new Set(["print.html", "404.html", "googleeb492e7405c8e489.html"]);

const pages = [];
for (const rel of htmlFiles(astroDir)) {
  if (EXCLUDED.has(rel)) continue;
  const stem = rel.replace(/\.html$/, "");
  pages.push({
    path: rel === "index.html" ? "/" : `/${rel.replace(/index\.html$/, "")}`,
    source: [join("site/src/pages", `${stem}.astro`), join("site/src/pages", stem, "index.astro")],
  });
}
for (const rel of htmlFiles(bookDir)) {
  if (EXCLUDED.has(rel)) continue;
  // The book's index is assembled from SUMMARY.md, so that file is its
  // lastmod source rather than a nonexistent src/index.md.
  const source =
    rel === "index.html"
      ? "docs/src/SUMMARY.md"
      : ["", "index", "README"].map((stem) => join("docs/src", `${rel.replace(/\.html$/, "")}${stem && `/${stem}`}.md`));
  pages.push({
    path: rel === "index.html" ? "/docs/" : `/docs/${rel.replace(/index\.html$/, "")}`,
    source,
  });
}

const git = (args_) =>
  execFileSync("git", ["-C", repoRoot, ...args_], { encoding: "utf8" }).trim();
const newestCommit = git(["log", "-1", "--format=%cs"]);

// A page not yet under version control (first deploy of a new page) has no
// history; the newest commit date keeps the entry valid rather than empty.
const lastMod = (sources) => {
  for (const source of [sources].flat()) {
    const date = git(["log", "-1", "--format=%cs", "--", source]);
    if (date) return date;
  }
  return newestCommit;
};

const escapeXml = (s) => s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");

pages.sort((a, b) => (a.path === "/" ? -1 : b.path === "/" ? 1 : a.path.localeCompare(b.path)));
const xml = `<?xml version="1.0" encoding="UTF-8"?>
<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">
${pages
  .map((p) => {
    const loc = escapeXml(`${siteUrl}${p.path}`);
    return `  <url><loc>${loc}</loc><lastmod>${lastMod(p.source)}</lastmod></url>`;
  })
  .join("\n")}
</urlset>
`;
writeFileSync(outPath, xml);
console.log(`sitemap: ${pages.length} URLs -> ${outPath}`);
