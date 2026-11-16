#[derive(Clone, PartialEq)]
pub enum Tok {
    Int(i64),
    Str(String),
    Id(String),
    Sym(&'static str),
    Eof,
}

const SYMS: [&str; 23] = [
    "==", "!=", "<=", ">=", "&&", "||", "+", "-", "*", "/", "%", "<", ">", "=", "!", "(", ")", "{", "}", "[",
    "]", ";", ",",
];

pub fn lex(src: &str) -> Result<Vec<(Tok, usize)>, String> {
    let b = src.as_bytes();
    let (mut i, mut line, mut out) = (0, 1, Vec::new());
    while i < b.len() {
        let c = b[i];
        if c == b'\n' {
            line += 1;
            i += 1;
        } else if c.is_ascii_whitespace() {
            i += 1;
        } else if src[i..].starts_with("//") {
            while i < b.len() && b[i] != b'\n' {
                i += 1;
            }
        } else if c.is_ascii_digit() {
            let hex = src[i..].starts_with("0x");
            if hex {
                i += 2;
            }
            let s = i;
            while i < b.len() && b[i].is_ascii_hexdigit() && (hex || b[i].is_ascii_digit()) {
                i += 1;
            }
            let n = if hex { u64::from_str_radix(&src[s..i], 16).map(|n| n as i64) } else { src[s..i].parse() };
            let n = n.map_err(|_| format!("{line}: invalid integer"))?;
            out.push((Tok::Int(n), line));
        } else if c.is_ascii_alphabetic() || c == b'_' {
            let s = i;
            while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_') {
                i += 1;
            }
            out.push((Tok::Id(src[s..i].to_string()), line));
        } else if c == b'"' {
            let mut s = String::new();
            i += 1;
            loop {
                match b.get(i) {
                    None | Some(b'\n') => return Err(format!("{line}: unterminated string")),
                    Some(b'"') => break i += 1,
                    Some(b'\\') => {
                        s.push(match b.get(i + 1) {
                            Some(b'n') => '\n',
                            Some(b't') => '\t',
                            Some(b'\\') => '\\',
                            Some(b'"') => '"',
                            _ => return Err(format!("{line}: unknown escape")),
                        });
                        i += 2;
                    }
                    _ => {
                        let ch = src[i..].chars().next().unwrap();
                        s.push(ch);
                        i += ch.len_utf8();
                    }
                }
            }
            out.push((Tok::Str(s), line));
        } else {
            let sym = SYMS.iter().find(|s| src[i..].starts_with(**s)).ok_or_else(|| {
                format!("{line}: unexpected '{}'", src[i..].chars().next().unwrap())
            })?;
            i += sym.len();
            out.push((Tok::Sym(sym), line));
        }
    }
    out.push((Tok::Eof, line));
    Ok(out)
}
