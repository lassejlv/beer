use beer_ast::*;
use beer_errors::CompileError;
use beer_lexer::Token;
use beer_span::Span;

pub fn parse(tokens: Vec<(Token, Span)>) -> Result<ParsedFile, CompileError> {
    let mut p = Parser { toks: tokens, pos: 0 };
    let mut uses = Vec::new();
    let mut funcs = Vec::new();

    while p.peek() == Some(&Token::Use) {
        uses.push(p.parse_use()?);
    }
    while !p.eof() {
        match p.peek() {
            Some(Token::Use) => {
                return Err(CompileError::at(
                    p.span_here(),
                    "`use` declarations must appear before any `fn`",
                ));
            }
            _ => funcs.push(p.parse_func()?),
        }
    }
    Ok(ParsedFile { uses, funcs })
}

struct Parser {
    toks: Vec<(Token, Span)>,
    pos: usize,
}

impl Parser {
    fn eof(&self) -> bool {
        self.pos >= self.toks.len()
    }

    fn peek(&self) -> Option<&Token> {
        self.toks.get(self.pos).map(|(t, _)| t)
    }

    fn peek_at(&self, offset: usize) -> Option<&Token> {
        self.toks.get(self.pos + offset).map(|(t, _)| t)
    }

    fn span_here(&self) -> Span {
        self.toks
            .get(self.pos)
            .map(|(_, s)| *s)
            .or_else(|| self.toks.last().map(|(_, s)| *s))
            .unwrap_or_else(|| Span::new(0, 1, 1))
    }

    fn error(&self, msg: impl Into<String>) -> CompileError {
        CompileError::at(self.span_here(), msg)
    }

    fn advance(&mut self) -> Option<(Token, Span)> {
        if self.eof() {
            None
        } else {
            let pair = self.toks[self.pos].clone();
            self.pos += 1;
            Some(pair)
        }
    }

    fn expect(&mut self, t: &Token) -> Result<Span, CompileError> {
        match self.peek() {
            Some(x) if x == t => {
                let s = self.span_here();
                self.pos += 1;
                Ok(s)
            }
            other => Err(self.error(format!(
                "expected {}, got {}",
                token_display(t),
                token_display_opt(other)
            ))),
        }
    }

    fn consume_ident(&mut self) -> Result<(String, Span), CompileError> {
        let span = self.span_here();
        match self.advance() {
            Some((Token::Ident(s), _)) => Ok((s, span)),
            other => Err(CompileError::at(
                span,
                format!(
                    "expected identifier, got {}",
                    token_display_opt(other.as_ref().map(|(t, _)| t))
                ),
            )),
        }
    }

    fn parse_type(&mut self) -> Result<Type, CompileError> {
        let span = self.span_here();
        match self.advance() {
            Some((Token::Ident(s), _)) => match s.as_str() {
                "int" => Ok(Type::Int),
                "float" => Ok(Type::Float),
                "bool" => Ok(Type::Bool),
                "str" => Ok(Type::Str),
                _ => Err(CompileError::at(span, format!("unknown type: {}", s))),
            },
            other => Err(CompileError::at(
                span,
                format!(
                    "expected type, got {}",
                    token_display_opt(other.as_ref().map(|(t, _)| t))
                ),
            )),
        }
    }

    fn parse_use(&mut self) -> Result<UseDecl, CompileError> {
        let span = self.expect(&Token::Use)?;
        let path_span = self.span_here();
        match self.advance() {
            Some((Token::Str(s), _)) => Ok(UseDecl { path: s, span }),
            other => Err(CompileError::at(
                path_span,
                format!(
                    "`use` requires a string path, got {}",
                    token_display_opt(other.as_ref().map(|(t, _)| t))
                ),
            )),
        }
    }

    fn parse_func(&mut self) -> Result<Func, CompileError> {
        let span = self.expect(&Token::Fn)?;
        let (name, _) = self.consume_ident()?;
        self.expect(&Token::LParen)?;
        let mut params = Vec::new();
        if self.peek() != Some(&Token::RParen) {
            loop {
                let (pname, _) = self.consume_ident()?;
                self.expect(&Token::Colon)?;
                let pty = self.parse_type()?;
                params.push((pname, pty));
                if self.peek() == Some(&Token::Comma) {
                    self.advance();
                } else {
                    break;
                }
            }
        }
        self.expect(&Token::RParen)?;
        let ret = if self.peek() == Some(&Token::Arrow) {
            self.advance();
            self.parse_type()?
        } else {
            Type::Void
        };
        let body = self.parse_block()?;
        Ok(Func { name, params, ret, body, span })
    }

