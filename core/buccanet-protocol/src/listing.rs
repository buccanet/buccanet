//! The bounds of a public listing, shared by the relay that checks them and the clients that
//! build listings.

/// Longest slug, in characters.
pub const MAX_SLUG_CHARS: usize = 40;
/// Longest listing title, in characters.
pub const MAX_TITLE_CHARS: usize = 80;
/// Longest listing description, in characters.
pub const MAX_DESCRIPTION_CHARS: usize = 280;

/// A slug is 1 to 40 lowercase ASCII letters, digits and inner hyphens, so it reads the same
/// in a path segment as in a command.
pub fn valid_slug(slug: &str) -> bool {
    let shaped = !slug.is_empty() && slug.len() <= MAX_SLUG_CHARS && !slug.starts_with('-') && !slug.ends_with('-');
    shaped && slug.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// Why a listing's title or description is refused, if it is.
pub fn check_text(title: &str, description: &str) -> Result<(), String> {
    if title.trim().is_empty() || title.chars().count() > MAX_TITLE_CHARS {
        return Err(format!("A listing title has 1 to {MAX_TITLE_CHARS} characters."));
    }
    if description.chars().count() > MAX_DESCRIPTION_CHARS {
        return Err(format!("A listing description has at most {MAX_DESCRIPTION_CHARS} characters."));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs_are_lowercase_words_joined_by_hyphens() {
        assert!(valid_slug("demo") && valid_slug("public-room-2"));
        assert!(!valid_slug("") && !valid_slug("Demo") && !valid_slug("-demo") && !valid_slug("demo-"));
        assert!(!valid_slug("a b") && !valid_slug("a/b") && !valid_slug(&"a".repeat(41)));
    }

    #[test]
    fn titles_and_descriptions_are_bounded() {
        assert!(check_text("Public room", "").is_ok());
        assert!(check_text("  ", "").is_err());
        assert!(check_text(&"t".repeat(81), "").is_err());
        assert!(check_text("Room", &"d".repeat(281)).is_err());
    }
}
