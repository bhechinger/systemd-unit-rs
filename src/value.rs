use once_cell::sync::Lazy;
use ordered_multimap::ListOrderedMultimap;
use std::str::FromStr;
use super::{parse_bool, quote_value, unquote_value};

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Entries {
    pub(crate) data: ListOrderedMultimap<EntryKey, EntryValue>,
}

impl Default for &Entries {
    fn default() -> Self {
        static EMPTY: Lazy<Entries> = Lazy::new(|| Entries::default());
        &EMPTY
    }
}

pub(crate) type EntryKey = String;

pub(crate) type EntryRawValue = String;

#[derive(Clone, Default, Debug, PartialEq)]
pub struct EntryValue {
    raw: EntryRawValue,
    unquoted: String,
}

impl EntryValue {
    pub fn from_unquoted<S: Into<String>>(unquoted: S) -> Self {
        let unquoted = unquoted.into();
        Self {
            raw: quote_value(unquoted.as_str()),
            unquoted,
        }
    }

    pub fn raw(&self) -> &String {
        &self.raw
    }

    pub fn unquote(&self) -> String {
        self.try_unquote().expect("parsing error")
    }

    #[deprecated = "use unquote() or try_unquote()"]
    pub fn unquoted(&self) -> &String {
        &self.unquoted
    }

    pub fn to_bool(&self) -> Result<bool, super::Error> {
        let trimmed = self.raw.trim();
        if trimmed.is_empty() {
            return Ok(false);
        }

        parse_bool(trimmed)
    }

    pub fn try_from_raw<S: Into<String>>(raw: S) -> Result<Self, super::Error> {
        let raw = raw.into();
        Ok(Self {
            unquoted: unquote_value(raw.as_str())?,
            raw,
        })
    }

    pub fn try_unquote(&self) -> Result<String, super::Error> {
        unquote_value(self.raw.as_str())
    }
}

/// experimental: not sure if this is the right way
impl From<&str> for EntryValue {
    fn from(unquoted: &str) -> Self {
        Self::from_unquoted(unquoted)
    }
}

/// experimental: not sure if this is the right way
impl From<String> for EntryValue {
    fn from(unquoted: String) -> Self {
        Self::from_unquoted(unquoted)
    }
}

/// experimental: not sure if this is the right way
impl FromStr for EntryValue {
    type Err = super::Error;

    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        Self::try_from_raw(raw)
    }
}

pub(crate) type SectionKey = String;
