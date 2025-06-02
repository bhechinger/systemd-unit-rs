mod constants;
mod parser;
mod quoted;
mod split;
mod value;
pub use self::constants::*;
pub use self::quoted::*;
pub use self::split::*;
pub use self::value::*;

use ordered_multimap::list_ordered_multimap::ListOrderedMultimap;
use std::fmt;
use std::fs::File;
use std::io;
use std::io::BufWriter;
use std::path::{Path, PathBuf};

// TODO: mimic https://doc.rust-lang.org/std/num/enum.IntErrorKind.html
// TODO: use thiserror?
#[derive(Debug, PartialEq)]
#[non_exhaustive]
pub enum Error {
    ParseBool,
    Unquoting(String),
    Unit(parser::ParseError),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::ParseBool => {
                write!(
                    f,
                    "value must be one of `1`, `yes`, `true`, `on`, `0`, `no`, `false`, `off`"
                )
            }
            Error::Unquoting(msg) => {
                write!(f, "failed unquoting value: {msg}")
            }
            Error::Unit(e) => {
                write!(f, "failed to parse unit file: {e}")
            }
        }
    }
}

impl From<parser::ParseError> for Error {
    fn from(e: parser::ParseError) -> Self {
        Error::Unit(e)
    }
}

pub fn parse_bool(s: &str) -> Result<bool, Error> {
    if ["1", "yes", "true", "on"].contains(&s) {
        return Ok(true);
    } else if ["0", "no", "false", "off"].contains(&s) {
        return Ok(false);
    }

    Err(Error::ParseBool)
}

#[derive(Debug, PartialEq)]
pub struct SystemdUnit {
    pub path: Option<PathBuf>,
    sections: ListOrderedMultimap<SectionKey, Entries>,
}

impl SystemdUnit {
    /// Appends `key=value` to last instance of `section`
    pub fn append_entry<S, K, V>(&mut self, section: S, key: K, value: V)
    where
        S: Into<String>,
        K: Into<String>,
        V: Into<String>,
    {
        self.append_entry_value(section, key, EntryValue::from_unquoted(value));
    }
    /// Appends `key=value` to last instance of `section`
    pub fn append_entry_value<S, K>(&mut self, section: S, key: K, value: EntryValue)
    where
        S: Into<String>,
        K: Into<String>,
    {
        self.sections
            .entry(section.into())
            .or_insert_entry(Entries::default())
            .into_mut()
            .data
            .append(key.into(), value);
    }

    pub fn has_key<S, K>(&self, section: S, key: K) -> bool
    where
        S: Into<String>,
        K: Into<String>,
    {
        self.sections
            .get(&section.into())
            .map_or(false, |e| e.data.contains_key(&key.into()))
    }