    fn parse_block(&mut self) -> Result<Vec<Stmt>, CompileError> {
        let open_span = self.expect(&Token::LBrace)?;
        let mut stmts = Vec::new();
        while self.peek() != Some(&Token::RBrace) && !self.eof() {
            stmts.push(self.parse_stmt()?);
        }
        if self.eof() {
            return Err(CompileError::at(
                open_span,
                "unclosed `{` — missing matching `}`",
            ));
        }
        self.expect(&Token::RBrace)?;
        Ok(stmts)
    }

    fn parse_stmt(&mut self) -> Result<Stmt, CompileError> {
        let start = self.span_here();
        match self.peek() {
            Some(Token::Let) => {
                self.advance();
                let (name, _) = self.consume_ident()?;
                self.expect(&Token::Eq)?;
                let value = self.parse_expr()?;
                Ok(Stmt { kind: StmtKind::Let { name, value }, span: start })
            }
            Some(Token::If) => {
                self.advance();
                let cond = self.parse_expr()?;
                let then_block = self.parse_block()?;
                let else_block = if self.peek() == Some(&Token::Else) {
                    self.advance();
                    Some(self.parse_block()?)
                } else {
                    None
                };
                Ok(Stmt {
                    kind: StmtKind::If { cond, then_block, else_block },
                    span: start,
                })
            }
            Some(Token::While) => {
                self.advance();
                let cond = self.parse_expr()?;
                let body = self.parse_block()?;
                Ok(Stmt { kind: StmtKind::While { cond, body }, span: start })
            }
            Some(Token::Return) => {
                self.advance();
                let has_val = !matches!(self.peek(), Some(Token::RBrace) | None);
                let val = if has_val { Some(self.parse_expr()?) } else { None };
                Ok(Stmt { kind: StmtKind::Return(val), span: start })
            }
            Some(Token::Ident(_)) if self.peek_at(1) == Some(&Token::Eq) => {
                let (name, _) = self.consume_ident()?;
                self.expect(&Token::Eq)?;
                let value = self.parse_expr()?;
                Ok(Stmt { kind: StmtKind::Assign { name, value }, span: start })
            }
            _ => {
                let e = self.parse_expr()?;
                Ok(Stmt { kind: StmtKind::Expr(e), span: start })
            }
        }
    }

    fn parse_expr(&mut self) -> Result<Expr, CompileError> {
        self.parse_expr_bp(0)
    }

    fn parse_expr_bp(&mut self, min_bp: u8) -> Result<Expr, CompileError> {
        let mut lhs = self.parse_unary()?;
        loop {
            if self.peek() == Some(&Token::As) {
                let bp = 6;
                if bp < min_bp {
                    break;
                }
                self.advance();
                let target = self.parse_type()?;
                let span = lhs.span;
                lhs = Expr {
                    kind: ExprKind::Cast { expr: Box::new(lhs), target },
                    span,
                };
                continue;
            }
            if let Some((op, bp)) = self.peek().and_then(binop_info) {
                if bp < min_bp {
                    break;
                }
                self.advance();
                let rhs = self.parse_expr_bp(bp + 1)?;
                let span = lhs.span;
                lhs = Expr {
                    kind: ExprKind::Binary {
                        op,
                        lhs: Box::new(lhs),
                        rhs: Box::new(rhs),
                    },
                    span,
                };
                continue;
            }
            break;
        }
        Ok(lhs)
    }

    fn parse_unary(&mut self) -> Result<Expr, CompileError> {
        let span = self.span_here();
        match self.peek() {
            Some(Token::Minus) => {
                self.advance();
                let e = self.parse_unary()?;
                Ok(Expr { kind: ExprKind::Unary { op: UnOp::Neg, expr: Box::new(e) }, span })
            }
            Some(Token::Bang) => {
                self.advance();
                let e = self.parse_unary()?;
                Ok(Expr { kind: ExprKind::Unary { op: UnOp::Not, expr: Box::new(e) }, span })
            }
            _ => self.parse_primary(),
        }
    }

