mod constants;
mod parser;
mod quoted;
mod split;
mod value;

pub use self::constants::*;
pub use self::quoted::*;
pub use self::split::*;
pub(crate) use self::value::*;

use nix::unistd::{Gid, Uid, User, Group};
use std::fmt;
use std::io;
use std::path::PathBuf;
use ordered_multimap::list_ordered_multimap::ListOrderedMultimap;

// TODO: mimic https://doc.rust-lang.org/std/num/enum.IntErrorKind.html
// TODO: use thiserror?
#[derive(Debug, PartialEq)]
#[non_exhaustive]
pub enum Error {
    ParseBool,
    Unquoting(String),
    Unit(parser::ParseError),
    Gid(nix::errno::Errno),
    Uid(nix::errno::Errno),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::ParseBool => {
                write!(f, "value must be one of `1`, `yes`, `true`, `on`, `0`, `no`, `false`, `off`")
            },
            Error::Unquoting(msg) => {
                write!(f, "failed unquoting value: {msg}")
            },
            Error::Unit(e) => {
                write!(f, "failed to parse unit file: {e}")
            },
            Error::Gid(e) => {
                write!(f, "failed to parse group name/id: {e}")
            },
            Error::Uid(e) => {
                write!(f, "failed to parse user name/id: {e}")
            },
        }
    }
}

impl From<parser::ParseError> for Error {
    fn from(e: parser::ParseError) -> Self {
        Error::Unit(e)
    }
}

pub(crate) fn parse_bool(s: &str) -> Result<bool, Error> {
    if ["1", "yes", "true", "on"].contains(&s) {
        return Ok(true);
    } else if ["0", "no", "false", "off"].contains(&s) {
        return Ok(false)
    }

    Err(Error::ParseBool)
}

pub(crate) fn parse_gid(s: &str) -> Result<Gid, Error> {
    match s.parse::<u32>() {
        Ok(uid) => return Ok(Gid::from_raw(uid)),
        Err(_) => (),
    }

    match Group::from_name(s) {
        Ok(g) => Ok(g.unwrap().gid),
        Err(e) => Err(Error::Gid(e)),
    }
}

pub(crate) fn parse_uid(s: &str) -> Result<Uid, Error> {
    match s.parse::<u32>() {
        Ok(uid) => return Ok(Uid::from_raw(uid)),
        Err(_) => (),
    }

    match User::from_name(s) {
        Ok(u) => Ok(u.unwrap().uid),
        Err(e) => Err(Error::Uid(e)),
    }
}

#[derive(Debug, PartialEq)]
pub(crate) struct SystemdUnit {
    pub(crate) path: Option<PathBuf>,
    sections: ListOrderedMultimap<SectionKey, Entries>,
}

impl SystemdUnit {
    /// Appends `key=value` to last instance of `section`
    pub(crate) fn append_entry<S, K, V>(&mut self, section: S, key: K, value: V)
    where S: Into<String>,
          K: Into<String>,
          V: Into<String>,
    {
        self.append_entry_value(
            section,
            key,
            EntryValue::from_unquoted(value),
        );
    }
    /// Appends `key=value` to last instance of `section`
    pub(crate) fn append_entry_value<S, K>(&mut self, section: S, key: K, value: EntryValue)
    where S: Into<String>,
          K: Into<String>,
    {
        self.sections
            .entry(section.into())
            .or_insert_entry(Entries::default())
            .into_mut()
            .data.append(key.into(), value);
    }

    pub(crate) fn has_key<S, K>(&self, section: S, key: K) -> bool
    where S: Into<String>,
          K: Into<String>,
    {
        self.sections
            .get(&section.into())
            .map_or(false, |e| e.data.contains_key(&key.into()))
    }

    /// Retrun `true` if there's an (non-empty) instance of section `name`
    pub(crate) fn has_section<S: Into<String>>(&self, name: S) -> bool {
        self.sections.contains_key(&name.into())
    }

    /// Number of unique sections (i.e. with different names)
    pub fn len(&self) -> usize {
        self.sections.keys_len()
    }

    /// Load from a string
    pub fn load_from_str(data: &str) -> Result<Self, Error> {
        let mut parser = parser::Parser::new(data);
        let unit = parser.parse()?;

        Ok(unit)
    }

    /// Get an interator of values for all `key`s in all instances of `section`
    pub(crate) fn lookup_all<S, K>(&self, section: S, key: K) -> impl DoubleEndedIterator<Item=&str>
    where S: Into<String>,
          K: Into<String>,
    {
        self.lookup_all_values(section, key)
            .map(|v| v.unquoted().as_str())
    }

    /// Get an interator of values for all `key`s in all instances of `section`
    pub(crate) fn lookup_all_values<S, K>(&self, section: S, key: K) -> impl DoubleEndedIterator<Item=&EntryValue>
    where S: Into<String>,
          K: Into<String>,
    {
        self.sections
            .get(&section.into())
            .unwrap_or_default()
            .data
            .get_all(&key.into())
            .map(|v| v)
    }

