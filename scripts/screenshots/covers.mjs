// Own cover art for the demo library: one SVG per book (motif + palette + typography), rendered to PNG.

const PALETTES = [
  { bg: "#1f4d3f", bg2: "#2f6b57", ink: "#f3e9d2", acc: "#d9b36a" },
  { bg: "#5e1a20", bg2: "#8e2f2f", ink: "#f2e6cf", acc: "#e0b15c" },
  { bg: "#1b2744", bg2: "#2c4170", ink: "#efe6d0", acc: "#e5c07b" },
  { bg: "#b9862a", bg2: "#e0b04a", ink: "#2b2118", acc: "#7a2e1f" },
  { bg: "#17545b", bg2: "#2f8a8f", ink: "#f4ecd8", acc: "#f2c879" },
  { bg: "#43203f", bg2: "#6e3a66", ink: "#f3e7d7", acc: "#e9b872" },
  { bg: "#e9dcc0", bg2: "#f5ecd8", ink: "#2c2a26", acc: "#9b2f2a" },
  { bg: "#2b313c", bg2: "#4c566a", ink: "#eceff4", acc: "#d08770" },
  { bg: "#7f3519", bg2: "#b5562a", ink: "#f7ead5", acc: "#f1d29a" },
  { bg: "#9fbfd0", bg2: "#dbe8ee", ink: "#1f3443", acc: "#b04a3a" },
];

const SERIF = "Caladea, 'Bitstream Charter', 'Liberation Serif', serif";
const SANS = "Inter, 'Liberation Sans', sans-serif";

function esc(s) { return s.replace(/&/g, "&amp;").replace(/</g, "&lt;"); }

function wrap(text, max) {
  const lines = [];
  let line = "";
  for (const w of text.split(" ")) {
    if ((line + " " + w).trim().length > max && line) { lines.push(line); line = w; } else line = (line + " " + w).trim();
  }
  if (line) lines.push(line);
  return lines;
}

