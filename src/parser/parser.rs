use std::borrow::Cow;
use std::fmt;

use nom::Finish;

use crate::marked_sections::MarkedSectionStatus;
use crate::parser::events;
use crate::{entities, is_sgml_whitespace, Data, SgmlFragment};

// Import used for documentation links
#[allow(unused_imports)]
use crate::SgmlEvent;

use super::ParseError;

/// The parser for SGML data.
///
/// The parser is only capable of working directly with strings,
/// meaning the content must be decoded beforehand. If you want to work with
/// data in character sets other than UTF-8, you may want to have a look at the
/// [`encoding_rs`] crate.
///
/// [`encoding_rs`]: https://docs.rs/encoding_rs/
#[derive(Debug)]
pub struct Parser {
    config: ParserConfig,
}

impl Parser {
    /// Creates a new parser with default settings.
    ///
    /// The default settings are:
    ///
    /// * Whitespace is automatically trimmed
    /// * Tag and attribute names are kept in original casing
    /// * Only `CDATA` and `RCDATA` marked sections are allowed;
    ///   `IGNORE` and `INCLUDE` blocks, for instance, are rejected,
    ///   as are parameter entities (`%example;`) in marked sections
    /// * Only character references (`&#33;`) are accepted; all entities (`&example;`)
    ///   are rejected
    /// * Markup declarations and processing instructions are preserved
    pub fn new() -> Self {
        Parser {
            config: Default::default(),
        }
    }

    /// Creates a new parser builder
    pub fn builder() -> ParserBuilder {
        ParserBuilder::new()
    }

    /// Parses the given input.
    pub fn parse<'a>(&self, input: &'a str) -> crate::Result<SgmlFragment<'a>> {
        Ok(self.parse_with_error_type(input)?)
    }

    /// Parses the given input, using a different error handler for parser errors.
    ///
    /// Different [`nom`] error handlers may be used to adjust between speed and
    /// level of detail in error messages.
    pub fn parse_with_error_type<'a, E>(
        &self,
        input: &'a str,
    ) -> Result<SgmlFragment<'a>, ParseError<&'a str, E>>
    where
        E: nom::error::ParseError<&'a str>
            + nom::error::ContextError<&'a str>
            + nom::error::FromExternalError<&'a str, crate::Error>
            + fmt::Display,
    {
        let (rest, events) = events::document_entity::<E>(input, &self.config)
            .finish()
            .map_err(|error| ParseError::from_nom(input, error))?;
        debug_assert!(rest.is_empty(), "document_entity should be all_consuming");

        let events = events.collect::<Vec<_>>();

        Ok(SgmlFragment::from(events))
    }
}

/// The configuration for a [`Parser`].
pub struct ParserConfig {
    /// When `true`, leading and trailing whitespace from [`Character`](SgmlEvent::Character) events will be trimmed.
    /// Defaults to `true`.
    pub trim_whitespace: bool,
    /// Defines how tag and attribute names should be handled.
    pub name_normalization: NameNormalization,
    pub marked_section_handling: MarkedSectionHandling,
    pub ignore_markup_declarations: bool,
    pub ignore_processing_instructions: bool,
    entity_fn: Option<EntityFn>,
    parameter_entity_fn: Option<EntityFn>,
}

type EntityFn = Box<dyn Fn(&str) -> Option<Cow<'static, str>>>;

/// How tag and attribute names should be handled.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NameNormalization {
    /// Keep tag and attribute names as-is.
    Unchanged,
    /// Normalize all tag and attribute names to lowercase.
    ToLowercase,
    /// Normalize all tag and attribute names to uppercase.
    ToUppercase,
}

impl Default for NameNormalization {
    fn default() -> Self {
        NameNormalization::Unchanged
    }
}

impl NameNormalization {
    pub fn normalize<'a>(&self, mut name: Cow<'a, str>) -> Cow<'a, str> {
        match self {
            NameNormalization::ToLowercase if name.chars().any(|c| c.is_ascii_uppercase()) => {
                name.to_mut().make_ascii_lowercase();
                name
            }
            NameNormalization::ToUppercase if name.chars().any(|c| c.is_ascii_lowercase()) => {
                name.to_mut().make_ascii_uppercase();
                name
            }
            _ => name,
        }
    }
}

/// How marked sections (`<![CDATA[example]]>`) should be handled.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MarkedSectionHandling {
    /// Keep all marked sections as [`MarkedSection`](SgmlEvent::MarkedSection)
    /// events in the stream.
    KeepUnmodified,
    /// Expand `CDATA` and `RCDATA` sections into [`Character`][SgmlEvent::Character] events,
    /// treat anything else as a parse error.
    AcceptOnlyCharacterData,
    /// Expand also `INCLUDE` and `IGNORE` sections.
    ExpandAll,
}

impl Default for MarkedSectionHandling {
    fn default() -> Self {
        MarkedSectionHandling::AcceptOnlyCharacterData
    }
}

impl MarkedSectionHandling {
    /// Parses the status keywords in the given string according to the chosen rules.
    ///
    /// Returns `None` if any of the keywords is rejected.
    pub fn parse_keywords(&self, status_keywords: &str) -> Option<MarkedSectionStatus> {
        match self {
            // In this mode, only one keyword is accepted; even combining
            // two otherwise acceptable keywords (e.g. `<![CDATA CDATA[`) is rejected
            MarkedSectionHandling::AcceptOnlyCharacterData => match status_keywords.parse() {
                Ok(status @ (MarkedSectionStatus::CData | MarkedSectionStatus::RcData)) => {
                    Some(status)
                }
                _ => None,
            },
            _ => MarkedSectionStatus::from_keywords(status_keywords).ok(),
        }
    }
}

