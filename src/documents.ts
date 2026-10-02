// The files open in Manual editors in every window, as the core tracks them (ticket 16): kept live
// for whoever asks, so an Edit permission card can warn about a file with unsaved changes.
import { createSignal } from "solid-js";
import { core, type OpenDocument } from "./core";

const [documents, setDocuments] = createSignal<OpenDocument[]>([]);
let following = false;

function follow() {
  if (following) return;
  following = true;
  let heard = false;
  void core.onEvent((event) => {
    if (event.kind !== "documentsChanged") return;
    heard = true;
    setDocuments(event.documents);
  });
  // (Unless an event already brought a newer list.)
  void core
    .openDocuments()
    .then((docs) => heard || setDocuments(docs))
    .catch(() => {});
}

/** Whether a file under the folder `dir` has unsaved changes in some Manual editor. */
export function hasUnsavedChangesUnder(dir: string): boolean {
  follow();
  const prefix = dir.replace(/[\\/]+$/, "");
  return documents().some((d) => d.dirty && (d.path.startsWith(prefix + "\\") || d.path.startsWith(prefix + "/")));
}

/** Whether `path` (absolute, as the core gives paths) has unsaved changes in some Manual editor. */
export function hasUnsavedChanges(path: string): boolean {
  follow();
  return documents().some((d) => d.dirty && d.path === path);
}
