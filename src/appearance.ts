// The Appearance settings (ticket 32) as CSS variables on the page's root: the fonts (each in front
// of the bundled one, which still stands in for a font that isn't installed), the Manual editor's
// font size, and ligatures. styles.css reads them; unset, its defaults apply.
import type { Appearance } from "./core";

/** The editor's code font sizes, as the core allows them. */
export const CODE_FONT_SIZES = { min: 9, max: 28, default: 13 };

const quoted = (family: string) => `"${family.replace(/["\\]/g, "\\$&")}"`;

export function applyAppearance(appearance: Appearance | undefined) {
  const root = document.documentElement.style;
  const font = (name: string, family: string | null | undefined) => {
    if (family?.trim()) root.setProperty(name, quoted(family.trim()));
    else root.removeProperty(name);
  };
  font("--user-ui", appearance?.uiFont);
  font("--user-mono", appearance?.codeFont);
  const size = appearance?.codeFontSize ?? CODE_FONT_SIZES.default;
  root.setProperty("--code-size", `${Math.min(CODE_FONT_SIZES.max, Math.max(CODE_FONT_SIZES.min, size))}px`);
  root.setProperty("--code-ligatures", appearance?.ligatures === false ? "none" : "normal");
}
