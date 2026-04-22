use beer_errors::CompileError;
use beer_span::Span;

#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    Let,
    Fn,
    If,
    Else,
    While,
    Return,
    True,
    False,
    As,
    Use,

    Int(i64),
    Float(f64),
    Str(String),
    Ident(String),

    LParen,
    RParen,
    LBrace,
    RBrace,
    Comma,
    Colon,
    Arrow,

    Eq,
    EqEq,
    BangEq,
    Bang,
    Lt,
    LtEq,
    Gt,
    GtEq,
    AndAnd,
    OrOr,

    Plus,
    Minus,
    Star,
    Slash,
}

struct Lexer<'a> {
    src: std::str::Chars<'a>,
    buf0: Option<char>,
    buf1: Option<char>,
    file: u32,
    line: u32,
    col: u32,
}

impl<'a> Lexer<'a> {
    fn new(src: &'a str, file: u32) -> Self {
        Self { src: src.chars(), buf0: None, buf1: None, file, line: 1, col: 1 }
    }

    fn peek(&mut self) -> Option<char> {
        if self.buf0.is_none() {
            self.buf0 = self.src.next();
        }
        self.buf0
    }

    fn peek2(&mut self) -> Option<char> {
        self.peek();
        if self.buf1.is_none() {
            self.buf1 = self.src.next();
        }
        self.buf1
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.buf0.take().or_else(|| self.src.next())?;
        self.buf0 = self.buf1.take();
        if c == '\n' {
            self.line += 1;
            self.col = 1;
        } else {
            self.col += 1;
        }
        Some(c)
    }

    fn span(&self) -> Span {
        Span::new(self.file, self.line, self.col)
    }
}

pub fn tokenize(src: &str, file: u32) -> Result<Vec<(Token, Span)>, CompileError> {
    let mut lx = Lexer::new(src, file);
    let mut out = Vec::new();

    while let Some(c) = lx.peek() {
        match c {
            ' ' | '\t' | '\n' | '\r' => {
                lx.bump();
            }
            '/' => {
                let start = lx.span();
                lx.bump();
                if lx.peek() == Some('/') {
                    while let Some(c) = lx.peek() {
                        lx.bump();
                        if c == '\n' {
                            break;
                        }
                    }
                } else {
                    out.push((Token::Slash, start));
                }
            }
            '(' => { let s = lx.span(); lx.bump(); out.push((Token::LParen, s)); }
            ')' => { let s = lx.span(); lx.bump(); out.push((Token::RParen, s)); }
            '{' => { let s = lx.span(); lx.bump(); out.push((Token::LBrace, s)); }
            '}' => { let s = lx.span(); lx.bump(); out.push((Token::RBrace, s)); }
            ',' => { let s = lx.span(); lx.bump(); out.push((Token::Comma, s)); }
            ':' => { let s = lx.span(); lx.bump(); out.push((Token::Colon, s)); }
            '+' => { let s = lx.span(); lx.bump(); out.push((Token::Plus, s)); }
            '*' => { let s = lx.span(); lx.bump(); out.push((Token::Star, s)); }
            '-' => {
                let s = lx.span();
                lx.bump();
                if lx.peek() == Some('>') {
                    lx.bump();
                    out.push((Token::Arrow, s));
                } else {
                    out.push((Token::Minus, s));
                }
            }
            '=' => {
                let s = lx.span();
                lx.bump();
                if lx.peek() == Some('=') {
                    lx.bump();
                    out.push((Token::EqEq, s));
                } else {
                    out.push((Token::Eq, s));
                }
            }
            '!' => {
                let s = lx.span();
                lx.bump();
                if lx.peek() == Some('=') {
                    lx.bump();
                    out.push((Token::BangEq, s));
                } else {
                    out.push((Token::Bang, s));
                }
            }
            '<' => {
                let s = lx.span();
                lx.bump();
                if lx.peek() == Some('=') {
                    lx.bump();
                    out.push((Token::LtEq, s));
                } else {
                    out.push((Token::Lt, s));
                }
            }
            '>' => {
                let s = lx.span();
                lx.bump();
                if lx.peek() == Some('=') {
                    lx.bump();
                    out.push((Token::GtEq, s));
                } else {
                    out.push((Token::Gt, s));
                }
            }
            '&' => {
                let s = lx.span();
                lx.bump();
                if lx.peek() == Some('&') {
                    lx.bump();
                    out.push((Token::AndAnd, s));
                } else {
                    return Err(CompileError::at(s, "unexpected '&' (did you mean '&&'?)"));
                }
            }
            '|' => {
                let s = lx.span();
                lx.bump();
                if lx.peek() == Some('|') {
                    lx.bump();
                    out.push((Token::OrOr, s));
                } else {
                    return Err(CompileError::at(s, "unexpected '|' (did you mean '||'?)"));
                }
            }
            '"' => {
                let start = lx.span();
                lx.bump();
                let mut s = String::new();
                loop {
                    match lx.bump() {
                        None => return Err(CompileError::at(start, "unterminated string literal")),
                        Some('"') => break,
                        Some('\\') => match lx.bump() {
                            Some('n') => s.push('\n'),
                            Some('t') => s.push('\t'),
                            Some('\\') => s.push('\\'),
                            Some('"') => s.push('"'),
                            Some('0') => s.push('\0'),
                            Some(c) => {
                                return Err(CompileError::at(start, format!("unknown escape \\{}", c)));
                            }
                            None => return Err(CompileError::at(start, "unterminated string literal")),
                        },
                        Some(c) => s.push(c),
                    }
                }
                out.push((Token::Str(s), start));
            }
            c if c.is_ascii_digit() => {
                let start = lx.span();
                let mut int_text = String::new();
                while let Some(c) = lx.peek() {
                    if c.is_ascii_digit() {
                        int_text.push(c);
                        lx.bump();
                    } else {
                        break;
                    }
                }
                if lx.peek() == Some('.')
                    && lx.peek2().map_or(false, |c| c.is_ascii_digit())
                {
                    lx.bump();
                    let mut frac_text = String::new();
                    while let Some(c) = lx.peek() {
                        if c.is_ascii_digit() {
                            frac_text.push(c);
                            lx.bump();
                        } else {
                            break;
                        }
                    }
                    let lit = format!("{}.{}", int_text, frac_text);
                    let f: f64 = lit.parse().map_err(|_| {
                        CompileError::at(start, format!("invalid float literal: {}", lit))
                    })?;
                    out.push((Token::Float(f), start));
                } else {
                    let n: i64 = int_text.parse().map_err(|_| {
                        CompileError::at(start, format!("integer literal overflow: {}", int_text))
                    })?;
                    out.push((Token::Int(n), start));
                }
            }
            c if c.is_ascii_alphabetic() || c == '_' => {
                let start = lx.span();
                let mut name = String::new();
                while let Some(c) = lx.peek() {
                    if c.is_ascii_alphanumeric() || c == '_' {
                        name.push(c);
                        lx.bump();
                    } else {
                        break;
                    }
                }
                let tok = match name.as_str() {
                    "let" => Token::Let,
                    "fn" => Token::Fn,
                    "if" => Token::If,
                    "else" => Token::Else,
                    "while" => Token::While,
                    "return" => Token::Return,
                    "true" => Token::True,
                    "false" => Token::False,
                    "as" => Token::As,
                    "use" => Token::Use,
                    _ => Token::Ident(name),
                };
                out.push((tok, start));
            }
            c => {
                let s = lx.span();
                return Err(CompileError::at(s, format!("unexpected character: {:?}", c)));
            }
        }
    }

    Ok(out)
}
