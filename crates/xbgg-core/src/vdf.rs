//! Minimal parser for Valve's text KeyValues format (`.vdf` / `.acf`).

use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Str(String),
    Obj(Vec<(String, Value)>),
}

impl Value {
    /// Case-insensitive lookup of a child key (Valve files are inconsistent with casing).
    pub fn get(&self, key: &str) -> Option<&Value> {
        match self {
            Value::Obj(items) => items
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(key))
                .map(|(_, v)| v),
            Value::Str(_) => None,
        }
    }

    pub fn str(&self, key: &str) -> Option<&str> {
        match self.get(key)? {
            Value::Str(s) => Some(s),
            Value::Obj(_) => None,
        }
    }

    pub fn children(&self) -> &[(String, Value)] {
        match self {
            Value::Obj(items) => items,
            Value::Str(_) => &[],
        }
    }

    /// Flattens string children into a map (convenience for simple sections).
    pub fn string_map(&self) -> BTreeMap<String, String> {
        self.children()
            .iter()
            .filter_map(|(k, v)| match v {
                Value::Str(s) => Some((k.clone(), s.clone())),
                Value::Obj(_) => None,
            })
            .collect()
    }
}

#[derive(Debug, PartialEq)]
enum Token {
    Str(String),
    Open,
    Close,
}

fn tokenize(input: &str) -> Result<Vec<Token>, String> {
    let mut tokens = Vec::new();
    let mut chars = input.chars().peekable();
    while let Some(&c) = chars.peek() {
        match c {
            c if c.is_whitespace() => {
                chars.next();
            }
            '/' => {
                chars.next();
                if chars.peek() == Some(&'/') {
                    for c in chars.by_ref() {
                        if c == '\n' {
                            break;
                        }
                    }
                } else {
                    return Err("unexpected '/'".into());
                }
            }
            '{' => {
                chars.next();
                tokens.push(Token::Open);
            }
            '}' => {
                chars.next();
                tokens.push(Token::Close);
            }
            '"' => {
                chars.next();
                let mut s = String::new();
                loop {
                    match chars.next() {
                        None => return Err("unterminated string".into()),
                        Some('"') => break,
                        Some('\\') => match chars.next() {
                            Some('n') => s.push('\n'),
                            Some('t') => s.push('\t'),
                            Some(other) => s.push(other),
                            None => return Err("unterminated escape".into()),
                        },
                        Some(other) => s.push(other),
                    }
                }
                tokens.push(Token::Str(s));
            }
            _ => {
                // Unquoted token (rare, but allowed by the format).
                let mut s = String::new();
                while let Some(&c) = chars.peek() {
                    if c.is_whitespace() || c == '{' || c == '}' || c == '"' {
                        break;
                    }
                    s.push(c);
                    chars.next();
                }
                tokens.push(Token::Str(s));
            }
        }
    }
    Ok(tokens)
}

fn parse_obj(tokens: &[Token], pos: &mut usize, nested: bool) -> Result<Vec<(String, Value)>, String> {
    let mut items = Vec::new();
    while *pos < tokens.len() {
        match &tokens[*pos] {
            Token::Close => {
                if nested {
                    *pos += 1;
                    return Ok(items);
                }
                return Err("unexpected '}'".into());
            }
            Token::Open => return Err("unexpected '{'".into()),
            Token::Str(key) => {
                let key = key.clone();
                *pos += 1;
                match tokens.get(*pos) {
                    Some(Token::Str(v)) => {
                        items.push((key, Value::Str(v.clone())));
                        *pos += 1;
                    }
                    Some(Token::Open) => {
                        *pos += 1;
                        let child = parse_obj(tokens, pos, true)?;
                        items.push((key, Value::Obj(child)));
                    }
                    _ => return Err(format!("missing value for key '{key}'")),
                }
            }
        }
    }
    if nested {
        Err("unexpected end of input".into())
    } else {
        Ok(items)
    }
}

/// Parses a KeyValues document. The result is an object holding the top-level keys.
pub fn parse(input: &str) -> Result<Value, String> {
    let input = input.trim_start_matches('\u{feff}');
    let tokens = tokenize(input)?;
    let mut pos = 0;
    Ok(Value::Obj(parse_obj(&tokens, &mut pos, false)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_library_folders() {
        let doc = r#"
"libraryfolders"
{
	"0"
	{
		"path"		"D:\\Steam"
		"apps"
		{
			"228980"		"367631878"
		}
	}
	// comment
	"1" { "path" "E:\\Games" }
}"#;
        let v = parse(doc).unwrap();
        let lf = v.get("libraryfolders").unwrap();
        assert_eq!(lf.children().len(), 2);
        assert_eq!(lf.get("0").unwrap().str("path"), Some("D:\\Steam"));
        assert_eq!(lf.get("1").unwrap().str("PATH"), Some("E:\\Games"));
    }

    #[test]
    fn rejects_unterminated() {
        assert!(parse("\"a\" { \"b\" \"c\"").is_err());
    }
}
