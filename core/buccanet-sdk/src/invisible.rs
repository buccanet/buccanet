//! Characters a reader cannot see: the default ignorable code points (zero width spaces and
//! joiners, the soft hyphen, the word joiner, variation selectors, the byte order mark), the
//! format characters and the line and paragraph separators. A name or a key can hide them
//! between its letters, so checks read text without them.

use unicode_normalization::UnicodeNormalization;
use unicode_properties::{GeneralCategory, UnicodeGeneralCategory};

/// The ranges of the Default_Ignorable_Code_Point property (Unicode DerivedCoreProperties.txt).
const DEFAULT_IGNORABLE: [(char, char); 17] = [
    ('\u{00AD}', '\u{00AD}'),
    ('\u{034F}', '\u{034F}'),
    ('\u{061C}', '\u{061C}'),
    ('\u{115F}', '\u{1160}'),
    ('\u{17B4}', '\u{17B5}'),
    ('\u{180B}', '\u{180F}'),
    ('\u{200B}', '\u{200F}'),
    ('\u{202A}', '\u{202E}'),
    ('\u{2060}', '\u{206F}'),
    ('\u{3164}', '\u{3164}'),
    ('\u{FE00}', '\u{FE0F}'),
    ('\u{FEFF}', '\u{FEFF}'),
    ('\u{FFA0}', '\u{FFA0}'),
    ('\u{FFF0}', '\u{FFF8}'),
    ('\u{1BCA0}', '\u{1BCA3}'),
    ('\u{1D173}', '\u{1D17A}'),
    ('\u{E0000}', '\u{E0FFF}'),
];

/// Whether `c` shows nothing: default ignorable, a format character or a line or paragraph
/// separator.
pub fn is_invisible(c: char) -> bool {
    let category =
        matches!(c.general_category(), GeneralCategory::Format | GeneralCategory::LineSeparator | GeneralCategory::ParagraphSeparator);
    category || DEFAULT_IGNORABLE.iter().any(|&(first, last)| (first..=last).contains(&c))
}

/// `text` in NFKC with every invisible character taken out, before and after the normalization:
/// the form that secret checks and name checks read, so `ｋｅｙ` and `k\u{200B}ey` read `key`.
pub fn normalized(text: &str) -> String {
    let visible: String = text.chars().filter(|c| !is_invisible(*c)).collect();
    visible.nfkc().filter(|c| !is_invisible(*c)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hidden_and_compatibility_characters_fold_away() {
        assert_eq!(normalized("A\u{200B}d\u{200D}m\u{2060}i\u{FEFF}n\u{00AD}\u{2028}"), "Admin");
        assert_eq!(normalized("ｓｔａｆｆ"), "staff");
        assert_eq!(normalized("Zoë Ada"), "Zoë Ada");
        assert!(is_invisible('\u{2029}') && is_invisible('\u{E0041}') && !is_invisible(' '));
    }
}
