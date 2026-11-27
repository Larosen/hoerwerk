// Public-domain artwork for the demo covers, from Wikimedia Commons (cached in scripts/screenshots/.art).
// Only files that Commons marks as public domain are used; artist, licence and source go to CREDITS.md.
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";

const DIR = new URL(".art/", import.meta.url).pathname;
const API = "https://commons.wikimedia.org/w/api.php";
const UA = { "User-Agent": "hoerwerk-readme-screenshots/1.0 (demo covers)" };

const strip = (html = "") => html.replace(/<[^>]+>/g, "").replace(/\s+/g, " ").trim();

async function search(query) {
  const u = new URL(API);
  Object.entries({
    action: "query", format: "json", generator: "search", gsrsearch: `${query} filetype:bitmap`, gsrnamespace: "6", gsrlimit: "8",
    prop: "imageinfo", iiprop: "url|extmetadata|size", iiurlwidth: "900",
  }).forEach(([k, v]) => u.searchParams.set(k, v));
  let res;
  for (let wait = 5000; ; wait *= 2) {                    // Commons asks for patience when asked too often
    res = await fetch(u, { headers: UA });
    if (res.ok && (res.headers.get("content-type") || "").includes("json")) break;
    if (wait > 80000) throw new Error(`Commons: ${res.status}`);
    await new Promise((r) => setTimeout(r, wait));
  }
  const pages = Object.values((await res.json()).query?.pages || {}).sort((a, b) => a.index - b.index);
  for (const p of pages) {
    const ii = p.imageinfo?.[0];
    const m = ii?.extmetadata || {};
    const licence = strip(m.LicenseShortName?.value);
    if (!ii || !/public domain|^pd/i.test(licence) || ii.width < 400) continue;
    return { file: p.title, thumb: ii.thumburl, page: ii.descriptionurl, artist: strip(m.Artist?.value) || "?", licence, date: strip(m.DateTimeOriginal?.value) };
  }
  return null;
}

/** id → { path, credit } for every book with a query; downloads only what is not cached. */
export async function fetchArt(books) {
  mkdirSync(DIR, { recursive: true });
  const index = join(DIR, "index.json");
  const known = existsSync(index) ? JSON.parse(readFileSync(index, "utf8")) : {};
  for (const b of books) {
    if (!b.art || (known[b.id]?.query === b.art && existsSync(join(DIR, `${b.id}.jpg`)))) continue;
    const hit = await search(b.art);
    await new Promise((r) => setTimeout(r, 1500));
    if (!hit) { console.warn(`no public-domain picture for ${b.title}: ${b.art}`); delete known[b.id]; continue; }
    const img = await fetch(hit.thumb, { headers: UA });
    writeFileSync(join(DIR, `${b.id}.jpg`), Buffer.from(await img.arrayBuffer()));
    known[b.id] = { ...hit, query: b.art };
    await new Promise((r) => setTimeout(r, 1500));         // polite to Commons
  }
  writeFileSync(index, JSON.stringify(known, null, 1));
  return Object.fromEntries(Object.entries(known).map(([id, c]) => [id, { path: join(DIR, `${id}.jpg`), credit: c }]));
}

export function creditsMarkdown(books, art) {
  const rows = books.filter((b) => art[b.id]).map((b) => {
    const c = art[b.id].credit;
    return `| ${b.title} | ${c.artist.replace(/\|/g, "/")}${c.date ? ` (${c.date.replace(/date QS.*$/, "").trim().slice(0, 40)})` : ""} | ${c.licence} | [${c.file.replace(/^File:/, "")}](${c.page}) |`;
  });
  return `# Pictures in the screenshots

The covers of the demo library combine public-domain artwork from Wikimedia Commons with Hörwerk's own typography.
The books are public-domain works. \`scripts/screenshots/shoot.mjs\` makes the screenshots.

| Book | Artist | Licence | Source |
|---|---|---|---|
${rows.join("\n")}
`;
}
