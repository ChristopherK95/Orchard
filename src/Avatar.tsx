// A session's face: a 10×10 pixel character drawn from its name, so the same name always gets the
// same face (its hair, eyes, mouth, skin and extras), dressed in its Worktree's colour.
import { For } from "solid-js";

type Grid = string[];

/** The head and shoulders everyone starts from: s skin, c clothes. */
const BASE: Grid = [
  "..........",
  "..........",
  "..ssssss..",
  ".ssssssss.",
  ".ssssssss.",
  ".ssssssss.",
  ".ssssssss.",
  "..ssssss..",
  "..cccccc..",
  ".cccccccc.",
];

// Overlays, laid over the base where they aren't ".": h hair, a extra, e eye, w eye white, m mouth.
const HAIR: Grid[] = [
  [],
  ["..........", "..hhhhhh..", ".hhhhhhhh.", ".h......h."],
  [".h.h..h.h.", ".hhhhhhhh.", ".hhhhhhhh."],
  ["....hh....", "..hhhhhh..", ".hhhhhhhh."],
  ["....hh....", "....hh....", "...hhhh..."],
  ["..........", "..hhhhhh..", ".hhhhhhhh.", ".h......h.", ".h......h.", ".h......h.", ".h......h.", ".h......h."],
  ["..........", "..aaaaaa..", ".aaaaaaaaa", ".h......h."],
  [".a......a.", ".aa....aa.", ".hhhhhhhh."],
];
const EYES: Grid[] = [
  ["", "", "", "", "..we..we.."],
  ["", "", "", "", "..ew..ew.."],
  ["", "", "", "", "...e..e..."],
  ["", "", "", "", "..ee..ee.."],
  ["", "", "", "", ".aaaaaaaa.", "..aa..aa.."],
  ["", "", "", "", "..we..ew.."],
];
const MOUTHS: Grid[] = [
  ["", "", "", "", "", "", "....mm...."],
  ["", "", "", "", "", "", "...mmmm..."],
  ["", "", "", "", "", "", "...m..m...", "....mm...."],
  ["", "", "", "", "", "", "....mm....", "....mm...."],
  ["", "", "", "", "", "", "...mm....."],
];
const EXTRAS: Grid[] = [
  [],
  [],
  ["", "", "", "", "", "", ".h......h.", "..hhhhhh.."], // a beard (under the mouth's row)
  ["", "", "", "", "", ".b......b."], // blush
];

const SKIN = ["#f2c6a0", "#d9a066", "#b07a4f", "#8fd18f", "#9ab8f0", "#c79ae8", "#f08a8a", "#e8d36b", "#a3a3b5", "#7fd6c8"];
const HAIR_COLOUR = ["#2b2118", "#5a3a22", "#c9782c", "#e8c35a", "#e4e4e7", "#c43c3c", "#3c6fc4", "#7d3cc4", "#2f8f5a", "#18181b"];
const EXTRA_COLOUR = ["#27272a", "#c43c3c", "#3c6fc4", "#e8c35a", "#2f8f5a", "#e4e4e7"];

/** FNV-1a: a well-mixed 32-bit hash. */
function hash(text: string): number {
  let h = 0x811c9dc5;
  for (const c of text) h = Math.imul(h ^ c.charCodeAt(0), 0x01000193);
  return h >>> 0;
}

/** The face's pixels, row by row: a colour per pixel, or null where it's clear. */
function face(name: string, clothes: string): (string | null)[][] {
  let h = hash(name);
  /** The next pick from `n` choices. */
  const pick = (n: number) => {
    const i = h % n;
    h = Math.imul(h ^ (h >>> 15), 0x2c1b3c6d) >>> 0;
    return i;
  };
  const skin = SKIN[pick(SKIN.length)];
  const hair = HAIR_COLOUR[pick(HAIR_COLOUR.length)];
  const extra = EXTRA_COLOUR[pick(EXTRA_COLOUR.length)];
  const layers = [HAIR[pick(HAIR.length)], EYES[pick(EYES.length)], MOUTHS[pick(MOUTHS.length)], EXTRAS[pick(EXTRAS.length)]];
  const colour: Record<string, string> = { s: skin, c: clothes, h: hair, a: extra, e: "#14141c", w: "#fafafa", m: "#7a2e2e", b: "#f08a8a" };
  const grid = BASE.map((row) => [...row]);
  for (const layer of layers)
    layer.forEach((row, y) => [...row].forEach((p, x) => p !== "." && (grid[y][x] = p)));
  return grid.map((row) => row.map((p) => (p === "." ? null : colour[p])));
}

/** A face as one outline per colour (a few nodes rather than a rect per pixel: avatars are on every
 *  sidebar card, Board card and column tab). */
type Shape = { fill: string; d: string };

const drawn = new Map<string, Shape[]>();

/** `face`'s pixels as shapes, worked out once per name and colour. */
function shapes(name: string, clothes: string): Shape[] {
  const key = `${clothes} ${name}`;
  let known = drawn.get(key);
  if (!known) {
    const byColour = new Map<string, string>();
    face(name, clothes).forEach((row, y) =>
      row.forEach((fill, x) => fill && byColour.set(fill, `${byColour.get(fill) ?? ""}M${x} ${y}h1v1h-1z`)),
    );
    known = [...byColour].map(([fill, d]) => ({ fill, d }));
    if (drawn.size >= 500) drawn.clear(); // (names come and go; this keeps it small)
    drawn.set(key, known);
  }
  return known;
}

export function Avatar(props: { name: string; colour: string; size?: number }) {
  return (
    <svg class="avatar" viewBox="-1 -1 12 12" width={props.size ?? 32} height={props.size ?? 32} shape-rendering="crispEdges" aria-hidden="true">
      <For each={shapes(props.name, props.colour)}>{(shape) => <path d={shape.d} fill={shape.fill} />}</For>
    </svg>
  );
}
