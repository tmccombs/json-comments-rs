//! `json_comments` is a library to strip out comments from JSON-like test. By processing text
//! through a [`StripComments`] adapter first, it is possible to use a standard JSON parser (such
//! as [serde_json](https://crates.io/crates/serde_json) with quasi-json input that contains
//! comments.
//!
//! In fact, this code makes few assumptions about the input and could probably be used to strip
//! comments out of other types of code as well, provided that strings use double quotes and
//! backslashes are used for escapes in strings.
//!
//! The following types of comments are supported:
//!   - C style block comments (`/* ... */`)
//!   - C style line comments (`// ...`)
//!   - Shell style line comments (`# ...`)
//!
//! ## Example using serde_json
//!
//! ```
//! use serde_json::{Result, Value};
//! use json_comments::StripComments;
//!
//! # fn main() -> Result<()> {
//! // Some JSON input data as a &str. Maybe this comes form the user.
//! let data = r#"
//!     {
//!         "name": /* full */ "John Doe",
//!         "age": 43,
//!         "phones": [
//!             "+44 1234567", // work phone
//!             "+44 2345678"  // home phone
//!         ]
//!     }"#;
//!
//! // Strip the comments from the input (use `as_bytes()` to get a `Read`).
//! let stripped = StripComments::new(data.as_bytes());
//! // Parse the string of data into serde_json::Value.
//! let v: Value = serde_json::from_reader(stripped)?;
//!
//! println!("Please call {} at the number {}", v["name"], v["phones"][0]);
//!
//! # Ok(())
//! # }
//! ```
//!
use std::io::{ErrorKind, Read, Result};

#[derive(Eq, PartialEq, Copy, Clone, Debug)]
enum State {
    Top,
    InString,
    StringEscape,
    InComment,
    InBlockComment,
    MaybeCommentEnd,
    InLineComment,
}

use State::*;

/// A [`Read`] that transforms another [`Read`] so that it changes all comments to spaces so that a downstream json parser
/// (such as json-serde) doesn't choke on them.
///
/// The supported comments are:
///   - C style block comments (`/* ... */`)
///   - C style line comments (`// ...`)
///   - Shell style line comments (`# ...`)
///
/// ## Example
/// ```
/// use json_comments::StripComments;
/// use std::io::Read;
///
/// let input = r#"{
/// // c line comment
/// "a": "comment in string /* a */",
/// ## shell line comment
/// } /** end */"#;
///
/// let mut stripped = String::new();
/// StripComments::new(input.as_bytes()).read_to_string(&mut stripped).unwrap();
///
/// assert_eq!(stripped, "{
///                  \n\"a\": \"comment in string /* a */\",
///                     \n}           ");
///
/// ```
///
pub struct StripComments<T: Read> {
    inner: T,
    state: State,
}

impl<T> StripComments<T>
where
    T: Read,
{
    pub fn new(input: T) -> Self {
        Self {
            inner: input,
            state: Top,
        }
    }
}

macro_rules! invalid_data {
    () => {
        return Err(ErrorKind::InvalidData.into());
    };
}

impl<T> Read for StripComments<T>
where
    T: Read,
{
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        let count = self.inner.read(buf)?;
        if count > 0 {
            for c in buf[..count].iter_mut() {
                self.state = match self.state {
                    Top => top(c),
                    InString => in_string(*c),
                    StringEscape => InString,
                    InComment => in_comment(c)?,
                    InBlockComment => in_block_comment(c),
                    MaybeCommentEnd => maybe_comment_end(c),
                    InLineComment => in_line_comment(c),
                }
            }
        } else if self.state != Top && self.state != InLineComment {
            invalid_data!();
        }
        Ok(count)
    }
}

fn top(c: &mut u8) -> State {
    match *c {
        b'"' => InString,
        b'/' => {
            *c = b' ';
            InComment
        }
        b'#' => {
            *c = b' ';
            InLineComment
        }
        _ => Top,
    }
}

fn in_string(c: u8) -> State {
    match c {
        b'"' => Top,
        b'\\' => StringEscape,
        _ => InString,
    }
}

fn in_comment(c: &mut u8) -> Result<State> {
    let new_state = match c {
        b'*' => InBlockComment,
        b'/' => InLineComment,
        _ => invalid_data!(),
    };
    *c = b' ';
    Ok(new_state)
}

fn in_block_comment(c: &mut u8) -> State {
    let old = *c;
    *c = b' ';
    if old == b'*' {
        MaybeCommentEnd
    } else {
        InBlockComment
    }
}

fn maybe_comment_end(c: &mut u8) -> State {
    if *c == b'/' {
        *c = b' ';
        Top
    } else {
        InBlockComment
    }
}

fn in_line_comment(c: &mut u8) -> State {
    if *c == b'\n' {
        Top
    } else {
        *c = b' ';
        InLineComment
    }
}

#[cfg(test)]
mod tests {
    use super::StripComments;
    use std::io::{ErrorKind, Read};

    fn strip_string(input: &str) -> String {
        let mut out = String::new();
        let count = StripComments::new(input.as_bytes())
            .read_to_string(&mut out)
            .unwrap();
        assert_eq!(count, input.len());
        out
    }

    #[test]
    fn block_comments() {
        let json = r#"{/* Comment */"hi": /** abc */ "bye"}"#;
        let stripped = strip_string(json);
        assert_eq!(stripped, r#"{             "hi":            "bye"}"#);
    }

    #[test]
    fn line_comments() {
        let json = r#"{
            // line comment
            "a": 4,
            # another
        }"#;

        let expected = "{
                           \n            \"a\": 4,
                     \n        }";

        assert_eq!(strip_string(json), expected);
    }

    #[test]
    fn incomplete_string() {
        let json = r#""foo"#;
        let mut stripped = String::new();

        let err = StripComments::new(json.as_bytes())
            .read_to_string(&mut stripped)
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidData);
    }

    #[test]
    fn incomplete_comment() {
        let json = r#"/* foo "#;
        let mut stripped = String::new();

        let err = StripComments::new(json.as_bytes())
            .read_to_string(&mut stripped)
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidData);
    }

    #[test]
    fn incomplete_comment2() {
        let json = r#"/* foo *"#;
        let mut stripped = String::new();

        let err = StripComments::new(json.as_bytes())
            .read_to_string(&mut stripped)
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidData);
    }
}