/** Motifs in the middle of the cover (drawn in a 600×600 box). */
const MOTIFS = {
  crime: (p) => `
    <g opacity=".9"><circle cx="300" cy="270" r="92" fill="none" stroke="${p.acc}" stroke-width="14"/>
    <circle cx="300" cy="270" r="70" fill="${p.ink}" opacity=".08"/>
    <line x1="366" y1="336" x2="452" y2="422" stroke="${p.acc}" stroke-width="26" stroke-linecap="round"/>
    <path d="M262 236 q38 -22 76 0" stroke="${p.ink}" stroke-width="6" fill="none" opacity=".5"/></g>
    ${[0, 1, 2].map((i) => `<path d="M60 ${420 + i * 22} q60 -14 120 0 t120 0 t120 0 t120 0" stroke="${p.ink}" stroke-width="3" fill="none" opacity="${0.18 - i * 0.04}"/>`).join("")}`,
  gothic: (p) => `
    <circle cx="390" cy="200" r="78" fill="${p.acc}" opacity=".95"/>
    <circle cx="418" cy="186" r="70" fill="${p.bg}" opacity=".35"/>
    <path d="M110 430 V330 l20 -30 l20 30 V300 h30 v-60 l25 -40 l25 40 v60 h40 V330 l20 -30 l20 30 V280 l30 -50 l30 50 v150 z" fill="${p.ink}" opacity=".16"/>
    <path d="M120 430 V340 l18 -26 l18 26 V310 h26 v-56 l22 -36 l22 36 v56 h34 V340 l18 -26 l18 26 V296 l26 -44 l26 44 v134 z" fill="#000" opacity=".45"/>
    ${[[200, 170], [250, 140], [470, 290]].map(([x, y]) => `<path d="M${x} ${y} q8 -10 16 0 q8 -10 16 0 q-8 -2 -16 6 q-8 -8 -16 -6z" fill="#000" opacity=".5"/>`).join("")}`,
  sea: (p) => `
    <circle cx="300" cy="230" r="70" fill="${p.acc}" opacity=".85"/>
    <path d="M250 330 l50 -150 l4 150 z" fill="${p.ink}" opacity=".9"/><path d="M310 330 l2 -120 l50 120 z" fill="${p.ink}" opacity=".75"/>
    <path d="M226 336 h150 l-22 26 h-108 z" fill="#000" opacity=".5"/>
    ${[0, 1, 2, 3].map((i) => `<path d="M40 ${370 + i * 24} q40 -18 80 0 t80 0 t80 0 t80 0 t80 0 t80 0 t80 0" stroke="${p.ink}" stroke-width="5" fill="none" opacity="${0.55 - i * 0.12}"/>`).join("")}`,
  west: (p) => `
    <circle cx="300" cy="300" r="120" fill="${p.acc}" opacity=".9"/>
    ${[0, 1, 2, 3, 4].map((i) => `<rect x="160" y="${250 + i * 22}" width="280" height="8" fill="${p.bg}" opacity=".55"/>`).join("")}
    <path d="M40 430 l120 -120 l60 50 l90 -110 l110 120 l70 -50 l70 110 z" fill="#000" opacity=".42"/>
    <path d="M40 430 l120 -96 l60 40 l90 -88 l110 96 l70 -40 l70 88 z" fill="${p.bg}" opacity=".85"/>`,
  space: (p) => `
    ${Array.from({ length: 40 }, (_, i) => `<circle cx="${(i * 137) % 560 + 20}" cy="${(i * 89) % 280 + 120}" r="${(i % 3) + 1}" fill="${p.ink}" opacity=".6"/>`).join("")}
    <circle cx="300" cy="280" r="90" fill="${p.acc}"/>
    <circle cx="270" cy="250" r="90" fill="${p.bg2}" opacity=".35"/>
    <ellipse cx="300" cy="280" rx="170" ry="34" fill="none" stroke="${p.ink}" stroke-width="7" opacity=".85" transform="rotate(-14 300 280)"/>`,
  fairy: (p) => `
    ${Array.from({ length: 14 }, (_, i) => { const x = (i * 97) % 520 + 40, y = (i * 61) % 260 + 130, r = 6 + (i % 3) * 4; return `<path d="M${x} ${y - r} L${x + r / 3} ${y - r / 3} L${x + r} ${y} L${x + r / 3} ${y + r / 3} L${x} ${y + r} L${x - r / 3} ${y + r / 3} L${x - r} ${y} L${x - r / 3} ${y - r / 3} Z" fill="${p.acc}" opacity=".8"/>`; }).join("")}
    <path d="M300 160 c-60 40 -90 90 -90 140 c0 50 40 80 90 80 c50 0 90 -30 90 -80 c0 -50 -30 -100 -90 -140z" fill="${p.ink}" opacity=".12"/>
    <path d="M300 430 v-110" stroke="${p.ink}" stroke-width="10" opacity=".7"/>
    <circle cx="300" cy="275" r="62" fill="${p.acc}" opacity=".9"/><circle cx="262" cy="310" r="40" fill="${p.acc}" opacity=".75"/><circle cx="340" cy="312" r="44" fill="${p.acc}" opacity=".7"/>`,
  deco: (p) => `
    ${Array.from({ length: 19 }, (_, i) => { const a = Math.PI + (i / 18) * Math.PI; return `<line x1="300" y1="330" x2="${300 + Math.cos(a) * 230}" y2="${330 + Math.sin(a) * 230}" stroke="${p.acc}" stroke-width="3" opacity=".55"/>`; }).join("")}
    <circle cx="300" cy="330" r="96" fill="${p.acc}"/><circle cx="300" cy="330" r="76" fill="${p.bg}" />
    <circle cx="300" cy="330" r="58" fill="none" stroke="${p.acc}" stroke-width="4"/>`,
  frame: (p) => `
    <rect x="150" y="150" width="300" height="270" rx="150" fill="none" stroke="${p.acc}" stroke-width="5"/>
    <rect x="166" y="166" width="268" height="238" rx="134" fill="${p.ink}" opacity=".07"/>
    <path d="M300 200 c30 40 30 80 0 120 c-30 -40 -30 -80 0 -120z" fill="${p.acc}"/>
    <path d="M300 320 c40 -10 70 10 80 40 c-40 10 -70 -10 -80 -40z M300 320 c-40 -10 -70 10 -80 40 c40 10 70 -10 80 -40z" fill="${p.acc}" opacity=".8"/>`,
  mountain: (p) => `
    <circle cx="410" cy="190" r="46" fill="${p.acc}" opacity=".9"/>
    <path d="M30 430 l150 -210 l70 90 l90 -150 l230 270 z" fill="${p.ink}" opacity=".2"/>
    <path d="M30 430 l150 -180 l70 80 l90 -130 l230 230 z" fill="#000" opacity=".35"/>
    <path d="M300 290 l40 -40 l40 46 l-30 -10 l-20 20 l-16 -18 z" fill="${p.ink}" opacity=".85"/>`,
  city: (p) => `
    ${[[70, 250], [130, 200], [190, 270], [250, 170], [320, 230], [380, 150], [450, 240], [500, 200]].map(([x, y], i) => `<rect x="${x}" y="${y}" width="${48 + (i % 2) * 10}" height="${430 - y}" fill="#000" opacity="${0.3 + (i % 3) * 0.08}"/>`).join("")}
    ${Array.from({ length: 22 }, (_, i) => `<rect x="${80 + (i * 53) % 440}" y="${210 + (i * 37) % 190}" width="8" height="12" fill="${p.acc}" opacity=".85"/>`).join("")}
    <circle cx="300" cy="150" r="34" fill="${p.ink}" opacity=".8"/>`,
  rose: (p) => `
    <path d="M300 430 C 290 380 310 350 300 300" stroke="${p.ink}" stroke-width="7" fill="none" opacity=".6"/>
    <path d="M300 370 c-40 -10 -60 -40 -50 -60 c30 0 50 30 50 60z M300 350 c40 -10 60 -40 50 -60 c-30 0 -50 30 -50 60z" fill="${p.ink}" opacity=".45"/>
    ${[0, 1, 2, 3, 4].map((i) => `<ellipse cx="300" cy="${250}" rx="${70 - i * 12}" ry="${58 - i * 10}" fill="${i % 2 ? p.bg : p.acc}" opacity=".95" transform="rotate(${i * 25} 300 250)"/>`).join("")}`,
  letter: (p, b) => `
    <text x="300" y="400" text-anchor="middle" font-family="${SERIF}" font-size="300" font-weight="700" fill="${p.acc}" opacity=".9">${esc(b.title.replace(/^(Der|Die|Das) /, "")[0])}</text>
    <rect x="120" y="420" width="360" height="4" fill="${p.ink}" opacity=".4"/>`,
  sky: (p) => `
    <circle cx="300" cy="260" r="110" fill="${p.acc}" opacity=".25"/>
    ${[[200, 240], [250, 210], [300, 190], [350, 210], [400, 240], [280, 300]].map(([x, y], i) => `<path d="M${x - 22} ${y} q22 -${14 + i} 22 0 q0 -${14 + i} 22 0" stroke="${p.ink}" stroke-width="5" fill="none"/>`).join("")}
    <path d="M40 430 q130 -60 260 -20 t260 -10 v30z" fill="${p.bg}" opacity=".9"/>`,
};

