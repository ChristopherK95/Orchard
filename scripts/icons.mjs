// Renders the app icon set from src-tauri/icons/orchard-icon.svg: the PNGs at 1024, 512, 256, 128,
// 64, 32 and 16 px, the Windows .ico and the macOS .icns. At 32 px and below the squircle holds the
// mark's compact cut (src/assets/orchard-mark-small.svg) instead, which keeps all five fruit
// readable; the 16 px raster also drops the squircle's hairline (there it only muddies the edge).
//
//   node scripts/icons.mjs
import { execFileSync } from "node:child_process";
import { copyFileSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const root = join(import.meta.dirname, "..");
const icons = join(root, "src-tauri", "icons");
const source = join(icons, "orchard-icon.svg");
const compactMark = join(root, "src", "assets", "orchard-mark-small.svg");
const temp = mkdtempSync(join(tmpdir(), "orchard-icons-"));

const tauriIcon = (input, ...args) =>
  execFileSync("pnpm", ["tauri", "icon", input, "-o", ...args], { stdio: "inherit", shell: true });

try {
  // The icon with the compact cut in place of the mark, in the same inset (the mark's group). Its
  // outer fruit overhang its viewBox, so that mustn't clip.
  const svg = readFileSync(source, "utf8");
  const inset = /<g transform="translate\(([\d.]+) ([\d.]+)\) scale\(([\d.]+)\)">[\s\S]*<\/g>/;
  const [, x, y, scale] = svg.match(inset) ?? [];
  if (!scale) throw new Error("orchard-icon.svg: the mark's inset group wasn't found");
  const mark = readFileSync(compactMark, "utf8");
  const viewBox = mark.match(/viewBox="([^"]*)"/)?.[1];
  const drawing = mark.match(/<svg[^>]*>([\s\S]*)<\/svg>/)?.[1];
  if (!viewBox || !drawing) throw new Error("orchard-mark-small.svg: no viewBox or drawing");
  const size = 320 * Number(scale);
  const compact = svg.replace(inset, `<svg x="${x}" y="${y}" width="${size}" height="${size}" viewBox="${viewBox}" overflow="visible">${drawing}</svg>`);
  const noHairline = compact.replace(/ stroke="[^"]*" stroke-width="[^"]*"/, "");
  if (noHairline === compact) throw new Error("orchard-icon.svg: the hairline's stroke attributes weren't found");
  const small = join(temp, "orchard-icon-32.svg");
  const smallest = join(temp, "orchard-icon-16.svg");
  writeFileSync(small, compact);
  writeFileSync(smallest, noHairline);

  // The .icns (and nothing else) from the default set.
  tauriIcon(source, join(temp, "default"));
  copyFileSync(join(temp, "default", "icon.icns"), join(icons, "icon.icns"));

  tauriIcon(source, join(temp, "png"), "-p", "1024,512,256,128,64,48");
  tauriIcon(small, join(temp, "png"), "-p", "32,24");
  tauriIcon(smallest, join(temp, "png"), "-p", "16");
  const png = (size) => join(temp, "png", `${size}x${size}.png`);
  for (const size of [1024, 512, 256, 128, 64, 32, 16]) copyFileSync(png(size), join(icons, `${size}x${size}.png`));

  // The .ico: PNG-encoded entries (Vista and later), the sizes as above.
  const entries = [16, 24, 32, 48, 64, 256].map((size) => ({ size, data: readFileSync(png(size)) }));
  const header = Buffer.alloc(6 + 16 * entries.length);
  header.writeUInt16LE(0, 0);
  header.writeUInt16LE(1, 2); // icon
  header.writeUInt16LE(entries.length, 4);
  let offset = header.length;
  entries.forEach(({ size, data }, i) => {
    const at = 6 + 16 * i;
    header.writeUInt8(size % 256, at); // (256 is written as 0)
    header.writeUInt8(size % 256, at + 1);
    header.writeUInt16LE(1, at + 4); // planes
    header.writeUInt16LE(32, at + 6); // bits per pixel
    header.writeUInt32LE(data.length, at + 8);
    header.writeUInt32LE(offset, at + 12);
    offset += data.length;
  });
  writeFileSync(join(icons, "icon.ico"), Buffer.concat([header, ...entries.map((e) => e.data)]));
} finally {
  rmSync(temp, { recursive: true, force: true });
}
