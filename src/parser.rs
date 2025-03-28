use super::*;

use std::fmt::Display;
use std::str::Chars;

const LINE_CONTINUATION_REPLACEMENT: &str = " ";

type ParseResult<T> = Result<T, ParseError>;
#[derive(Debug, PartialEq, Eq)]
pub struct ParseError {
    pub line: usize,
    pub col: usize,
    pub msg: String,
}

impl Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{} {}", self.line, self.col, self.msg)
    }
}

#[derive(Debug)]
pub struct Parser<'a> {
    cur: Option<char>,
    buf: Chars<'a>,
    line: usize,
    column: usize,
}

impl<'a> Parser<'a> {
    pub fn new(buf: &'a str) -> Self {
        let mut p = Self {
            cur: None,
            buf: buf.chars(),
            line: 0,
            column: 0,
        };
        p.bump();
        p
    }

    fn bump(&mut self) {
        self.cur = self.buf.next();
        match self.cur {
            Some('\n') => {
                self.line += 1;
                self.column = 0;
            }
            Some(..) => {
                self.column += 1;
            }
            None => {}
        }
    }

    fn error(&self, msg: String) -> ParseError {
        ParseError {
            line: self.line,
            col: self.column,
            msg,
         }
    }

    pub fn parse(&mut self) -> ParseResult<SystemdUnit> {
        self.parse_unit()
    }

    // COMMENT        = ('#' | ';') ANY* NL
    fn parse_comment(&mut self) -> ParseResult<String> {
        match self.cur {
            Some('#' | ';') => (),
            Some(c) => return Err(self.error(format!("expected comment, but found {c:?}"))),
            None => return Err(self.error("expected comment, but found EOF".to_string())),
        }

        let comment = self.parse_until_any_of(&['\n']);
        Ok(comment)
    }

    // ENTRY          = KEY WS* '=' WS* VALUE NL
    fn parse_entry(&mut self) -> ParseResult<(EntryKey, EntryRawValue)> {
        let key = self.parse_key()?;

        // skip whitespace before '='
        let _ = self.parse_until_none_of(&[' ', '\t']);
        match self.cur {
            Some('=') => self.bump(),
            Some(c) => return Err(self.error(format!("expected '=' after key, but found {c:?}"))),
            None => return Err(self.error("expected '=' after key, but found EOF".to_string())),
        }
        // skip whitespace after '='
        let _ = self.parse_until_none_of(&[' ', '\t']);

        let value = self.parse_value()?;

        Ok((key, value))
    }

    // KEY            = [A-Za-z0-9-]
    fn parse_key(&mut self) -> ParseResult<EntryKey> {
        let key: String = self.parse_until_any_of(&['=', /*+ WHITESPACE*/' ', '\t', '\n', '\r'] );

        if !key.chars().all(|c| c.is_alphanumeric() || c == '-') {
            return Err(self.error(format!("Invalid key {:?}. Allowed characters are A-Za-z0-9-", key)))
        }

        Ok(key)
    }

    // SECTION        = SECTION_HEADER [COMMENT | ENTRY]*
    fn parse_section(&mut self) -> ParseResult<(SectionKey, Vec<(EntryKey, EntryRawValue)>)> {
        let name = self.parse_section_header()?;
        let mut entries: Vec<(EntryKey, EntryRawValue)> = Vec::new();


        while let Some(c) = self.cur {
            match c {
                '#' | ';' => {
                    // ignore comment
                    let _ = self.parse_comment();
                },
                '[' => break,
                _ if c.is_ascii_whitespace() => self.bump(),
                _ => {
                    entries.push(self.parse_entry()?);
                },
            }
        }

        Ok((name, entries))
    }

    // SECTION_HEADER = '[' ANY+ ']' NL
    fn parse_section_header(&mut self) -> ParseResult<String> {
        match self.cur {
            Some('[') => self.bump(),
            Some(c) => return Err(self.error(format!("expected '[' as start of section header, but found {c:?}"))),
            None => return Err(self.error("expected '[' as start of section header, but found EOF".to_string())),
        }

        let section_name = self.parse_until_any_of(&[']', '\n']);

        match self.cur {
            Some(']') => self.bump(),
            Some(c) => return Err(self.error(format!("expected ']' as end of section header, but found {c:?}"))),
            None => return Err(self.error("expected ']' as end of section header, but found EOF".to_string())),
        }

        if section_name.is_empty() {
            return Err(self.error("section header cannot be empty".into()));
        } else {
            // TODO: validate section name
        }

        // TODO: silently accept whitespace until EOL

        Ok(section_name)
    }

    // UNIT           = [COMMENT | SECTION]*
    fn parse_unit(&mut self) -> ParseResult<SystemdUnit> {
        let mut unit = SystemdUnit::new();

        while let Some(c) = self.cur {
            match c {
                '#' | ';' => {
                    // ignore comment
                    let _ = self.parse_comment();
                },
                '[' => {
                    let (section, entries) = self.parse_section()?;
                    // make sure there's a section entry (even if `entries` is empty)
                    unit.sections.entry(section.clone()).or_insert(Entries::default());
                    for (key, value) in entries {
                        unit.append_entry_value(
                            section.as_str(),
                            key,
                            match EntryValue::try_from_raw(value) {
                                Ok(v) => v,
                                Err(e) => return Err(self.error(e.to_string())),
                            },
                        );
                    }
                },
                _ if c.is_ascii_whitespace() => self.bump(),
                _ => return Err(self.error("Expected comment or section".into())),
            };
        }

        Ok(unit)
    }

    fn parse_until_any_of(&mut self, end: &[char]) -> String {
        let mut s = String::new();

        while let Some(c) = self.cur {
            if end.contains(&c) {
                break;
            }
            s.push(c);
            self.bump();
        }

        s
    }

    fn parse_until_none_of(&mut self, end: &[char]) -> String {
        let mut s = String::new();

        while let Some(c) = self.cur {
            if !end.contains(&c) {
                break;
            }
            s.push(c);
            self.bump();
        }

        s
    }

    // VALUE          = ANY* CONTINUE_NL [COMMENT]* VALUE
    fn parse_value(&mut self) -> ParseResult<EntryRawValue> {
        let mut value: String = String::new();
        let mut backslash = false;
        let mut line_continuation = false;

        while let Some(c) = self.cur {
            if backslash {
                backslash = false;
                match c {
                    // line continuation -> add replacement to value and continue normally
                    '\n' => {
                        value.push_str(LINE_CONTINUATION_REPLACEMENT);
                        line_continuation = true;
                    },
                    // just an escape sequence -> add to value and continue normally
                    _ => {
                        value.push('\\');
                        value.push(c);
                    },
                }
            } else if line_continuation {
                line_continuation = false;
                match c {
                    '#' | ';' => {
                        // ignore interspersed comments
                        let _ = self.parse_comment();
                        line_continuation = true;
                    },
                    // end of value
                    '\n' => break,
                    // start of section header (although an unexpected one), i.e. end of value
                    // NOTE: we're trying to be clever here and assume the line continuation was a mistake
                    '[' => break,
                    // value continues after line continuation, add the actual line
                    // continuation characters back to value and continue normally
                    _ => {
                        if c == '\\' {
                            // we may have a line continuation following another line continuation
                            backslash = true;
                        } else {
                            value.push(c);
                        }
                    },
                }
            } else {
                match c {
                    // may be start of a line continuation
                    '\\' => backslash = true,
                    // end of value
                    '\n' => break,
                    _ => value.push(c),
                }
            }
            self.bump();
        }

        Ok(value)
    }
}
