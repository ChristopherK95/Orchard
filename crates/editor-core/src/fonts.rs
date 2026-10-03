//! The fonts installed on this machine (ticket 32), for the settings page's font pickers.

use serde::Serialize;

/// One installed font family.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FontFamily {
    pub name: String,
    /// Its faces are fixed-width (a code font).
    pub monospace: bool,
}

/// Every installed family, by name (case-insensitively), each once. Reads the system's font
/// folders, so it's slow-ish (tens to hundreds of ms): call it off the UI thread, and once.
pub(crate) fn installed() -> Vec<FontFamily> {
    let mut db = fontdb::Database::new();
    db.load_system_fonts();
    let mut families: Vec<FontFamily> = vec![];
    for face in db.faces() {
        // (The first name is the English one, where there are several.)
        let Some((name, _)) = face.families.first() else {
            continue;
        };
        match families
            .iter_mut()
            .find(|f| f.name.eq_ignore_ascii_case(name))
        {
            Some(family) => family.monospace |= face.monospaced,
            None => families.push(FontFamily {
                name: name.clone(),
                monospace: face.monospaced,
            }),
        }
    }
    families.sort_by_key(|f| f.name.to_lowercase());
    families
}

#[cfg(test)]
mod tests {
    // (Needs a machine with fonts installed, as any desktop has.)
    #[test]
    fn installed_families_are_listed_once_each_with_some_monospace() {
        let families = super::installed();
        assert!(!families.is_empty());
        assert!(families.iter().any(|f| f.monospace));
        let mut names: Vec<_> = families.iter().map(|f| f.name.to_lowercase()).collect();
        names.dedup();
        assert_eq!(names.len(), families.len());
    }
}
