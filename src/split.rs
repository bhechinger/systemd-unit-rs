use std::str::Chars;

const WHITESPACE: [char; 4] = [' ', '\t', '\n', '\r'];

/// Splits a string at whitespace and removes quotes while preserving whitespace *inside* quotes.
/// It will *keep* escape sequences as they are (i.e. treat them as normal characters).
///
/// splits space separated values similar to the systemd config_parse_strv, merging multiple values into a single vector
/// equals behavior of Systemd's `extract_first_word()` with  `EXTRACT_RETAIN_ESCAPE|EXTRACT_UNQUOTE` flags
// EXTRACT_UNQUOTE       = Ignore separators in quoting with "" and '', and remove the quotes.
// EXTRACT_RETAIN_ESCAPE = Treat escape character '\' as any other character without special meaning
pub struct SplitStrv<'a> {
    chars: Chars<'a>,  // `src.chars()`
    c: Option<char>,  // the current character
}

impl<'a> SplitStrv<'a> {
    fn bump(&mut self) {
        self.c = self.chars.next();
    }

    pub fn new(src: &'a str) -> Self {
        let mut s = Self {
            chars: src.chars(),
            c: None,
        };
        s.bump();
        s
    }

    pub fn next<'b>(&mut self) -> Option<String> {
        let separators = &WHITESPACE;
        let mut word = String::new();

        // skip initial whitespace
        self.parse_until_none_of(separators);

        let mut quote: Option<char> = None;  // None or Some('\'') or Some('"')
        while let Some(c) = self.c {
            if let Some(q) = quote {
                // inside either single or double quotes
                match self.c {
                    Some(c) if c == q => {
                        quote = None
                    },
                    _ => word.push(c),
                }
            } else {
                match c {
                    '\'' | '"' => {
                        quote = Some(c)
                    },
                    _ if separators.contains(&c) => {
                        // word is done
                        break
                    },
                    _ => word.push(c),
                }
            }

            self.bump();
        }

        if word.is_empty() {
            None
        } else {
            Some(word)
        }
    }

    fn parse_until_none_of(&mut self, end: &[char]) -> String {
        let mut s = String::new();

        while let Some(c) = self.c {
            if !end.contains(&c) {
                break;
            }
            s.push(c);
            self.bump();
        }

        s
    }
}

impl<'a> Iterator for SplitStrv<'a> {
    type Item = String;

    fn next(&mut self) -> Option<Self::Item> {
        self.next()
    }
}

/// Splits a string at whitespace and removes quotes while preserving whitespace *inside* quotes.
/// It will also unescape known escape sequences.
///
/// equals behavior of Systemd's `extract_first_word()` with  `EXTRACT_RELAX|EXTRACT_UNQUOTE|EXTRACT_CUNESCAPE` flags
// EXTRACT_RELAX     = Allow unbalanced quote and eat up trailing backslash.
// EXTRACT_CUNESCAPE = Unescape known escape sequences.
// EXTRACT_UNQUOTE   = Ignore separators in quoting with "" and '', and remove the quotes.
pub struct SplitWord<'a> {
    chars: Chars<'a>,  // `src.chars()`
    c: Option<char>,  // the current character
}

impl<'a> SplitWord<'a> {
    fn bump(&mut self) {
        self.c = self.chars.next();
    }

    pub fn new(src: &'a str) -> Self {
        let mut s = Self {
            chars: src.chars(),
            c: None,
        };
        s.bump();
        s
    }

    pub fn next<'b>(&mut self) -> Option<String> {
        let separators = &WHITESPACE;
        let mut word = String::new();

        // skip initial whitespace
        self.parse_until_none_of(separators);

        let mut quote: Option<char> = None;  // None or Some('\'') or Some('"')
        let mut backslash = false;  // whether we've just seen a backslash
        while let Some(c) = self.c {
            if backslash {
                match self.parse_escape_sequence() {
                    Ok(r) => word.push(r),
                    Err(_) => return None,
                };

                backslash = false;
            } else if let Some(q) = quote {
                // inside either single or double quotes
                word.push_str(self.parse_until_any_of(&[q, '\\']).as_str());

                match self.c {
                    Some(c) if c == q => {
                        quote = None;
                    },
                    Some('\\') => backslash = true,
                    _ => (),
                }
            } else {
                match c {
                    '\'' | '"' => {
                        quote = Some(c)
                    },
                    '\\' => {
                        backslash = true;
                    }
                    _ if separators.contains(&c) => {
                        // word is done
                        break;
                    },
                    _ => word.push(c),
                }
            }

            self.bump();
        }

        // if backslash {
        //     // do nothing -> eat up trailing backslash
        //     // otherwise we'd have to push it onto `word`
        // }

        if word.is_empty() {
            None
        } else {
            Some(word)
        }
    }

    fn parse_escape_sequence(&mut self) -> Result<char, String> {
        if let Some(c) = self.c {
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
                c => c
            };

            Ok(r)
        } else {
            Err("expecting escape sequence, but found EOF.".into())
        }
    }

    fn parse_unicode_escape(&mut self, prefix: Option<char>, max_chars: usize, radix: u32) -> Result<char, String> {
        assert!(prefix.is_none() || (prefix.is_some() && ['x', 'u', 'U'].contains(&prefix.unwrap())));
        assert!([8, 16].contains(&radix));

        let mut code = String::with_capacity(max_chars);
        for _ in 0..max_chars {
            if let Some(c) = self.c {
                code.push(c);
                if radix == 16 && !c.is_ascii_hexdigit() {
                    return Err(format!("Expected {max_chars} hex values after \"\\{c}\", but got \"\\{c}{code}\"" ))
                } else if radix == 8 && (!c.is_ascii_digit() || c == '8' || c == '9') {
                    return Err(format!("Expected {max_chars} octal values after \"\\\", but got \"\\{code}\"" ))
                }
            } else {
                return Err("expecting unicode escape sequence, but found EOF.".into())
            }

            if code.len() != max_chars {
                self.bump();
            }
        }

        let ucp = u32::from_str_radix(code.as_str(), radix).unwrap();
        if ucp == 0 {
            return Err("\\0 character not allowed in escape sequence".into())
        }

        match char::try_from(ucp) {
            Ok(u) => Ok(u),
            Err(e) => Err(format!("invalid unicode character in escape sequence: {e}")),
        }
    }

    fn parse_until_any_of(&mut self, end: &[char]) -> String {
        let mut s = String::new();

        while let Some(c) = self.c {
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

        while let Some(c) = self.c {
            if !end.contains(&c) {
                break;
            }
            s.push(c);
            self.bump();
        }

        s
    }
}

impl<'a> Iterator for SplitWord<'a> {
    type Item = String;

    fn next(&mut self) -> Option<Self::Item> {
        self.next()
    }
}

// impl<'a> IntoIterator for SplitWord<'a> {
//     type Item = &'a str;
//     type IntoIter = Self;

//     fn into_iter(self) -> Self::IntoIter {
//         self
//     }
// }