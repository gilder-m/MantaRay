//! The chemical elements, by symbol and atomic number.
//!
//! Nothing here is an evaluation - these are the IUPAC symbols, and they are
//! the same in every reference. They are here so that a name typed by an
//! operator can be checked before it is looked up: `Xy-137` is a typo, not a
//! nuclide, and saying so is more use than an empty result. It also settles a
//! genuine ambiguity in how names are written, since `22mg` is magnesium-22
//! and `137mBa` is a metastable barium.

/// Symbols in order of atomic number, hydrogen first.
const SYMBOLS: [&str; 118] = [
    "H", "He", "Li", "Be", "B", "C", "N", "O", "F", "Ne", //   1 - 10
    "Na", "Mg", "Al", "Si", "P", "S", "Cl", "Ar", "K", "Ca", //  11 - 20
    "Sc", "Ti", "V", "Cr", "Mn", "Fe", "Co", "Ni", "Cu", "Zn", //  21 - 30
    "Ga", "Ge", "As", "Se", "Br", "Kr", "Rb", "Sr", "Y", "Zr", //  31 - 40
    "Nb", "Mo", "Tc", "Ru", "Rh", "Pd", "Ag", "Cd", "In", "Sn", //  41 - 50
    "Sb", "Te", "I", "Xe", "Cs", "Ba", "La", "Ce", "Pr", "Nd", //  51 - 60
    "Pm", "Sm", "Eu", "Gd", "Tb", "Dy", "Ho", "Er", "Tm", "Yb", //  61 - 70
    "Lu", "Hf", "Ta", "W", "Re", "Os", "Ir", "Pt", "Au", "Hg", //  71 - 80
    "Tl", "Pb", "Bi", "Po", "At", "Rn", "Fr", "Ra", "Ac", "Th", //  81 - 90
    "Pa", "U", "Np", "Pu", "Am", "Cm", "Bk", "Cf", "Es", "Fm", //  91 - 100
    "Md", "No", "Lr", "Rf", "Db", "Sg", "Bh", "Hs", "Mt", "Ds", // 101 - 110
    "Rg", "Cn", "Nh", "Fl", "Mc", "Lv", "Ts", "Og", //             111 - 118
];

/// The canonical spelling of a symbol, however it was typed.
///
/// `cs`, `CS` and `Cs` all give `Cs`. Anything that is not a symbol gives
/// `None`, which is how a typo is told from a nuclide this library happens not
/// to hold.
pub fn normalise(symbol: &str) -> Option<&'static str> {
    let symbol = symbol.trim();
    SYMBOLS
        .iter()
        .copied()
        .find(|known| known.eq_ignore_ascii_case(symbol))
}

/// The atomic number of a symbol, however it was typed.
pub fn atomic_number(symbol: &str) -> Option<u32> {
    let symbol = symbol.trim();
    SYMBOLS
        .iter()
        .position(|known| known.eq_ignore_ascii_case(symbol))
        .map(|index| index as u32 + 1)
}

/// The symbol of an atomic number, hydrogen being 1.
pub fn symbol(atomic_number: u32) -> Option<&'static str> {
    SYMBOLS.get(atomic_number.checked_sub(1)? as usize).copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_table_runs_from_hydrogen_to_oganesson() {
        assert_eq!(symbol(1), Some("H"));
        assert_eq!(symbol(118), Some("Og"));
        assert_eq!(symbol(0), None);
        assert_eq!(symbol(119), None);
        // The ones a gamma spectroscopist reaches for, at their right numbers.
        assert_eq!(atomic_number("Cs"), Some(55));
        assert_eq!(atomic_number("Co"), Some(27));
        assert_eq!(atomic_number("U"), Some(92));
        assert_eq!(atomic_number("Am"), Some(95));
    }

    #[test]
    fn a_symbol_is_recognised_however_it_is_typed() {
        assert_eq!(normalise("cs"), Some("Cs"));
        assert_eq!(normalise("CS"), Some("Cs"));
        assert_eq!(normalise(" Cs "), Some("Cs"));
        assert_eq!(normalise("Xy"), None);
        assert_eq!(normalise("M"), None);
        assert_eq!(normalise(""), None);
    }

    #[test]
    fn every_symbol_is_distinct_and_well_formed() {
        for (index, symbol) in SYMBOLS.iter().enumerate() {
            assert!(
                (1..=2).contains(&symbol.len()),
                "{symbol} is not one or two letters"
            );
            assert_eq!(atomic_number(symbol), Some(index as u32 + 1), "{symbol}");
        }
        let mut sorted: Vec<&str> = SYMBOLS.to_vec();
        sorted.sort_unstable();
        let before = sorted.len();
        sorted.dedup();
        assert_eq!(sorted.len(), before, "a symbol appears twice");
    }
}
