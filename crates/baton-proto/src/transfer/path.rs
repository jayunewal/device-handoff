// SPDX-License-Identifier: Apache-2.0

//! The paths in a transfer.
//!
//! A path names a file or a folder inside what the person chose to send. It is relative, its
//! parts are separated by `/` on every platform, and it is untrusted text: it comes from another
//! device and ends up as a name on this one's disk.
//!
//! This module holds the rules that are the same everywhere, which a receiver applies before it
//! looks at a path in any other way. Whatever a particular file system does not allow (control
//! characters, the reserved names of Windows, trailing dots and spaces, `\` and `:`, names that
//! differ only in case) is the receiver's to put right when it chooses the name to create. It is
//! not a reason to refuse the transfer, and it is not checked here.

use std::cmp::Ordering;

use crate::limits::{LimitError, MAX_FILE_NAME_BYTES, MAX_PATH_BYTES, MAX_PATH_DEPTH};

/// Why a path or a name is not accepted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PathError {
    /// There is nothing in it.
    #[error("it is empty")]
    Empty,
    /// The whole path is longer than allowed.
    #[error("it is {len} bytes, more than the limit of {max}")]
    TooLong {
        /// Length of the path in bytes.
        len: usize,
        /// Largest allowed length in bytes.
        max: usize,
    },
    /// It has more parts than allowed.
    #[error("it has {depth} parts, more than the limit of {max}")]
    TooDeep {
        /// Number of parts.
        depth: usize,
        /// Largest allowed number of parts.
        max: usize,
    },
    /// One name in it is longer than allowed.
    #[error("a name in it is {len} bytes, more than the limit of {max}")]
    NameTooLong {
        /// Length of the name in bytes.
        len: usize,
        /// Largest allowed length in bytes.
        max: usize,
    },
    /// A name in it is empty: the path starts or ends with `/`, or has `//` in it.
    #[error("it has an empty name")]
    EmptyName,
    /// A name in it is `.` or `..`.
    #[error("it has a name that is . or ..")]
    DotName,
    /// A name in it has a NUL character, which no file system allows.
    #[error("it has a NUL character")]
    Nul,
    /// A single name was expected and this is a path of several.
    #[error("it is a path of several names, not one name")]
    NotOneName,
}

impl PathError {
    /// The same fault as a [`LimitError`] about the field called `field`.
    pub(crate) fn into_limit(self, field: &'static str) -> LimitError {
        match self {
            Self::Empty => LimitError::Empty { field },
            Self::TooLong { len, max } | Self::NameTooLong { len, max } => {
                LimitError::TooLong { field, len, max }
            }
            Self::TooDeep { depth, max } => LimitError::TooMany {
                field,
                count: depth,
                max,
            },
            Self::EmptyName => LimitError::Invalid {
                field,
                reason: "has an empty name",
            },
            Self::DotName => LimitError::Invalid {
                field,
                reason: "has a name that is . or ..",
            },
            Self::Nul => LimitError::Invalid {
                field,
                reason: "has a NUL character",
            },
            Self::NotOneName => LimitError::Invalid {
                field,
                reason: "is a path, not a name",
            },
        }
    }
}

/// Checks a relative path and returns the number of names in it.
///
/// A path is valid if it is not empty, is at most [`MAX_PATH_BYTES`] long and has at most
/// [`MAX_PATH_DEPTH`] names, and each name is at most [`MAX_FILE_NAME_BYTES`] long, is not empty,
/// is not `.` or `..`, and has no NUL character. A path that is absolute starts with `/`, so it
/// has an empty first name and is not valid.
pub fn check_path(path: &str) -> Result<usize, PathError> {
    if path.is_empty() {
        return Err(PathError::Empty);
    }
    if path.len() > MAX_PATH_BYTES {
        return Err(PathError::TooLong {
            len: path.len(),
            max: MAX_PATH_BYTES,
        });
    }
    let mut depth = 0_usize;
    for name in path.split('/') {
        match name {
            "" => return Err(PathError::EmptyName),
            "." | ".." => return Err(PathError::DotName),
            _ => {}
        }
        if name.len() > MAX_FILE_NAME_BYTES {
            return Err(PathError::NameTooLong {
                len: name.len(),
                max: MAX_FILE_NAME_BYTES,
            });
        }
        if name.contains('\0') {
            return Err(PathError::Nul);
        }
        depth += 1;
    }
    if depth > MAX_PATH_DEPTH {
        return Err(PathError::TooDeep {
            depth,
            max: MAX_PATH_DEPTH,
        });
    }
    Ok(depth)
}

/// Checks that `name` is one valid name and not a path of several.
pub fn check_name(name: &str) -> Result<(), PathError> {
    if check_path(name)? == 1 {
        Ok(())
    } else {
        Err(PathError::NotOneName)
    }
}

/// Splits a path into the folder it is in (empty for a top-level item) and its own name.
///
/// The path is expected to have passed [`check_path`]; for one that did not, the result is
/// still well defined but means nothing.
pub fn split_name(path: &str) -> (&str, &str) {
    path.rsplit_once('/').unwrap_or(("", path))
}

/// The order in which paths appear in a list: name by name, each name by its bytes.
///
/// A folder comes before what is inside it, and what is inside a folder comes before the next
/// item of the folder around it. Comparing the whole text instead would put `a-b` between `a`
/// and `a/b`, because `-` sorts before `/`, and the contents of `a` would no longer follow it
/// directly.
pub fn compare(a: &str, b: &str) -> Ordering {
    a.split('/').cmp(b.split('/'))
}