    /// Retrun `true` if there's an (non-empty) instance of section `name`
    pub fn has_section<S: Into<String>>(&self, name: S) -> bool {
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
    pub fn lookup_all<S, K>(&self, section: S, key: K) -> impl DoubleEndedIterator<Item = String>
    where
        S: Into<String>,
        K: Into<String>,
    {
        self.lookup_all_values(section, key).map(|v| v.unquote())
    }

    /// Get an interator of values for all `key`s in all instances of `section`
    pub fn lookup_all_values<S, K>(
        &self,
        section: S,
        key: K,
    ) -> impl DoubleEndedIterator<Item = &EntryValue>
    where
        S: Into<String>,
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
    pub fn lookup_all_with_reset<S, K>(&self, section: S, key: K) -> Vec<&str>
    where
        S: Into<String>,
        K: Into<String>,
    {
        let values = self
            .sections
            .get(&section.into())
            .unwrap_or_default()
            .data
            .get_all(&key.into())
            .map(|v| v.unquoted().as_str());

        // size_hint.0 is not optimal, but may prevent forseeable growing
        let est_cap = values.size_hint().0;
        values.fold(Vec::with_capacity(est_cap), |mut res, v| {
            if v.is_empty() {
                res.clear();
            } else {
                res.push(v);
            }
            res
        })
    }

    // Get the last value for `key` in all instances of `section`
    pub fn lookup_last<S, K>(&self, section: S, key: K) -> Option<String>
    where
        S: Into<String>,
        K: Into<String>,
    {
        self.lookup_last_value(section, key).map(|v| v.unquote())
    }

    // Get the last value for `key` in all instances of `section`
    pub fn lookup_last_value<S, K>(&self, section: S, key: K) -> Option<&EntryValue>
    where
        S: Into<String>,
        K: Into<String>,
    {
        self.sections
            .get(&section.into())
            .unwrap_or_default()
            .data
            .get_all(&key.into())
            .last()
    }

    pub fn new() -> Self {
        SystemdUnit {
            path: None,
            sections: Default::default(),
        }
    }

    pub fn merge_from(&mut self, other: &SystemdUnit) {
        for (section, entries) in other.sections.iter() {
            for (key, value) in entries.data.iter() {
                self.append_entry_value(section, key, value.clone());
            }
        }
    }

    pub fn path(&self) -> &Option<PathBuf> {
        &self.path
    }

    pub fn rename_section<S: Into<String>>(&mut self, from: S, to: S) {
        let from_key = from.into();

        if !self.sections.contains_key(&from_key) {
            return;
        }

        let from_values: Vec<Entries> = self.sections.remove_all(&from_key).collect();

        if from_values.is_empty() {
            return;
        }

        let to_key = to.into();
        for entries in from_values {
            for (ek, ev) in entries.data {
                self.append_entry_value(to_key.clone(), ek, ev);
            }
        }
    }

    pub fn section_entries<S: Into<String>>(
        &self,
        name: S,
    ) -> impl DoubleEndedIterator<Item = (&str, String)> {
        self.section_entry_values(name)
            .map(|(k, v)| (k, v.unquote()))
    }

    pub fn section_entry_values<S: Into<String>>(
        &self,
        name: S,
    ) -> impl DoubleEndedIterator<Item = (&str, &EntryValue)> {
        self.sections
            .get(&name.into())
            .unwrap_or_default()
            .data
            .iter()
            .map(|(k, v)| (k.as_str(), v))
    }

    pub fn set_entry<S, K, V>(&mut self, section: S, key: K, value: V)
    where
        S: Into<String>,
        K: Into<String>,
        V: Into<String>,
    {
        let value = value.into();

        self.set_entry_value(section, key, EntryValue::from_unquoted(value));
    }

    pub fn set_entry_raw<S, K, V>(&mut self, section: S, key: K, value: V)
    where
        S: Into<String>,
        K: Into<String>,
        V: Into<String>,
    {
        self.set_entry_value(section, key, EntryValue::try_from_raw(value).unwrap());
    }

    pub fn set_entry_value<S, K>(&mut self, section: S, key: K, value: EntryValue)
    where
        S: Into<String>,
        K: Into<String>,
    {
        let entries = self
            .sections
            .entry(section.into())
            .or_insert(Entries::default());

        let key = key.into();

        // we can't replace the last value directly, so we have to get "creative" O.o
        // we do a stupid form of read-modify-write called remove-modify-append m(
        // the good thing is: both remove() and append preserve the order of values (with this key)
        let mut values: Vec<_> = entries.data.remove_all(&key).collect();
        values.pop(); // remove the "old" last value ...
        // ... reinsert all the values again ...
        for v in values {
            entries.data.append(key.clone(), v);
        }
        // ... and append a "new" last value
        entries.data.append(key.into(), value);
    }

    /// Write to a writer
    pub fn write_to<W: io::Write>(&self, writer: &mut W) -> io::Result<()> {
        write!(writer, "# Automatically generated by systemd-unit-rs\n")?;

        for (section, entries) in &self.sections {
            write!(writer, "[{}]\n", section)?;
            for (k, v) in &entries.data {
                write!(writer, "{}={}\n", k, v.raw())?;
            }
            write!(writer, "\n")?;
        }

        Ok(())
    }

    pub fn generate_service_file(
        &self,
        output_path: &Path,
        service_name: &PathBuf,
    ) -> io::Result<()> {
        let out_filename = output_path.join(service_name);

        let out_file = File::options()
            .truncate(true)
            .write(true)
            .create(true)
            .open(&out_filename)?;
        let mut writer = BufWriter::new(out_file);

        self.write_to(&mut writer)?;

        Ok(())
    }
}
