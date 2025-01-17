//! Utilities for expanding entity and character references.

use std::borrow::Cow;
use std::char;
use std::ops::Range;

use nom::branch::alt;
use nom::bytes::complete::{tag, take_while1};
use nom::character::complete::digit1;
use nom::combinator::{consumed, map, opt, recognize};
use nom::sequence::{preceded, terminated};
use nom::IResult;

use crate::parser::raw::{is_name_char, name};

/// The type returned by expansion operations.
pub type Result<T = ()> = std::result::Result<T, EntityError>;

/// The error type in the event an invalid entity or character reference is found.
///
/// That means the entity expansion closure was called, and it returned `None`.
/// When invoking [`expand_characters`], any entity reference is considered undefined.
#[derive(Clone, Debug, PartialEq, thiserror::Error)]
#[error("entity '{entity}' is not defined")]
pub struct EntityError {
    /// The name of the entity that was not found.
    pub entity: String,
    /// The slice range of the entity in the source string.
    pub position: Range<usize>,
}

/// Expands character references (`&#123;`) in the given text.
/// Any entity references are treated as errors.
///
/// # Example
///
/// ```rust
/// # use sgmlish::entities::expand_characters;
/// let expanded = expand_characters("&#60;hello&#44; world&#33;&#62;");
/// assert_eq!(expanded, Ok("<hello, world!>".into()));
/// ```
pub fn expand_characters(text: &str) -> Result<Cow<str>> {
    expand_entities(text, |_| None::<&str>)
}

/// Expands entity references (`&foo;`) in the text using the given closure as lookup.
///
/// Character references (`&#123;`) are also expanded, without going through the closure.
/// Function names (`&#SPACE;`) as well as invalid character references
/// (codes that go beyond Unicode, e.g. `&#1234567;`) are also passed to the closure.
///
/// If the closure returns `None`, the entity is considered invalid,
/// and the expansion fails.
///
/// # Example
///
/// ```rust
/// # use std::collections::HashMap;
/// # use sgmlish::entities::expand_entities;
/// let mut entities = HashMap::new();
/// entities.insert("eacute", "é");
///
/// let expanded = expand_entities("caf&eacute; &#9749;", |entity| entities.get(entity));
/// assert_eq!(expanded, Ok("café ☕".into()));
/// ```
pub fn expand_entities<F, T>(text: &str, f: F) -> Result<Cow<str>>
where
    F: FnMut(&str) -> Option<T>,
    T: AsRef<str>,
{
    expand_entities_with(text, "&", entity_or_char_ref, f)
}

/// Expands parameter entities (`%foo;`) in the text using the given closure as lookup.
/// Parameter referencies are only used in specific parts of DTDs;
/// for SGML document content, use [`expand_entities`] instead.
///
/// If the closure returns `None`, the parameter entity is considered invalid,
/// and the expansion fails.
///
/// # Example
///
/// ```rust
/// # use std::collections::HashMap;
/// # use sgmlish::entities::expand_parameter_entities;
/// let mut entities = HashMap::new();
/// entities.insert("HTML.Reserved", "IGNORE");
///
/// let expanded = expand_parameter_entities(" %HTML.Reserved; ", |entity| entities.get(entity));
/// assert_eq!(expanded, Ok(" IGNORE ".into()));
/// ```
pub fn expand_parameter_entities<F, T>(text: &str, f: F) -> Result<Cow<str>>
where
    F: FnMut(&str) -> Option<T>,
    T: AsRef<str>,
{
    expand_entities_with(text, "%", entity_ref, f)
}

