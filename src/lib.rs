use std::fmt;
use std::io::{self, ErrorKind, Read, Result};

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

/// Errors specific to removing comments.
///
/// Note that due to the signature of [`Read::read`], this will actually be wrapped inside a
/// [`std::io::Error`].
#[derive(Debug, Eq, PartialEq)]
pub enum Error {
    /// Error if the stream ends with a string that was never terminated.
    IncompleteString,
    /// Error if the stream ends with a block comment that was never terminated.
    IncompleteComment,
    /// Error if there is a forward slash at the top level that isn't immediately followed by a "*"
    /// or "/" to start a comment.
    UnexpectedForwardSlash,
}

impl fmt::Display for Error {
    fn fmt(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
        write!(fmt, "Invalid state for json with comments ({:?})", self)
    }
}

impl std::error::Error for Error {}

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

impl<T> Read for StripComments<T>
where
    T: Read,
{
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        let count = self.inner.read(buf)?;
        if count > 0 {
            let mut iter = buf[..count].iter_mut();
            while let Some(c) = iter.next() {
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
        } else {
            match self.state {
                InString | StringEscape => return Err(data_error(Error::IncompleteString)),
                InComment | InBlockComment | MaybeCommentEnd => {
                    return Err(data_error(Error::IncompleteComment))
                }
                _ => {}
            }
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
        _ => return Err(data_error(Error::UnexpectedForwardSlash)),
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

fn data_error(err: Error) -> io::Error {
    io::Error::new(ErrorKind::InvalidData, err)
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