export function coverSvg(b, size = 600) {
  const p = PALETTES[b.palette % PALETTES.length];
  const motif = (MOTIFS[b.motif] || MOTIFS.frame)(p, b);
  const lines = wrap(b.title, b.title.length > 22 ? 17 : 14);
  const fs = lines.length > 2 ? 44 : 54;
  const top = b.palette % 2 === 0;                     // title above or below the motif
  const ty = top ? 96 : 600 - 70 - (lines.length - 1) * fs * 1.08;
  const ay = top ? 560 : 70;
  const shift = top ? 40 : -50;
  return `<svg xmlns="http://www.w3.org/2000/svg" width="${size}" height="${size}" viewBox="0 0 600 600">
  <defs>
    <linearGradient id="g${b.id}" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="${p.bg2}"/><stop offset="1" stop-color="${p.bg}"/></linearGradient>
    <filter id="n${b.id}"><feTurbulence type="fractalNoise" baseFrequency=".9" numOctaves="2" stitchTiles="stitch"/><feColorMatrix values="0 0 0 0 0  0 0 0 0 0  0 0 0 0 0  0 0 0 .5 0"/></filter>
  </defs>
  <rect width="600" height="600" fill="url(#g${b.id})"/>
  <g transform="translate(0 ${shift})">${motif}</g>
  <rect width="600" height="600" filter="url(#n${b.id})" opacity=".35"/>
  <rect x="22" y="22" width="556" height="556" fill="none" stroke="${p.ink}" stroke-opacity=".35" stroke-width="2"/>
  ${lines.map((l, i) => `<text x="300" y="${ty + i * fs * 1.08}" text-anchor="middle" font-family="${SERIF}" font-weight="700" font-size="${fs}" fill="${p.ink}">${esc(l)}</text>`).join("")}
  <text x="300" y="${ay}" text-anchor="middle" font-family="${SANS}" font-weight="600" font-size="20" letter-spacing="5" fill="${p.ink}" opacity=".85">${esc(b.author.toUpperCase())}</text>
</svg>`;
}

/** All covers as PNG buffers (id → Buffer), drawn in the browser with the system fonts. */
export async function renderCovers(browser, books) {
  const page = await browser.newPage({ viewport: { width: 600, height: 600 } });
  const out = new Map();
  for (const b of books) {
    await page.setContent(`<html><body style="margin:0">${coverSvg(b)}</body></html>`);
    out.set(b.id, await page.locator("svg").screenshot({ type: "png" }));
  }
  await page.close();
  return out;
}
