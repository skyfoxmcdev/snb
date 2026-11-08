// Generates the Sky & Beyond mark as a 1024x1024 PNG, then the Tauri CLI derives
// .ico/.icns/platform sizes from it. Pure Node: no native image dependency.
import { deflateSync } from "node:zlib";
import { mkdirSync, writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname } from "node:path";

const W = 1024;
const SS = 2;
const N = W * SS;

const outPath = fileURLToPath(new URL("../src-tauri/icons/source.png", import.meta.url));
mkdirSync(dirname(outPath), { recursive: true });

function mix(a, b, t) {
  const k = Math.max(0, Math.min(1, t));
  return [
    a[0] + (b[0] - a[0]) * k,
    a[1] + (b[1] - a[1]) * k,
    a[2] + (b[2] - a[2]) * k,
  ];
}

function clamp01(v) {
  return Math.max(0, Math.min(1, v));
}

function smooth(t) {
  const x = clamp01(t);
  return x * x * (3 - 2 * x);
}

function insideRoundedSquare(px, py, center, half, radius) {
  const qx = Math.abs(px - center) - (half - radius);
  const qy = Math.abs(py - center) - (half - radius);
  const ax = Math.max(qx, 0);
  const ay = Math.max(qy, 0);
  const outside = Math.hypot(ax, ay);
  const inside = Math.min(Math.max(qx, qy), 0);
  return outside + inside - radius <= 0;
}

function starPolygon(cx, cy, outer, inner, points = 4) {
  const verts = [];
  for (let i = 0; i < points * 2; i++) {
    const angle = (-Math.PI / 2) + (i * Math.PI) / points;
    const radius = i % 2 === 0 ? outer : inner;
    verts.push([cx + Math.cos(angle) * radius, cy + Math.sin(angle) * radius]);
  }
  return verts;
}

function insidePolygon(x, y, poly) {
  let inside = false;
  for (let i = 0, j = poly.length - 1; i < poly.length; j = i++) {
    const [xi, yi] = poly[i];
    const [xj, yj] = poly[j];
    if (yi > y !== yj > y && x < ((xj - xi) * (y - yi)) / (yj - yi) + xi) {
      inside = !inside;
    }
  }
  return inside;
}

// Deterministic star field.
let seed = 0x5b1d4e;
function rnd() {
  seed = (seed * 1664525 + 1013904223) >>> 0;
  return seed / 0xffffffff;
}
const stars = Array.from({ length: 34 }, () => {
  const x = 120 + rnd() * 784;
  const y = 90 + rnd() * 500;
  const size = 0.6 + rnd() * 2.1;
  const alpha = 0.18 + rnd() * 0.55;
  return { x, y, size, alpha };
});

const bigStar = starPolygon(512, 408, 108, 25, 4);
const smallStarA = starPolygon(710, 276, 25, 6, 4);
const smallStarB = starPolygon(332, 306, 16, 4, 4);
const ringCenter = [512, 430];
const ringRadius = 292;
const horizonCenter = [512, 1040];
const horizonRx = 430;
const horizonRy = 326;