    fn parse_primary(&mut self) -> Result<Expr, CompileError> {
        let span = self.span_here();
        match self.advance() {
            Some((Token::Int(n), _)) => Ok(Expr { kind: ExprKind::Int(n), span }),
            Some((Token::Float(f), _)) => Ok(Expr { kind: ExprKind::Float(f), span }),
            Some((Token::True, _)) => Ok(Expr { kind: ExprKind::Bool(true), span }),
            Some((Token::False, _)) => Ok(Expr { kind: ExprKind::Bool(false), span }),
            Some((Token::Str(s), _)) => Ok(Expr { kind: ExprKind::Str(s), span }),
            Some((Token::LParen, _)) => {
                let e = self.parse_expr()?;
                self.expect(&Token::RParen)?;
                Ok(e)
            }
            Some((Token::Ident(name), _)) => {
                if self.peek() == Some(&Token::LParen) {
                    self.advance();
                    let mut args = Vec::new();
                    if self.peek() != Some(&Token::RParen) {
                        loop {
                            args.push(self.parse_expr()?);
                            if self.peek() == Some(&Token::Comma) {
                                self.advance();
                            } else {
                                break;
                            }
                        }
                    }
                    self.expect(&Token::RParen)?;
                    Ok(Expr { kind: ExprKind::Call { name, args }, span })
                } else {
                    Ok(Expr { kind: ExprKind::Ident(name), span })
                }
            }
            other => Err(CompileError::at(
                span,
                format!(
                    "unexpected {} in expression",
                    token_display_opt(other.as_ref().map(|(t, _)| t))
                ),
            )),
        }
    }
}

fn token_display(t: &Token) -> String {
    match t {
        Token::Let => "`let`".into(),
        Token::Fn => "`fn`".into(),
        Token::If => "`if`".into(),
        Token::Else => "`else`".into(),
        Token::While => "`while`".into(),
        Token::Return => "`return`".into(),
        Token::True => "`true`".into(),
        Token::False => "`false`".into(),
        Token::As => "`as`".into(),
        Token::Use => "`use`".into(),
        Token::Int(_) => "integer literal".into(),
        Token::Float(_) => "float literal".into(),
        Token::Str(_) => "string literal".into(),
        Token::Ident(s) => format!("`{}`", s),
        Token::LParen => "`(`".into(),
        Token::RParen => "`)`".into(),
        Token::LBrace => "`{`".into(),
        Token::RBrace => "`}`".into(),
        Token::Comma => "`,`".into(),
        Token::Colon => "`:`".into(),
        Token::Arrow => "`->`".into(),
        Token::Eq => "`=`".into(),
        Token::EqEq => "`==`".into(),
        Token::BangEq => "`!=`".into(),
        Token::Bang => "`!`".into(),
        Token::Lt => "`<`".into(),
        Token::LtEq => "`<=`".into(),
        Token::Gt => "`>`".into(),
        Token::GtEq => "`>=`".into(),
        Token::AndAnd => "`&&`".into(),
        Token::OrOr => "`||`".into(),
        Token::Plus => "`+`".into(),
        Token::Minus => "`-`".into(),
        Token::Star => "`*`".into(),
        Token::Slash => "`/`".into(),
    }
}

fn token_display_opt(t: Option<&Token>) -> String {
    match t {
        Some(t) => token_display(t),
        None => "end of file".into(),
    }
}

fn binop_info(t: &Token) -> Option<(BinOp, u8)> {
    Some(match t {
        Token::OrOr => (BinOp::Or, 1),
        Token::AndAnd => (BinOp::And, 2),
        Token::EqEq => (BinOp::Eq, 3),
        Token::BangEq => (BinOp::Ne, 3),
        Token::Lt => (BinOp::Lt, 3),
        Token::LtEq => (BinOp::Le, 3),
        Token::Gt => (BinOp::Gt, 3),
        Token::GtEq => (BinOp::Ge, 3),
        Token::Plus => (BinOp::Add, 4),
        Token::Minus => (BinOp::Sub, 4),
        Token::Star => (BinOp::Mul, 5),
        Token::Slash => (BinOp::Div, 5),
        _ => return None,
    })
}