impl ParserConfig {
    /// Trims the given text according to the configured rules.
    pub fn trim<'a>(&self, text: &'a str) -> &'a str {
        if self.trim_whitespace {
            text.trim_matches(is_sgml_whitespace)
        } else {
            text
        }
    }

    /// Parses the given replaceable character data, returning its final form.
    pub fn parse_rcdata<'a>(&self, rcdata: &'a str) -> crate::Result<Data<'a>> {
        let f = self.entity_fn.as_deref().unwrap_or(&|_| None);
        entities::expand_entities(rcdata, f)
            .map(Data::CData)
            .map_err(From::from)
    }

    /// Parses parameter entities in the given markup declaration text, returning its final form.
    pub fn parse_markup_declaration_text<'a>(
        &self,
        text: &'a str,
    ) -> entities::Result<Cow<'a, str>> {
        let f = self.parameter_entity_fn.as_deref().unwrap_or(&|_| None);
        entities::expand_parameter_entities(text, f).map_err(From::from)
    }
}

impl Default for ParserConfig {
    /// Creates a new, default `ParserConfig`. See [`Parser::new`] for the default settings.
    fn default() -> Self {
        ParserConfig {
            trim_whitespace: true,
            name_normalization: Default::default(),
            marked_section_handling: Default::default(),
            ignore_markup_declarations: false,
            ignore_processing_instructions: false,
            entity_fn: None,
            parameter_entity_fn: None,
        }
    }
}

impl fmt::Debug for ParserConfig {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_struct("ParserConfig")
            .field("trim_whitespace", &self.trim_whitespace)
            .field("process_marked_sections", &self.marked_section_handling)
            .field("expand_entity", &omit(&self.entity_fn))
            .field("expand_parameter_entity", &omit(&self.parameter_entity_fn))
            .finish()
    }
}

/// A fluent interface for configuring parsers.
#[derive(Default, Debug)]
pub struct ParserBuilder {
    config: ParserConfig,
}

/// A builder for parser configurations.
impl ParserBuilder {
    /// Creates a new builder.
    pub fn new() -> Self {
        Default::default()
    }

    /// Defines whether whitespace surrounding text should be trimmed.
    pub fn trim_whitespace(mut self, trim_whitespace: bool) -> Self {
        self.config.trim_whitespace = trim_whitespace;
        self
    }

    /// Defines how tag and attribute names should be normalized.
    pub fn name_normalization(mut self, name_normalization: NameNormalization) -> Self {
        self.config.name_normalization = name_normalization;
        self
    }

    /// Normalizes all tag and attribute names to lowercase.
    pub fn lowercase_names(self) -> Self {
        self.name_normalization(NameNormalization::ToLowercase)
    }

    /// Normalizes all tag and attribute names to lowercase.
    pub fn uppercase_names(self) -> Self {
        self.name_normalization(NameNormalization::ToUppercase)
    }

    /// Defines a closure to be used to resolve entities.
    pub fn expand_entities<F, T>(mut self, f: F) -> Self
    where
        F: Fn(&str) -> Option<T> + 'static,
        T: Into<Cow<'static, str>>,
    {
        self.config.entity_fn = Some(Box::new(move |entity| f(entity).map(Into::into)));
        self
    }

    /// Defines a closure to be used to resolve entities.
    pub fn expand_parameter_entities<F, T>(mut self, f: F) -> Self
    where
        F: Fn(&str) -> Option<T> + 'static,
        T: Into<Cow<'static, str>>,
    {
        self.config.parameter_entity_fn = Some(Box::new(move |entity| f(entity).map(Into::into)));
        self
    }

    /// Changes how marked sections should be handled.
    pub fn marked_section_handling(mut self, mode: MarkedSectionHandling) -> Self {
        self.config.marked_section_handling = mode;
        self
    }

    /// Enables support for all marked sections, including `<![INCLUDE[...]]>`
    /// and `<![IGNORE[...]]>`.
    ///
    /// By default, only `CDATA` and `RCDATA` marked sections are accepted.
    pub fn expand_marked_sections(self) -> Self {
        self.marked_section_handling(MarkedSectionHandling::ExpandAll)
    }

    /// Changes whether markup declarations (`<!EXAMPLE>`) should be ignored
    /// or present in the event stream.
    pub fn ignore_markup_declarations(mut self, ignore: bool) -> Self {
        self.config.ignore_markup_declarations = ignore;
        self
    }

    /// Changes whether processing instructions (`<?example>`) should be ignored
    /// or present in the event stream.
    pub fn ignore_processing_instructions(mut self, ignore: bool) -> Self {
        self.config.ignore_processing_instructions = ignore;
        self
    }

    /// Builds a new parser from the given configuration.
    pub fn build(self) -> Parser {
        Parser {
            config: self.config,
        }
    }

    /// Parses the given input with the built parser.
    ///
    /// To reuse the same parser for multiple inputs, use [`build()`](ParserBuilder::build)
    /// then [`Parser::parse()`].
    pub fn parse(self, input: &str) -> crate::Result<SgmlFragment> {
        self.build().parse(input)
    }

    /// Returns a [`ParserConfig`] with the configuration that was built using other methods.
    pub fn into_config(self) -> ParserConfig {
        self.config
    }
}

fn omit<T>(opt: &Option<T>) -> impl fmt::Debug {
    opt.as_ref().map(|_| Ellipsis)
}

struct Ellipsis;

impl fmt::Debug for Ellipsis {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("...")
    }
}