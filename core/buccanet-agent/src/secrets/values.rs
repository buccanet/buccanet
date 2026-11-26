//! Secrets known by their value, such as the ones an agent holds in its environment: a message
//! that holds one whole, or a long enough piece of a long one, holds the secret. The message is
//! read in its normalized form, so invisible characters cannot split a value.

use buccanet_sdk::invisible::normalized;

/// A long value is also caught when a piece of this length appears.
const PIECE_LENGTH: usize = 16;

/// Every run of `PIECE_LENGTH` characters inside a long value.
fn pieces(value: &str) -> impl Iterator<Item = &str> {
    value
        .char_indices()
        .map(|(start, _)| start)
        .filter(move |&start| value.len() >= start + PIECE_LENGTH && value.is_char_boundary(start + PIECE_LENGTH))
        .map(move |start| &value[start..start + PIECE_LENGTH])
}

/// Whether `text` holds one of `values`, whole or, for a long value, a long enough piece of it.
pub fn contains_any(text: &str, values: &[String]) -> bool {
    let text = normalized(text);
    values.iter().any(|value| {
        let long = value.len() >= PIECE_LENGTH + 4;
        text.contains(value.as_str()) || (long && pieces(value).any(|piece| text.contains(piece)))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catches_whole_values_and_pieces_of_long_ones() {
        let values = vec!["sk-live-0123456789abcdefghijklmnop".to_string()];
        assert!(contains_any("here: sk-live-0123456789abcdefghijklmnop", &values));
        assert!(contains_any("half of it 456789abcdefghijk is enough", &values));
        assert!(!contains_any("nothing secret here", &values));
        assert!(contains_any("split sk-live-0123\u{200B}456789abcdefghijklmnop", &values));
    }
}