    /// Get a Vec of values for all `key`s in all instances of `section`
    /// This mimics quadlet's behavior in that empty values reset the list.
    pub(crate) fn lookup_all_with_reset<S, K>(&self, section: S, key: K) -> Vec<&str>
    where S: Into<String>,
          K: Into<String>,
    {
        let values = self.sections
            .get(&section.into())
            .unwrap_or_default()
            .data.get_all(&key.into())
            .map(|v| v.unquoted().as_str());

        // size_hint.0 is not optimal, but may prevent forseeable growing
        let est_cap = values.size_hint().0;
        values.fold( Vec::with_capacity(est_cap), |mut res, v| {
            if v.is_empty() {
                res.clear();
            } else {
                res.push(v);
            }
            res
        })
    }

    // Get the last value for `key` in all instances of `section`
    pub(crate) fn lookup_last<S, K>(&self, section: S, key: K) -> Option<&str>
    where S: Into<String>,
          K: Into<String>,
    {
        self.lookup_last_value(section, key)
            .map(|v| v.unquoted().as_str())
    }

    // Get the last value for `key` in all instances of `section`
    pub(crate) fn lookup_last_value<S, K>(&self, section: S, key: K) -> Option<&EntryValue>
    where S: Into<String>,
          K: Into<String>,
    {
        self.sections
            .get(&section.into())
            .unwrap_or_default()
            .data
            .get_all(&key.into())
            .last()
    }

    pub(crate) fn new() -> Self {
        SystemdUnit {
            path: None,
            sections: Default::default(),
        }
    }

    pub(crate) fn merge_from(&mut self, other: &SystemdUnit) {
        for (section, entries) in other.sections.iter() {
            for (key, value) in entries.data.iter() {
                self.append_entry_value(section, key, value.clone());
            }
        }
    }

    pub(crate) fn path(&self) -> &Option<PathBuf> {
        &self.path
    }

    pub(crate) fn rename_section<S: Into<String>>(&mut self, from: S, to: S) {
        let from_key = from.into();

        if !self.sections.contains_key(&from_key) {
            return
        }

        let from_values: Vec<Entries> = self.sections.remove_all(&from_key).collect();

        if from_values.is_empty() {
            return
        }

        let to_key = to.into();
        for entries in from_values {
            for (ek, ev) in entries.data {
                self.append_entry_value(to_key.clone(), ek, ev);
            }
        }
    }

    pub(crate) fn section_entries<S: Into<String>>(&self, name: S) -> impl DoubleEndedIterator<Item=(&str, &str)> {
        self.section_entry_values(name)
            .map(|(k, v)| (k, v.unquoted().as_str()))
    }

    pub(crate) fn section_entry_values<S: Into<String>>(&self, name: S) -> impl DoubleEndedIterator<Item=(&str, &EntryValue)> {
        self.sections
            .get(&name.into())
            .unwrap_or_default()
            .data
            .iter()
            .map(|(k, v)| (k.as_str(), v))
    }

    pub(crate) fn set_entry<S, K, V>(&mut self, section: S, key: K, value: V)
    where S: Into<String>,
          K: Into<String>,
          V: Into<String>,
    {
        let value = value.into();

        self.set_entry_value(
            section,
            key,
            EntryValue::from_unquoted(value),
        );
    }

    pub(crate) fn set_entry_raw<S, K, V>(&mut self, section: S, key: K, value: V)
    where S: Into<String>,
          K: Into<String>,
          V: Into<String>,
    {
        self.set_entry_value(
            section,
            key,
            EntryValue::try_from_raw(value).unwrap(),
        );
    }

    pub(crate) fn set_entry_value<S, K>(&mut self, section: S, key: K, value: EntryValue)
    where S: Into<String>,
          K: Into<String>,
    {
        let entries = self.sections
            .entry(section.into())
            .or_insert(Entries::default());

        let key = key.into();

        // we can't replace the last value directly, so we have to get "creative" O.o
        // we do a stupid form of read-modify-write called remove-modify-append m(
        // the good thing is: both remove() and append preserve the order of values (with this key)
        let mut values: Vec<_> = entries.data.remove_all(&key).collect();
        values.pop();  // remove the "old" last value ...
        // ... reinsert all the values again ...
        for v in values {
            entries.data.append(key.clone(), v);

        }
        // ... and append a "new" last value
        entries.data.append(key.into(), value);
    }

    /// Write to a writer
    pub(crate) fn write_to<W: io::Write>(&self, writer: &mut W) -> io::Result<()> {
        for (section, entries) in &self.sections {
            write!(writer, "[{}]\n", section)?;
            for (k, v) in &entries.data {
                write!(writer, "{}={}\n", k, v.raw())?;
            }
            write!(writer, "\n")?;
        }

        Ok(())
    }
}