function sample(px, py) {
  if (!insideRoundedSquare(px, py, 512, 512, 224)) {
    return [0, 0, 0, 0];
  }

  const vertical = py / 1024;
  let col = mix([0x04, 0x0a, 0x17], [0x0e, 0x1d, 0x34], smooth(vertical * 1.15));
  col = mix(col, [0x08, 0x12, 0x24], smooth((py - 620) / 500) * 0.55);

  // A soft moonlit halo behind the mark.
  const halo = Math.exp(-(((px - 512) ** 2 + (py - 400) ** 2) / (2 * 250 ** 2)));
  col = mix(col, [0x72, 0xa7, 0xdc], halo * 0.16);

  // Star field, fading toward the horizon.
  const starFade = clamp01(1 - (py - 80) / 660);
  for (const star of stars) {
    const d = Math.hypot(px - star.x, py - star.y);
    if (d < star.size * 3.8) {
      const glow = Math.exp(-(d * d) / (2 * (star.size * 1.7) ** 2));
      col = mix(col, [0xea, 0xf4, 0xff], glow * star.alpha * starFade);
    }
  }

  // Fine orbital ring.
  const ringDist = Math.abs(Math.hypot(px - ringCenter[0], py - ringCenter[1]) - ringRadius);
  if (ringDist < 2.2) {
    const ringAlpha = (1 - ringDist / 2.2) * 0.18;
    col = mix(col, [0xbf, 0xd9, 0xff], ringAlpha);
  }

  // Aurora horizon: a luminous ellipse edge with a three-colour sweep.
  const nx = (px - horizonCenter[0]) / horizonRx;
  const ny = (py - horizonCenter[1]) / horizonRy;
  const ellipse = Math.hypot(nx, ny);
  const edge = Math.exp(-((ellipse - 1) ** 2) / (2 * 0.037 ** 2));
  if (py < 880 && edge > 0.004) {
    const t = clamp01((px - 130) / 764);
    let aurora = mix([0x65, 0xd8, 0xe6], [0x9f, 0xb3, 0xff], smooth(t));
    aurora = mix(aurora, [0xff, 0xd3, 0x98], smooth((t - 0.62) / 0.38));
    const horizonFade = clamp01(1 - Math.max(0, py - 760) / 150);
    col = mix(col, aurora, edge * 0.78 * horizonFade);
    const glow = Math.exp(-((ellipse - 1) ** 2) / (2 * 0.15 ** 2));
    col = mix(col, aurora, glow * 0.13 * horizonFade);
  }

  // The principal four-point star and two companions.
  const starGlow = Math.exp(-(((px - 512) ** 2 + (py - 408) ** 2) / (2 * 170 ** 2)));
  col = mix(col, [0xa9, 0xd6, 0xff], starGlow * 0.1);
  if (insidePolygon(px, py, bigStar)) {
    const g = mix([0xff, 0xff, 0xf4], [0xff, 0xdc, 0x9c], clamp01((py - 360) / 150));
    col = mix(col, g, 0.97);
  }
  if (insidePolygon(px, py, smallStarA)) col = mix(col, [0xff, 0xf2, 0xd0], 0.9);
  if (insidePolygon(px, py, smallStarB)) col = mix(col, [0xdf, 0xf2, 0xff], 0.82);

  // Gentle vignette.
  const radial = clamp01(Math.hypot(px - 512, py - 512) / 720);
  const vignette = 1 - 0.28 * smooth(radial);
  col = col.map((v) => v * vignette);

  return [col[0], col[1], col[2], 255];
}

const pixels = Buffer.alloc(W * W * 4);
for (let oy = 0; oy < W; oy++) {
  for (let ox = 0; ox < W; ox++) {
    let sr = 0, sg = 0, sb = 0, sa = 0;
    for (let sy = 0; sy < SS; sy++) {
      for (let sx = 0; sx < SS; sx++) {
        const px = (ox + (sx + 0.5) / SS);
        const py = (oy + (sy + 0.5) / SS);
        const [r, g, b, a] = sample(px, py);
        const af = a / 255;
        sr += r * af;
        sg += g * af;
        sb += b * af;
        sa += af;
      }
    }
    const n = SS * SS;
    const a = sa / n;
    const i = (oy * W + ox) * 4;
    if (a <= 0.0001) {
      pixels[i] = pixels[i + 1] = pixels[i + 2] = pixels[i + 3] = 0;
    } else {
      pixels[i] = Math.round(sr / sa);
      pixels[i + 1] = Math.round(sg / sa);
      pixels[i + 2] = Math.round(sb / sa);
      pixels[i + 3] = Math.round(a * 255);
    }
  }
}

// PNG encoding.
const crcTable = new Uint32Array(256);
for (let n = 0; n < 256; n++) {
  let c = n;
  for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
  crcTable[n] = c >>> 0;
}
function crc32(buf) {
  let c = 0xffffffff;
  for (const byte of buf) c = crcTable[(c ^ byte) & 0xff] ^ (c >>> 8);
  return (c ^ 0xffffffff) >>> 0;
}
function chunk(type, data) {
  const typeBuf = Buffer.from(type, "ascii");
  const body = Buffer.concat([typeBuf, data]);
  const out = Buffer.alloc(4 + body.length + 4);
  out.writeUInt32BE(data.length, 0);
  body.copy(out, 4);
  out.writeUInt32BE(crc32(body), 4 + body.length);
  return out;
}
const ihdr = Buffer.alloc(13);
ihdr.writeUInt32BE(W, 0);
ihdr.writeUInt32BE(W, 4);
ihdr[8] = 8; // bit depth
ihdr[9] = 6; // RGBA
const stride = W * 4;
const raw = Buffer.alloc((stride + 1) * W);
for (let y = 0; y < W; y++) {
  raw[y * (stride + 1)] = 0;
  pixels.copy(raw, y * (stride + 1) + 1, y * stride, (y + 1) * stride);
}
const png = Buffer.concat([
  Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
  chunk("IHDR", ihdr),
  chunk("IDAT", deflateSync(raw, { level: 9 })),
  chunk("IEND", Buffer.alloc(0)),
]);
writeFileSync(outPath, png);
console.log(`Wrote ${outPath} (${W}x${W}, ${(png.length / 1024).toFixed(1)} KiB)`);
