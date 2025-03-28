use std::str::Chars;

use super::Error;

fn char_needs_escaping(c: char) -> bool {
    if c as usize > 128 {
        return false;
    }

    c.is_ascii_control() ||
        c.is_ascii_whitespace() ||
        c == '"' ||
        c == '\'' ||
        c == '\\'
}

pub fn quote_value(value: &str) -> String {
    let mut escaped: String = String::with_capacity(value.len());
    for c in value.chars() {
        // anything beyond ASCII doesn't need to be escaped (yay Unicode)
        // we only care about ASCII control characters AND '\'
        if !char_needs_escaping(c) {
            escaped.push(c);
            continue;
        }

        match c {
            '\x07' => escaped.push_str("\\a"),
            '\x08' => escaped.push_str("\\b"),
            '\n'   => escaped.push_str("\\n"),
            '\r'   => escaped.push_str("\\r"),
            '\t'   => escaped.push_str("\\t"),
            '\x0b' => escaped.push_str("\\v"),
            '\x0c' => escaped.push_str("\\f"),
            '\\'   => escaped.push_str("\\\\"),
            ' '    => escaped.push_str(" "),
            '"'    => escaped.push_str("\\\""),
            '\''    => escaped.push_str("'"),
            _ => escaped.push_str(&format!("\\x{:02x}", c as isize)[..])
        }
    }
    escaped
}

pub fn quote_words<'a, S>(words: impl Iterator<Item=S>) -> String
    where S: Into<&'a str>
{
    words.map(|word| {
        let word = word.into();
        if word_needs_escaping(word) {
            format!("\"{}\"", quote_value(word))
        } else {
            word.to_string()
        }
    })
    .collect::<Vec<String>>()
    .join(" ")
}

pub fn unquote_value(raw: &str) -> Result<String, Error> {
    let mut parser = Quoted {
        chars: raw.chars(),
        cur: None,
    };
    parser.bump();

    parser.parse_and_unquote()
}

fn word_needs_escaping(word: &str) -> bool {
    word.chars().any(char_needs_escaping)
}

struct Quoted<'a> {
    chars: Chars<'a>,
    cur: Option<char>,
}

impl<'a> Quoted<'a> {
    fn bump(&mut self) {
        self.cur = self.chars.next();
    }

    fn parse_and_unquote(&mut self) -> Result<String, Error> {
        let mut result: String = String::new();
        let mut quote: Option<char> = None;

        while self.cur.is_some() {
            match self.cur {
                None => return Err(Error::Unquoting("found early EOF".into())),
                Some('\'' | '"') if result.ends_with([' ', '\t', '\n']) || result.is_empty() => {
                    quote = self.cur;
                }
                Some('\\') => {
                    self.bump();
                    match self.cur {
                        None => return Err(Error::Unquoting("expecting escape sequence, but found EOF.".into())),
                        // line continuation (i.e. value continues on the next line)
                        Some(_) => result.push(self.parse_escape_sequence()?),
                    }
                }
                Some(c) => {
                    if self.cur == quote {
                        // inside either single or double quotes
                        quote = None;
                    } else {
                        result.push(c);
                    }
                }
            }
            self.bump();
        }
        Ok(result)
    }

    fn parse_escape_sequence(&mut self) -> Result<char, Error> {
        if let Some(c) = self.cur {
            let r = match c {
                'a'  => '\u{7}',
                'b'  => '\u{8}',
                'f'  => '\u{c}',
                'n'  => '\n',
                'r'  => '\r',
                't'  => '\t',
                'v'  => '\u{b}',
                '\\' => '\\',
                '"'  => '"',
                '\'' => '\'',
                's'  => ' ',

                'x'  => {  // 2 character hex encoding
                    self.bump();
                    self.parse_unicode_escape(Some('x'), 2, 16)?
                },
                'u'  => {  // 4 character hex encoding
                    self.bump();
                    self.parse_unicode_escape(Some('u'), 4, 16)?
                },
                'U'  => {  // 8 character hex encoding
                    self.bump();
                    self.parse_unicode_escape(Some('U'), 8, 16)?
                },
                '0'..='7' => {  // 3 character octal encoding
                    self.parse_unicode_escape(None, 3, 8)?
                }
                c => return Err(Error::Unquoting(format!("expecting escape sequence, but found {c:?}.")))
            };

            Ok(r)
        } else {
            Err(Error::Unquoting("expecting escape sequence, but found EOF.".into()))
        }
    }

    fn parse_unicode_escape(&mut self, prefix: Option<char>, max_chars: usize, radix: u32) -> Result<char, Error> {
        assert!(prefix.is_none() || (prefix.is_some() && ['x', 'u', 'U'].contains(&prefix.unwrap())));
        assert!([8, 16].contains(&radix));

        let mut code = String::with_capacity(max_chars);
        for _ in 0..max_chars {
            if let Some(c) = self.cur {
                code.push(c);
                if radix == 16 && !c.is_ascii_hexdigit() {
                    return Err(Error::Unquoting(format!("expected {max_chars} hex values after \"\\{c}\", but got \"\\{c}{code}\"" )))
                } else if radix == 8 && (!c.is_ascii_digit() || c == '8' || c == '9') {
                    return Err(Error::Unquoting(format!("expected {max_chars} octal values after \"\\\", but got \"\\{code}\"" )))
                }
            } else {
                return Err(Error::Unquoting("expecting unicode escape sequence, but found EOF.".into()))
            }

            if code.len() != max_chars {
                self.bump();
            }
        }

        let ucp = u32::from_str_radix(code.as_str(), radix).unwrap();
        if ucp == 0 {
            return Err(Error::Unquoting("\\0 character not allowed in escape sequence".into()))
        }

        match char::try_from(ucp) {
            Ok(u) => Ok(u),
            Err(e) => Err(Error::Unquoting(format!("invalid unicode character in escape sequence: {e}"))),
        }
    }
}
