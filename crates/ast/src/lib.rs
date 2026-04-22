use beer_span::Span;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Type {
    Int,
    Float,
    Bool,
    Str,
    Void,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BinOp {
    Add, Sub, Mul, Div,
    Eq, Ne, Lt, Le, Gt, Ge,
    And, Or,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum UnOp {
    Neg,
    Not,
}

#[derive(Debug, Clone)]
pub struct Expr {
    pub kind: ExprKind,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub enum ExprKind {
    Int(i64),
    Float(f64),
    Bool(bool),
    Str(String),
    Ident(String),
    Binary { op: BinOp, lhs: Box<Expr>, rhs: Box<Expr> },
    Unary { op: UnOp, expr: Box<Expr> },
    Call { name: String, args: Vec<Expr> },
    Cast { expr: Box<Expr>, target: Type },
}

#[derive(Debug, Clone)]
pub struct Stmt {
    pub kind: StmtKind,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub enum StmtKind {
    Let { name: String, value: Expr },
    Assign { name: String, value: Expr },
    If {
        cond: Expr,
        then_block: Vec<Stmt>,
        else_block: Option<Vec<Stmt>>,
    },
    While { cond: Expr, body: Vec<Stmt> },
    Return(Option<Expr>),
    Expr(Expr),
}

#[derive(Debug, Clone)]
pub struct Func {
    pub name: String,
    pub params: Vec<(String, Type)>,
    pub ret: Type,
    pub body: Vec<Stmt>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct UseDecl {
    pub path: String,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct ParsedFile {
    pub uses: Vec<UseDecl>,
    pub funcs: Vec<Func>,
}

#[derive(Debug, Clone)]
pub struct Program {
    pub funcs: Vec<Func>,
}