#[cfg(test)]
mod tests {
    use super::*;

    // The longest path below is built for these two limits.
    const _: () = assert!(MAX_FILE_NAME_BYTES == 255 && MAX_PATH_BYTES == 1024);

    #[test]
    fn ordinary_paths_are_accepted_with_their_depth() {
        for (path, depth) in [
            ("a", 1),
            ("report.pdf", 1),
            ("a/b", 2),
            ("Holiday 2026/IMG 0001.jpg", 2),
            ("é/ü/日本語/ファイル.txt", 4),
            ("a/.hidden/..dots", 3),
            ("a/b/.../c", 4),
            // Characters that some systems do not allow are fixed by the receiver.
            ("a\\b", 1),
            ("C:/x", 2),
            ("con", 1),
            ("trailing./space ", 2),
            ("line\nbreak", 1),
        ] {
            assert_eq!(check_path(path), Ok(depth), "{path:?}");
        }
    }

    #[test]
    fn paths_that_could_leave_the_folder_or_cannot_exist_are_refused() {
        for (path, error) in [
            ("", PathError::Empty),
            ("/", PathError::EmptyName),
            ("/etc/passwd", PathError::EmptyName),
            ("a/", PathError::EmptyName),
            ("a//b", PathError::EmptyName),
            (".", PathError::DotName),
            ("..", PathError::DotName),
            ("../a", PathError::DotName),
            ("a/..", PathError::DotName),
            ("a/../b", PathError::DotName),
            ("a/./b", PathError::DotName),
            ("a\0b", PathError::Nul),
            ("a/b\0", PathError::Nul),
        ] {
            assert_eq!(check_path(path), Err(error), "{path:?}");
        }
    }

    #[test]
    fn the_limits_on_names_paths_and_depth_are_in_bytes_and_exact() {
        assert_eq!(check_path(&"n".repeat(MAX_FILE_NAME_BYTES)), Ok(1));
        assert_eq!(
            check_path(&"n".repeat(MAX_FILE_NAME_BYTES + 1)),
            Err(PathError::NameTooLong {
                len: MAX_FILE_NAME_BYTES + 1,
                max: MAX_FILE_NAME_BYTES
            })
        );
        // Two bytes a character: 127 of them fit in 255 bytes and 128 do not.
        assert_eq!(check_path(&"é".repeat(127)), Ok(1));
        assert!(matches!(
            check_path(&"é".repeat(128)),
            Err(PathError::NameTooLong { len: 256, .. })
        ));

        let deepest = vec!["d"; MAX_PATH_DEPTH].join("/");
        assert_eq!(check_path(&deepest), Ok(MAX_PATH_DEPTH));
        let too_deep = vec!["d"; MAX_PATH_DEPTH + 1].join("/");
        assert_eq!(
            check_path(&too_deep),
            Err(PathError::TooDeep {
                depth: MAX_PATH_DEPTH + 1,
                max: MAX_PATH_DEPTH
            })
        );

        // The longest path is made of names that are themselves allowed: three of 255 bytes,
        // one of 254 and one of 1 byte, with four `/` between them, add up to 1024.
        let names = [255, 255, 255, 254, 1].map(|len| "n".repeat(len));
        let longest = names.join("/");
        assert_eq!(longest.len(), MAX_PATH_BYTES);
        assert_eq!(check_path(&longest), Ok(5));
        assert_eq!(
            check_path(&format!("{longest}x")),
            Err(PathError::TooLong {
                len: MAX_PATH_BYTES + 1,
                max: MAX_PATH_BYTES
            })
        );
    }

    #[test]
    fn a_name_is_one_part() {
        assert_eq!(check_name("notes.txt"), Ok(()));
        assert_eq!(check_name("a/b"), Err(PathError::NotOneName));
        assert_eq!(check_name(""), Err(PathError::Empty));
        assert_eq!(check_name(".."), Err(PathError::DotName));
        assert_eq!(check_name("/a"), Err(PathError::EmptyName));
    }

    #[test]
    fn a_path_splits_into_its_folder_and_its_name() {
        assert_eq!(split_name("a"), ("", "a"));
        assert_eq!(split_name("a/b"), ("a", "b"));
        assert_eq!(split_name("a/b/c.txt"), ("a/b", "c.txt"));
    }

    #[test]
    fn paths_are_ordered_name_by_name() {
        use Ordering::{Equal, Greater, Less};
        for (a, b, expected) in [
            ("a", "a", Equal),
            ("a", "b", Less),
            ("b", "a", Greater),
            // A folder comes before what is inside it.
            ("a", "a/b", Less),
            // What is inside a folder comes before the next item next to the folder, even
            // where the whole text would sort the other way round (`-` is before `/`).
            ("a/b", "a-b", Less),
            ("a-b", "a/b", Greater),
            ("a/z", "b", Less),
            ("a/b/c", "a/c", Less),
            // Bytes, not letters: capitals come first, and so do shorter encodings.
            ("B", "a", Less),
            ("z", "é", Less),
            ("e", "é", Less),
        ] {
            assert_eq!(compare(a, b), expected, "{a:?} against {b:?}");
        }
    }
}
