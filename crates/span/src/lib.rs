#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Span {
    pub file: u32,
    pub line: u32,
    pub col: u32,
}

impl Span {
    pub fn new(file: u32, line: u32, col: u32) -> Self {
        Self { file, line, col }
    }
}
