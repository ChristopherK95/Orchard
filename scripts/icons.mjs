// Renders the app icon set from src-tauri/icons/orchard-icon.svg: the PNGs at 1024, 512, 256, 128,
// 64, 32 and 16 px, the Windows .ico and the macOS .icns. The 16 px raster drops the squircle's
// hairline (at that size it only muddies the edge); every other size is the SVG as drawn.
//
//   node scripts/icons.mjs
import { execFileSync } from "node:child_process";
import { copyFileSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const icons = join(import.meta.dirname, "..", "src-tauri", "icons");
const source = join(icons, "orchard-icon.svg");
const temp = mkdtempSync(join(tmpdir(), "orchard-icons-"));

const tauriIcon = (input, ...args) =>
  execFileSync("pnpm", ["tauri", "icon", input, "-o", ...args], { stdio: "inherit", shell: true });

try {
  const svg = readFileSync(source, "utf8");
  const noHairline = svg.replace(/ stroke="[^"]*" stroke-width="[^"]*"/, "");
  if (noHairline === svg) throw new Error("orchard-icon.svg: the hairline's stroke attributes weren't found");
  const small = join(temp, "orchard-icon-16.svg");
  writeFileSync(small, noHairline);

  // The .icns (and nothing else) from the default set.
  tauriIcon(source, join(temp, "default"));
  copyFileSync(join(temp, "default", "icon.icns"), join(icons, "icon.icns"));

  const sizes = [1024, 512, 256, 128, 64, 48, 32, 24];
  tauriIcon(source, join(temp, "png"), "-p", sizes.join(","));
  tauriIcon(small, join(temp, "png"), "-p", "16");
  const png = (size) => join(temp, "png", `${size}x${size}.png`);
  for (const size of [1024, 512, 256, 128, 64, 32, 16]) copyFileSync(png(size), join(icons, `${size}x${size}.png`));

  // The .ico: PNG-encoded entries (Vista and later), the 16 px one without the hairline.
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
