use crate::lexer::Tok;

#[derive(Clone, PartialEq)]
pub enum Ty {
    Int,
    Long,
    Bool,
    Str,
    Void,
    Arr(Box<Ty>, usize),
}

impl std::fmt::Display for Ty {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str(match self {
            Ty::Int => "int",
            Ty::Long => "long",
            Ty::Bool => "bool",
            Ty::Str => "string",
            Ty::Void => "void",
            Ty::Arr(t, n) => return write!(f, "{t}[{n}]"),
        })
    }
}

pub enum Expr {
    Int(i64),
    Str(String),
    Bool(bool),
    Var(String),
    Index(String, Box<Expr>),
    Call(String, Vec<Expr>),
    Un(&'static str, Box<Expr>),
    Bin(&'static str, Box<Expr>, Box<Expr>),
}

pub enum Stmt {
    Let(Ty, String, Option<Expr>),
    Assign(Expr, Expr),
    Expr(Expr),
    If(Expr, Box<Line>, Option<Box<Line>>),
    While(Expr, Box<Line>),
    Ret(Option<Expr>),
    Block(Vec<Line>),
    Unsafe(Vec<Line>),
    Asm(Vec<Vec<Tok>>),
}

pub type Line = (usize, Stmt);

pub struct Func {
    pub name: String,
    pub ret: Ty,
    pub params: Vec<(Ty, String)>,
    pub body: Vec<Line>,
    pub lib: Option<String>,
    pub line: usize,
    pub file: String,
}

const KW: [&str; 15] = [
    "int", "long", "bool", "string", "void", "if", "else", "while", "return", "extern", "unsafe", "true", "false", "asm", "import",
];
const PREC: [&[&str]; 6] =
    [&["||"], &["&&"], &["==", "!="], &["<", "<=", ">", ">="], &["+", "-"], &["*", "/", "%"]];

struct P {
    t: Vec<(Tok, usize)>,
    i: usize,
}

impl P {
    fn peek(&self) -> &Tok {
        &self.t[self.i].0
    }
    fn line(&self) -> usize {
        self.t[self.i].1
    }
    fn next(&mut self) -> Tok {
        let t = self.t[self.i].0.clone();
        self.i = (self.i + 1).min(self.t.len() - 1);
        t
    }
    fn err<T>(&self, m: &str) -> Result<T, String> {
        Err(format!("{}: {m}", self.line()))
    }
    fn is(&self, s: &str) -> bool {
        matches!(self.peek(), Tok::Sym(x) if *x == s)
    }
    fn eat(&mut self, s: &str) -> bool {
        let r = self.is(s);
        if r {
            self.i += 1;
        }
        r
    }
    fn expect(&mut self, s: &str) -> Result<(), String> {
        if self.eat(s) { Ok(()) } else { self.err(&format!("expected '{s}'")) }
    }
    fn kw(&mut self, k: &str) -> bool {
        let r = matches!(self.peek(), Tok::Id(x) if x == k);
        if r {
            self.i += 1;
        }
        r
    }
    fn ident(&mut self) -> Result<String, String> {
        match self.peek().clone() {
            Tok::Id(s) if !KW.contains(&s.as_str()) => {
                self.i += 1;
                Ok(s)
            }
            _ => self.err("expected identifier"),
        }
    }
    fn base_ty(&self) -> Option<Ty> {
        let Tok::Id(s) = self.peek() else { return None };
        Some(match s.as_str() {
            "int" => Ty::Int,
            "long" => Ty::Long,
            "bool" => Ty::Bool,
            "string" => Ty::Str,
            "void" => Ty::Void,
            _ => return None,
        })
    }
    fn ty(&mut self) -> Result<Ty, String> {
        let Some(t) = self.base_ty() else { return self.err("expected type") };
        self.i += 1;
        if !self.eat("[") {
            return Ok(t);
        }
        let Tok::Int(n) = self.next() else { return self.err("expected array size") };
        self.expect("]")?;
        if n <= 0 || !matches!(t, Ty::Int | Ty::Long | Ty::Bool) {
            return self.err("arrays need a size > 0 and int, long or bool elements");
        }
        Ok(Ty::Arr(Box::new(t), n as usize))
    }
    fn scalar(&mut self) -> Result<Ty, String> {
        let t = self.ty()?;
        if matches!(t, Ty::Arr(..)) {
            return self.err("arrays can't be parameters or return values yet");
        }
        Ok(t)
    }
    fn func(&mut self) -> Result<Func, String> {
        let line = self.line();
        let lib = if self.kw("extern") {
            let Tok::Str(s) = self.next() else { return self.err("expected library name string") };
            Some(s)
        } else {
            None
        };
        let ret = self.scalar()?;
        let name = self.ident()?;
        self.expect("(")?;
        let params = self.list(|p| {
            let t = p.scalar()?;
            if t == Ty::Void {
                return p.err("parameter can't be void");
            }
            Ok((t, p.ident()?))
        })?;
        let body = if lib.is_some() {
            self.expect(";")?;
            vec![]
        } else {
            self.block()?
        };
        Ok(Func { name, ret, params, body, lib, line, file: String::new() })
    }
    /// comma list up to ) after ( is eaten
    fn list<T>(&mut self, mut item: impl FnMut(&mut Self) -> Result<T, String>) -> Result<Vec<T>, String> {
        let mut v = vec![];
        if self.eat(")") {
            return Ok(v);
        }
        loop {
            v.push(item(self)?);
            if self.eat(")") {
                return Ok(v);
            }
            self.expect(",")?;
        }
    }
    fn block(&mut self) -> Result<Vec<Line>, String> {
        self.expect("{")?;
        let mut v = vec![];
        while !self.eat("}") {
            if *self.peek() == Tok::Eof {
                return self.err("expected '}'");
            }
            v.push(self.stmt()?);
        }
        Ok(v)
    }
    fn stmt(&mut self) -> Result<Line, String> {
        let line = self.line();
        let s = if self.is("{") {
            Stmt::Block(self.block()?)
        } else if self.kw("unsafe") {
            Stmt::Unsafe(self.block()?)
        } else if self.kw("asm") {
            self.expect("{")?;
            let mut v = vec![vec![]];
            while !self.eat("}") {
                if *self.peek() == Tok::Eof {
                    return self.err("expected '}'");
                }
                if self.eat(";") {
                    v.push(vec![]);
                } else {
                    v.last_mut().unwrap().push(self.next());
                }
            }
            v.retain(|l| !l.is_empty());
            Stmt::Asm(v)
        } else if self.kw("if") {
            let c = self.paren()?;
            let t = Box::new(self.stmt()?);
            let e = if self.kw("else") { Some(Box::new(self.stmt()?)) } else { None };
            Stmt::If(c, t, e)
        } else if self.kw("while") {
            Stmt::While(self.paren()?, Box::new(self.stmt()?))
        } else if self.kw("return") {
            let e = if self.is(";") { None } else { Some(self.expr()?) };
            self.expect(";")?;
            Stmt::Ret(e)
        } else if self.base_ty().is_some() {
            let t = self.ty()?;
            let n = self.ident()?;
            let e = if self.eat("=") { Some(self.expr()?) } else { None };
            self.expect(";")?;
            Stmt::Let(t, n, e)
        } else {
            let e = self.expr()?;
            let s = if self.eat("=") { Stmt::Assign(e, self.expr()?) } else { Stmt::Expr(e) };
            self.expect(";")?;
            s
        };
        Ok((line, s))
    }
    fn paren(&mut self) -> Result<Expr, String> {
        self.expect("(")?;
        let e = self.expr()?;
        self.expect(")")?;
        Ok(e)
    }
    fn expr(&mut self) -> Result<Expr, String> {
        self.bin(0)
    }
    fn bin(&mut self, lvl: usize) -> Result<Expr, String> {
        if lvl == PREC.len() {
            return self.unary();
        }
        let mut l = self.bin(lvl + 1)?;
        while let Tok::Sym(op) = *self.peek() {
            if !PREC[lvl].contains(&op) {
                break;
            }
            self.i += 1;
            l = Expr::Bin(op, Box::new(l), Box::new(self.bin(lvl + 1)?));
        }
        Ok(l)
    }
    fn unary(&mut self) -> Result<Expr, String> {
        for op in ["-", "!"] {
            if self.eat(op) {
                return Ok(Expr::Un(op, Box::new(self.unary()?)));
            }
        }
        self.primary()
    }
    fn primary(&mut self) -> Result<Expr, String> {
        if self.is("(") {
            return self.paren();
        }
        let line = self.line();
        Ok(match self.next() {
            Tok::Int(n) => Expr::Int(n),
            Tok::Str(s) => Expr::Str(s),
            Tok::Id(s) if s == "true" || s == "false" => Expr::Bool(s == "true"),
            Tok::Id(s) if !KW.contains(&s.as_str()) => {
                if self.eat("(") {
                    Expr::Call(s, self.list(Self::expr)?)
                } else if self.eat("[") {
                    let i = self.expr()?;
                    self.expect("]")?;
                    Expr::Index(s, Box::new(i))
                } else {
                    Expr::Var(s)
                }
            }
            _ => return Err(format!("{line}: expected expression")),
        })
    }
}

/// returns imports and functions
pub fn parse(t: Vec<(Tok, usize)>) -> Result<(Vec<String>, Vec<Func>), String> {
    let mut p = P { t, i: 0 };
    let (mut imports, mut fs) = (vec![], vec![]);
    while *p.peek() != Tok::Eof {
        if p.kw("import") {
            let Tok::Str(s) = p.next() else { return p.err("expected file name string") };
            p.expect(";")?;
            imports.push(s);
        } else {
            fs.push(p.func()?);
        }
    }
    Ok((imports, fs))
}
