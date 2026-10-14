// SPDX-License-Identifier: Apache-2.0

//! Splitting a line typed at the `baton listen` prompt into words.
//!
//! This is what a shell does for `baton send text "two words"` before the program sees it,
//! and the prompt has to do it for itself. It differs from a Unix shell in one way, on
//! purpose: a backslash is an ordinary character, so a Windows path such as `C:\Users\jay`
//! arrives as it was typed. Only inside double quotes does a backslash escape anything, and
//! then only a double quote or another backslash.

/// The words of `line`, or `None` if a quote is opened and never closed.
///
/// Words are separated by whitespace. `'...'` and `"..."` group what is inside them, spaces
/// included, and can sit in the middle of a word (`a"b c"d` is `ab cd`). `""` is an empty
/// word.
pub fn split(line: &str) -> Option<Vec<String>> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut in_word = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            c if c.is_whitespace() => {
                if in_word {
                    words.push(std::mem::take(&mut word));
                    in_word = false;
                }
            }
            '\'' => {
                in_word = true;
                loop {
                    match chars.next()? {
                        '\'' => break,
                        c => word.push(c),
                    }
                }
            }
            '"' => {
                in_word = true;
                loop {
                    match chars.next()? {
                        '"' => break,
                        '\\' if matches!(chars.peek(), Some('"' | '\\')) => {
                            word.push(chars.next()?);
                        }
                        c => word.push(c),
                    }
                }
            }
            c => {
                in_word = true;
                word.push(c);
            }
        }
    }
    if in_word {
        words.push(word);
    }
    Some(words)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(line: &str) -> Vec<String> {
        split(line).unwrap()
    }

    #[test]
    fn words_are_separated_by_any_whitespace() {
        assert_eq!(words("send  link\t x "), ["send", "link", "x"]);
        assert_eq!(words("   "), Vec::<String>::new());
        assert_eq!(words(""), Vec::<String>::new());
    }

    #[test]
    fn quotes_group_words() {
        assert_eq!(
            words(r#"send link https://x.org --title "An example""#),
            ["send", "link", "https://x.org", "--title", "An example"]
        );
        assert_eq!(
            words("send text 'it is  fine'"),
            ["send", "text", "it is  fine"]
        );
    }

    #[test]
    fn quoted_parts_join_the_word_they_touch() {
        assert_eq!(words(r#"a"b c"d"#), ["ab cd"]);
        assert_eq!(words(r#""a"'b'"#), ["ab"]);
    }

    #[test]
    fn empty_quotes_are_an_empty_word() {
        assert_eq!(words(r#"send text """#), ["send", "text", ""]);
        assert_eq!(words("send text ''"), ["send", "text", ""]);
    }

    #[test]
    fn a_backslash_is_an_ordinary_character() {
        assert_eq!(
            words(r"send text C:\Users\jay\notes.txt"),
            ["send", "text", r"C:\Users\jay\notes.txt"]
        );
        assert_eq!(words(r"a\ b"), [r"a\", "b"]);
        assert_eq!(words(r"'C:\Users'"), [r"C:\Users"]);
    }

    #[test]
    fn inside_double_quotes_a_backslash_escapes_only_a_quote_or_a_backslash() {
        assert_eq!(words(r#""say \"hi\"""#), [r#"say "hi""#]);
        assert_eq!(words(r#""a\\b""#), [r"a\b"]);
        assert_eq!(words(r#""C:\Users\jay""#), [r"C:\Users\jay"]);
    }

    #[test]
    fn a_quote_that_is_never_closed_is_an_error() {
        assert_eq!(split(r#"send text "oops"#), None);
        assert_eq!(split("send text 'oops"), None);
        assert_eq!(split(r#""ends with an escaped quote\""#), None);
    }
}