fn expand_entities_with<'a, M, F, T>(
    text: &'a str,
    prefix: &str,
    matcher: M,
    mut f: F,
) -> Result<Cow<'a, str>>
where
    M: FnMut(&str) -> IResult<&str, EntityRef>,
    F: FnMut(&'a str) -> Option<T>,
    T: AsRef<str>,
{
    // Suffix the matcher with optional `;`
    let mut matcher = terminated(matcher, opt(tag(";")));

    let mut remainder = text;
    let mut out = String::new();

    while let Some(position) = remainder.find(prefix) {
        let (mid, candidate) = remainder.split_at(position);
        out.push_str(mid);
        match matcher(&candidate[prefix.len()..]) {
            Ok((after, EntityRef::Entity(name))) => {
                out.push_str(
                    f(name)
                        .ok_or_else(|| EntityError {
                            entity: name.to_owned(),
                            position: text.len() - candidate.len()..text.len() - after.len(),
                        })?
                        .as_ref(),
                );
                remainder = after;
            }
            Ok((after, EntityRef::Char(c))) => {
                out.push(c);
                remainder = after;
            }
            Err(_) => {
                out.push_str(prefix);
                remainder = &candidate[prefix.len()..];
            }
        }
    }

    if remainder.len() == text.len() {
        return Ok(text.into());
    }

    out.push_str(remainder);
    Ok(out.into())
}

fn entity_or_char_ref(input: &str) -> IResult<&str, EntityRef> {
    alt((char_ref, entity_ref))(input)
}

fn char_ref(input: &str) -> IResult<&str, EntityRef> {
    map(
        consumed(preceded(
            tag("#"),
            alt((
                map(digit1, |code: &str| code.parse().ok()),
                // Hex escape codes are actually only valid in XML, but welp
                preceded(
                    tag("x"),
                    map(take_while1(is_name_char), |code| {
                        u32::from_str_radix(code, 16).ok()
                    }),
                ),
            )),
        )),
        |(raw, code)| {
            code.and_then(char::from_u32)
                .map(EntityRef::Char)
                .unwrap_or_else(|| EntityRef::Entity(raw))
        },
    )(input)
}

fn entity_ref(input: &str) -> IResult<&str, EntityRef> {
    map(recognize(preceded(opt(tag("#")), name)), EntityRef::Entity)(input)
}

enum EntityRef<'a> {
    Entity(&'a str),
    Char(char),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_invalid_refs() {
        fn assert_noop(s: &str) {
            let result = expand_characters(s);
            assert_eq!(result, Ok(s.into()));
        }

        assert_noop("foo&");
        assert_noop("foo&&");
        assert_noop("foo&;bar");
        assert_noop("foo&&;bar");
        assert_noop("foo&#");
        assert_noop("foo&#;");
        assert_noop("foo&#;bar");
        assert_noop("foo&##bar");
    }

    #[test]
    fn test_invalid_character_ref() {
        let result = expand_characters("foo&#x110000;bar");
        assert_eq!(
            result,
            Err(EntityError {
                entity: "#x110000".to_owned(),
                position: 3..13,
            })
        );
    }

    #[test]
    fn test_expand_characters() {
        let result = expand_characters("f&#111o bar &#128523;");
        assert_eq!(result, Ok("foo bar \u{1f60b}".into()));
    }

    #[test]
    fn test_expand_characters_hex() {
        let result = expand_characters("fo&#x6f; bar &#xFeFf;");
        assert_eq!(result, Ok("foo bar \u{feff}".into()));
    }

    #[test]
    fn test_expand_characters_missing_semicolon() {
        let result = expand_characters("fo&#x6f bar &#xFeFf");
        assert_eq!(result, Ok("foo bar \u{feff}".into()));
    }

    #[test]
    fn test_expand_entities_noop() {
        let result = expand_entities("this string has no references", |_| -> Option<&str> {
            unreachable!()
        });
        assert!(matches!(result.unwrap(), Cow::Borrowed(_)));
    }

    #[test]
    fn test_expand_entities_lookup() {
        let result = expand_entities("test &foo;&bar.x; &baz&qu-ux\n", |key| match key {
            "foo" => Some("x"),
            "bar.x" => Some("y"),
            "baz" => Some("z"),
            "qu-ux" => Some("w"),
            x => panic!("unexpected reference: {:?}", x),
        });
        assert_eq!(result, Ok("test xy zw\n".into()));
    }

    #[test]
    fn test_expand_entities_delegates_invalid_char_refs_to_closure() {
        let result = expand_entities(
            "test &#12345678&#x12345678 ok &#x &#xhello; &#xdeal",
            |key| Some(format!("({})", key)),
        );
        assert_eq!(
            result,
            Ok("test (#12345678)(#x12345678) ok (#x) (#xhello) (#xdeal)".into())
        );
    }

    #[test]
    fn test_expand_entities_invalid_entity() {
        let result = expand_entities("test &foo;&bar;", |key| match key {
            "foo" => Some("x"),
            "bar" => None,
            x => panic!("unexpected reference: {:?}", x),
        });
        assert_eq!(
            result,
            Err(EntityError {
                entity: "bar".into(),
                position: 10..15,
            })
        );
    }

    #[test]
    fn test_expand_entities_invalid_function() {
        let mut called = false;
        let result = expand_entities("foo&#test;bar", |x| {
            called = true;
            assert_eq!(x, "#test");
            None::<&str>
        });
        assert!(called);
        assert_eq!(
            result,
            Err(EntityError {
                entity: "#test".into(),
                position: 3..10,
            })
        );
    }

    #[test]
    fn test_expand_parameter_entities() {
        let result = expand_parameter_entities("CDATA %bar.baz ", |name| {
            assert_eq!(name, "bar.baz");
            Some("IGNORE")
        });
        assert_eq!(result, Ok("CDATA IGNORE ".into()));
    }

    #[test]
    fn test_expand_parameter_entities_ignores_general_entities() {
        let result = expand_parameter_entities("foo &bar;", |_| -> Option<&str> { unreachable!() });
        assert_eq!(result, Ok("foo &bar;".into()));
    }

    #[test]
    fn test_expand_parameter_entities_does_not_work_on_character_references() {
        let result = expand_parameter_entities("foo %#32;", |_| None::<&str>);
        assert_eq!(result, Ok("foo %#32;".into()));
    }
}
