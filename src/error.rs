use std::fmt;

use crate::span::Span;

#[derive(Debug)]
pub struct CompileError {
    pub span: Option<Span>,
    pub msg: String,
}

impl CompileError {
    pub fn new(msg: impl Into<String>) -> Self {
        Self { span: None, msg: msg.into() }
    }

    pub fn at(span: Span, msg: impl Into<String>) -> Self {
        Self { span: Some(span), msg: msg.into() }
    }
}

impl fmt::Display for CompileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.span {
            Some(s) => write!(f, "{}:{}: error: {}", s.line, s.col, self.msg),
            None => write!(f, "error: {}", self.msg),
        }
    }
}

impl From<String> for CompileError {
    fn from(s: String) -> Self {
        Self::new(s)
    }
}

impl From<&str> for CompileError {
    fn from(s: &str) -> Self {
        Self::new(s)
    }
}
