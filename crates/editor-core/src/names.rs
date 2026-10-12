//! The character names new Agent sessions get ("Bramble", "Clementine", …): one per session in the
//! Workspace, in a fixed order, then the same again as "Bramble II", "Bramble III", and so on.
//! The sidebar draws each one's avatar from its name.

const ROSTER: [&str; 40] = [
    "Bramble",
    "Clementine",
    "Juniper",
    "Pippin",
    "Hazel",
    "Quincy",
    "Rowan",
    "Marigold",
    "Basil",
    "Olive",
    "Russet",
    "Sorrel",
    "Fennel",
    "Damson",
    "Thistle",
    "Willow",
    "Cobnut",
    "Medlar",
    "Wren",
    "Nutmeg",
    "Barley",
    "Saffron",
    "Tansy",
    "Bartlett",
    "Greengage",
    "Linden",
    "Moss",
    "Pumpernickel",
    "Rhubarb",
    "Sage",
    "Tamarind",
    "Yuzu",
    "Bosc",
    "Elder",
    "Figaro",
    "Gooseberry",
    "Honeycrisp",
    "Kumquat",
    "Loquat",
    "Mulberry",
];

/// The `n`th name (from 1).
pub(crate) fn character_name(n: u64) -> String {
    let i = n.saturating_sub(1);
    let base = ROSTER[(i % ROSTER.len() as u64) as usize];
    match i / ROSTER.len() as u64 {
        0 => base.to_owned(),
        round => format!("{base} {}", roman(round + 1)),
    }
}

/// Which `n` `character_name` gave `name`, if it's one of them.
pub(crate) fn character_number(name: &str) -> Option<u64> {
    let (base, round) = match name.rsplit_once(' ') {
        Some((base, numeral)) => (base, (2..=1000).find(|&r| roman(r) == numeral)? - 1),
        None => (name, 0),
    };
    let at = ROSTER.iter().position(|&r| r == base)? as u64;
    Some(round * ROSTER.len() as u64 + at + 1)
}

fn roman(mut n: u64) -> String {
    const NUMERALS: [(u64, &str); 13] = [
        (1000, "M"),
        (900, "CM"),
        (500, "D"),
        (400, "CD"),
        (100, "C"),
        (90, "XC"),
        (50, "L"),
        (40, "XL"),
        (10, "X"),
        (9, "IX"),
        (5, "V"),
        (4, "IV"),
        (1, "I"),
    ];
    let mut out = String::new();
    for (value, numeral) in NUMERALS {
        while n >= value {
            out.push_str(numeral);
            n -= value;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_go_round_the_roster_then_number_the_rounds() {
        assert_eq!(character_name(1), "Bramble");
        assert_eq!(character_name(2), "Clementine");
        assert_eq!(character_name(40), "Mulberry");
        assert_eq!(character_name(41), "Bramble II");
        assert_eq!(character_name(82), "Clementine III");
    }

    #[test]
    fn a_name_gives_back_its_number() {
        for n in [1, 2, 40, 41, 83, 400] {
            assert_eq!(character_number(&character_name(n)), Some(n));
        }
        assert_eq!(character_number("Session 3"), None);
        assert_eq!(character_number("Bramble 2"), None);
        assert_eq!(character_number("Fix the login bug"), None);
    }
}
