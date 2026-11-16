use crate::asm::*;
use crate::lexer::Tok;
use crate::parser::{Expr, Func, Line, Stmt, Ty};
use std::collections::{HashMap, HashSet};
use std::fmt::Display;
use std::slice::from_ref;

pub struct Out {
    pub code: Vec<u8>,
    pub data: Vec<u8>,
    pub imports: Vec<(String, String)>,
    pub ext: Vec<(usize, Target)>,
}

struct G<'a> {
    a: Asm,
    data: Vec<u8>,
    strs: HashMap<String, usize>,
    imports: Vec<(String, String)>,
    fns: HashMap<&'a str, (&'a Func, usize)>,
    todo: Vec<&'a Func>,
    queued: HashSet<&'a str>,
    win: bool,
    line: usize,
    file: &'a str,
    scopes: Vec<HashMap<String, (i32, Ty)>>,
    frame: i32,
    ret: Ty,
    exit: usize,
    unsafe_: u32,
    write: usize,
    pstr: usize,
    pint: usize,
    oob: usize,
    div0: usize,
    ovf: usize,
    die: usize,
    quit: usize,
}

const REGS: [&str; 16] =
    ["rax", "rcx", "rdx", "rbx", "rsp", "rbp", "rsi", "rdi", "r8", "r9", "r10", "r11", "r12", "r13", "r14", "r15"];
const ALU: [(&str, u8, u8); 7] =
    [("add", 0x01, 0), ("or", 0x09, 1), ("and", 0x21, 4), ("sub", 0x29, 5), ("xor", 0x31, 6), ("cmp", 0x39, 7), ("mov", 0x89, 0)];
const SXD: &[u8] = &[0x48, 0x63, 0xC0]; // movsxd rax eax

enum Op {
    R(u8),
    M(u8, i32),
    I(i64),
}

impl Op {
    fn rm(&self) -> Option<Rm> {
        match *self {
            Op::R(r) => Some(Rm::Reg(r)),
            Op::M(b, d) => Some(Rm::Mem(b, d)),
            Op::I(_) => None,
        }
    }
}

fn reg(s: &str) -> Option<u8> {
    REGS.iter().position(|r| *r == s).map(|r| r as u8)
}

fn num(t: &Ty) -> bool {
    matches!(t, Ty::Int | Ty::Long)
}

fn lit(n: i64) -> Ty {
    if i32::try_from(n).is_ok() { Ty::Int } else { Ty::Long }
}

fn returns(b: &[Line]) -> bool {
    b.last().is_some_and(|(_, s)| match s {
        Stmt::Ret(_) => true,
        Stmt::Block(b) | Stmt::Unsafe(b) => returns(b),
        Stmt::If(_, t, Some(e)) => returns(from_ref(&**t)) && returns(from_ref(&**e)),
        _ => false,
    })
}

/// upper bound on local slots
fn slots(b: &[Line]) -> usize {
    b.iter().fold(0, |n: usize, (_, s)| {
        n.saturating_add(match s {
            Stmt::Let(Ty::Arr(_, k), ..) => *k,
            Stmt::Let(..) => 1,
            Stmt::Block(b) | Stmt::Unsafe(b) => slots(b),
            Stmt::If(_, t, e) => slots(from_ref(&**t)).saturating_add(e.as_ref().map_or(0, |e| slots(from_ref(&**e)))),
            Stmt::While(_, t) => slots(from_ref(&**t)),
            _ => 0,
        })
    })
}

/// call an import with rsp aligned and shadow space
fn ccall(a: &mut Asm, imp: usize) {
    a.mov_rr(R12, RSP);
    a.b(&[0x48, 0x83, 0xE4, 0xF0, 0x48, 0x83, 0xEC, 0x20, 0x31, 0xC0]); // al 0 for sysv varargs
    a.ext(&[0xFF, 0x15], Target::Import(imp));
    a.mov_rr(RSP, R12);
}

pub fn compile(fs: &[Func], win: bool) -> Result<Out, String> {
    let mut a = Asm::default();
    let [write, pstr, pint, oob, div0, ovf, die, quit] = std::array::from_fn(|_| a.label());
    let mut g = G {
        a,
        data: vec![],
        strs: HashMap::new(),
        imports: vec![],
        fns: HashMap::new(),
        todo: vec![],
        queued: HashSet::new(),
        win,
        line: 0,
        file: "",
        scopes: vec![],
        frame: 0,
        ret: Ty::Void,
        exit: 0,
        unsafe_: 0,
        write,
        pstr,
        pint,
        oob,
        div0,
        ovf,
        die,
        quit,
    };
    let at = |f: &Func, m: &str| Err(format!("{}:{}: '{}' {m}", f.file, f.line, f.name));
    for f in fs {
        let l = g.a.label();
        if ["print", "println", "panic"].contains(&f.name.as_str()) || g.fns.insert(&f.name, (f, l)).is_some() {
            return at(f, "is already defined");
        }
        if f.lib.is_none() && f.ret != Ty::Void && !returns(&f.body) {
            return at(f, "doesn't return a value on every path");
        }
        if f.params.iter().enumerate().any(|(i, p)| f.params[..i].iter().any(|q| q.1 == p.1)) {
            return at(f, "has duplicate parameter names");
        }
        // std is emitted only when called
        if f.lib.is_none() && f.file != "std" {
            g.queued.insert(&f.name);
            g.todo.push(f);
        }
    }
    let Some(&(m, ml)) = g.fns.get("main") else { return Err("no main function".into()) };
    if !m.params.is_empty() || !matches!(m.ret, Ty::Void | Ty::Int) || m.lib.is_some() {
        return at(m, "must be 'void main()' or 'int main()'");
    }
    g.a.j(CALL, ml);
    g.a.mov_rr(RCX, RAX);
    g.a.j(JMP, quit);
    while let Some(f) = g.todo.pop() {
        g.func(f)?;
    }
    g.runtime();
    g.a.finish();
    Ok(Out { code: g.a.code, data: g.data, imports: g.imports, ext: g.a.ext })
}

impl<'a> G<'a> {
    fn err<T>(&self, m: impl Display) -> Result<T, String> {
        Err(format!("{}:{}: {m}", self.file, self.line))
    }

    fn fit(&self, d: &Ty, s: &Ty) -> Result<(), String> {
        if d == s || (*d == Ty::Long && *s == Ty::Int) { Ok(()) } else { self.err(format!("can't assign {s} to {d}")) }
    }

    fn str(&mut self, s: &str) -> Target {
        let d = &mut self.data;
        Target::Data(*self.strs.entry(s.to_string()).or_insert_with(|| {
            let o = d.len();
            d.extend(s.as_bytes());
            d.push(0);
            o
        }))
    }

    fn print_str(&mut self, s: &str) {
        let s = self.str(s);
        self.a.ext(LEA, s);
        self.a.j(CALL, self.pstr);
    }

    fn import(&mut self, lib: &str, name: &str) -> usize {
        let k = (lib.to_string(), name.to_string());
        self.imports.iter().position(|x| *x == k).unwrap_or_else(|| {
            self.imports.push(k);
            self.imports.len() - 1
        })
    }

    fn var(&self, n: &str) -> Result<(i32, Ty), String> {
        match self.scopes.iter().rev().find_map(|s| s.get(n).cloned()) {
            Some(v) => Ok(v),
            None => self.err(format!("unknown variable '{n}'")),
        }
    }

    /// overflow-checked op with int in 32 bits
    fn op(&mut self, code: &[u8], t: &Ty) {
        let int = *t == Ty::Int;
        self.a.b(if int { &code[1..] } else { code });
        self.a.j(JO, self.ovf);
        if int {
            self.a.b(SXD);
        }
    }

    fn func(&mut self, f: &'a Func) -> Result<(), String> {
        (self.file, self.line) = (&f.file, f.line);
        let sz = slots(&f.body).saturating_mul(8);
        if sz > 1 << 28 {
            return self.err("stack frame too large");
        }
        let sz = (sz as i32 + 15) & !15;
        let pages = if sz > 4096 { sz / 4096 } else { 0 };
        self.a.bind(self.fns[f.name.as_str()].1);
        self.a.push(RBP);
        self.a.mov_rr(RBP, RSP);
        if pages > 0 {
            // touch each page so guard pages hit in order
            self.a.imm(RCX, pages as i64);
            let top = self.a.label();
            self.a.bind(top);
            self.a.b(&[0x48, 0x81, 0xEC, 0, 0x10, 0, 0, 0x48, 0x83, 0x0C, 0x24, 0, 0xFF, 0xC9]);
            self.a.j(JNZ, top);
        }
        if sz > pages * 4096 {
            self.a.b(&[0x48, 0x81, 0xEC]);
            self.a.b(&(sz - pages * 4096).to_le_bytes());
        }
        let n = f.params.len();
        let sc = f.params.iter().enumerate().map(|(i, (t, p))| (p.clone(), ((16 + 8 * (n - 1 - i)) as i32, t.clone())));
        self.scopes = vec![sc.collect()];
        (self.frame, self.ret, self.exit) = (0, f.ret.clone(), self.a.label());
        self.block(&f.body)?;
        self.a.b(&[0x31, 0xC0]);
        self.a.bind(self.exit);
        self.a.b(&[0xC9, 0xC3]); // leave ret
        Ok(())
    }

    fn block(&mut self, b: &[Line]) -> Result<(), String> {
        self.scopes.push(HashMap::new());
        let f = self.frame;
        for s in b {
            self.stmt(s)?;
        }
        self.frame = f;
        self.scopes.pop();
        Ok(())
    }

    /// jump to l if e is when
    fn branch(&mut self, e: &Expr, when: bool, l: usize) -> Result<(), String> {
        match e {
            Expr::Un("!", x) => return self.branch(x, !when, l),
            Expr::Bin(op @ ("&&" | "||"), x, y) => {
                if (*op == "&&") != when {
                    self.branch(x, when, l)?;
                    self.branch(y, when, l)?;
                } else {
                    let skip = self.a.label();
                    self.branch(x, !when, skip)?;
                    self.branch(y, when, l)?;
                    self.a.bind(skip);
                }
                return Ok(());
            }
            Expr::Bin(op, x, y) => {
                if let Some(cc) = self.compare(op, x, y)? {
                    self.a.j(&[0x0F, (cc - 0x10) ^ !when as u8], l);
                    return Ok(());
                }
            }
            _ => {}
        }
        if self.expr(e)? != Ty::Bool {
            return self.err("condition must be bool");
        }
        self.a.b(&[0x48, 0x85, 0xC0]);
        self.a.j(if when { JNZ } else { JZ }, l);
        Ok(())
    }

    /// lhs in rax rhs in rcx
    fn operands(&mut self, l: &Expr, r: &Expr) -> Result<(Ty, Ty), String> {
        let lt = self.expr(l)?;
        let rt = match r {
            Expr::Int(n) => {
                self.a.imm(RCX, *n);
                lit(*n)
            }
            Expr::Var(n) if !matches!(self.var(n)?.1, Ty::Arr(..)) => {
                let (o, t) = self.var(n)?;
                self.a.mem(0x8B, RCX, RBP, o);
                t
            }
            _ => {
                self.a.push(RAX);
                let t = self.expr(r)?;
                self.a.mov_rr(RCX, RAX);
                self.a.pop(RAX);
                t
            }
        };
        Ok((lt, rt))
    }

    /// cmp for comparisons and returns the setcc opcode
    fn compare(&mut self, op: &str, l: &Expr, r: &Expr) -> Result<Option<u8>, String> {
        let cc = match op {
            "==" => 0x94,
            "!=" => 0x95,
            "<" => 0x9C,
            "<=" => 0x9E,
            ">" => 0x9F,
            ">=" => 0x9D,
            _ => return Ok(None),
        };
        let (lt, rt) = self.operands(l, r)?;
        if !(num(&lt) && num(&rt) || cc < 0x96 && lt == Ty::Bool && rt == Ty::Bool) {
            return self.err(format!("can't apply '{op}' to {lt} and {rt}"));
        }
        self.a.b(&[0x48, 0x39, 0xC8]);
        Ok(Some(cc))
    }

    fn stmt(&mut self, (line, s): &Line) -> Result<(), String> {
        self.line = *line;
        match s {
            Stmt::Block(b) => self.block(b)?,
            Stmt::Unsafe(b) => {
                self.unsafe_ += 1;
                self.block(b)?;
                self.unsafe_ -= 1;
            }
            Stmt::Let(t, n, e) => {
                if *t == Ty::Void {
                    return self.err("variable can't be void");
                }
                if self.scopes.last().unwrap().contains_key(n) {
                    return self.err(format!("'{n}' already declared"));
                }
                self.frame += 8 * if let Ty::Arr(_, k) = t { *k as i32 } else { 1 };
                let o = -self.frame;
                match (t, e) {
                    (Ty::Arr(..), Some(_)) => return self.err("array initializers aren't supported yet"),
                    (Ty::Arr(_, k), None) => {
                        self.a.mem(0x8D, RDI, RBP, o);
                        self.a.imm(RCX, *k as i64);
                        self.a.b(&[0x31, 0xC0, 0xF3, 0x48, 0xAB]); // rep stosq
                    }
                    (_, e) => {
                        match e {
                            Some(e) => {
                                let et = self.expr(e)?;
                                self.fit(t, &et)?;
                            }
                            None if *t == Ty::Str => {
                                let s = self.str("");
                                self.a.ext(LEA, s);
                            }
                            None => self.a.b(&[0x31, 0xC0]),
                        }
                        self.a.mem(0x89, RAX, RBP, o);
                    }
                }
                self.scopes.last_mut().unwrap().insert(n.clone(), (o, t.clone()));
            }
            Stmt::Assign(target, v) => match target {
                Expr::Var(n) => {
                    let (o, t) = self.var(n)?;
                    let vt = self.expr(v)?;
                    self.fit(&t, &vt)?;
                    self.a.mem(0x89, RAX, RBP, o);
                }
                Expr::Index(n, i) => {
                    let vt = self.expr(v)?;
                    self.a.push(RAX);
                    let et = self.elem(n, i)?;
                    self.fit(&et, &vt)?;
                    self.a.pop(RDX);
                    self.a.b(&[0x48, 0x89, 0x14, 0xC1]); // mov [rcx+rax*8] rdx
                }
                _ => return self.err("invalid assignment target"),
            },
            Stmt::Expr(e) => {
                self.expr(e)?;
            }
            Stmt::Asm(lines) => {
                if self.unsafe_ == 0 {
                    return self.err("asm requires an unsafe block");
                }
                for l in lines {
                    self.asm_line(l)?;
                }
            }
            Stmt::If(c, t, e) => {
                let els = self.a.label();
                self.branch(c, false, els)?;
                self.stmt(t)?;
                match e {
                    Some(e) => {
                        let end = self.a.label();
                        self.a.j(JMP, end);
                        self.a.bind(els);
                        self.stmt(e)?;
                        self.a.bind(end);
                    }
                    None => self.a.bind(els),
                }
            }
            Stmt::While(c, body) => {
                // test at the bottom so each pass takes one jump
                let (top, test) = (self.a.label(), self.a.label());
                self.a.j(JMP, test);
                self.a.bind(top);
                self.stmt(body)?;
                self.a.bind(test);
                self.line = *line;
                self.branch(c, true, top)?;
            }
            Stmt::Ret(e) => {
                match e {
                    None if self.ret == Ty::Void => self.a.b(&[0x31, 0xC0]),
                    Some(e) if self.ret != Ty::Void => {
                        let t = self.expr(e)?;
                        self.fit(&self.ret, &t)?;
                    }
                    _ => return self.err("return value doesn't match function type"),
                }
                self.a.j(JMP, self.exit);
            }
        }
        Ok(())
    }

    /// reg imm [reg] [reg+n] or [var]
    fn operand(&self, t: &[Tok]) -> Result<Op, String> {
        Ok(match t {
            [Tok::Id(r)] if reg(r).is_some() => Op::R(reg(r).unwrap()),
            [Tok::Int(v)] => Op::I(*v),
            [Tok::Sym("-"), Tok::Int(v)] => Op::I(v.wrapping_neg()),
            [Tok::Sym("["), Tok::Id(n), Tok::Sym("]")] => match reg(n) {
                Some(r) => Op::M(r, 0),
                None => Op::M(RBP, self.var(n)?.0),
            },
            [Tok::Sym("["), Tok::Id(r), Tok::Sym(s @ ("+" | "-")), Tok::Int(d), Tok::Sym("]")] if reg(r).is_some() => {
                match i32::try_from(if *s == "-" { d.wrapping_neg() } else { *d }) {
                    Ok(d) => Op::M(reg(r).unwrap(), d),
                    Err(_) => return self.err("displacement too large"),
                }
            }
            _ => return self.err("invalid asm operand"),
        })
    }

    fn asm_line(&mut self, l: &[Tok]) -> Result<(), String> {
        let Tok::Id(m) = &l[0] else { return self.err("expected asm instruction") };
        let ops = match l.len() {
            1 => vec![],
            _ => l[1..].split(|t| *t == Tok::Sym(",")).map(|o| self.operand(o)).collect::<Result<Vec<_>, _>>()?,
        };
        let alu = ALU.iter().find(|x| x.0 == m);
        let a = &mut self.a;
        match (m.as_str(), ops.as_slice()) {
            ("syscall", []) => a.b(&[0x0F, 0x05]),
            ("nop", []) => a.b(&[0x90]),
            ("int3", []) => a.b(&[0xCC]),
            ("cqo", []) => a.b(&[0x48, 0x99]),
            ("db", [_, ..]) => {
                for o in &ops {
                    match o {
                        Op::I(v @ 0..=255) => a.b(&[*v as u8]),
                        _ => return self.err("db takes bytes 0..255"),
                    }
                }
            }
            ("push", [Op::R(r)]) => a.push(*r),
            ("pop", [Op::R(r)]) => a.pop(*r),
            ("lea", [Op::R(r), m @ Op::M(..)]) => a.modrm(&[0x8D], *r, m.rm().unwrap()),
            ("imul", [Op::R(r), s]) if s.rm().is_some() => a.modrm(&[0x0F, 0xAF], *r, s.rm().unwrap()),
            (u @ ("inc" | "dec" | "not" | "neg"), [d]) if d.rm().is_some() => {
                let (op, ext) = match u {
                    "inc" => (0xFF, 0),
                    "dec" => (0xFF, 1),
                    "not" => (0xF7, 2),
                    _ => (0xF7, 3),
                };
                a.modrm(&[op], ext, d.rm().unwrap());
            }
            (_, [d, s]) if alu.is_some() => {
                let &(_, op, ext) = alu.unwrap();
                match (d, s) {
                    (_, Op::R(s)) if d.rm().is_some() => a.modrm(&[op], *s, d.rm().unwrap()),
                    (Op::R(d), Op::M(b, x)) => a.modrm(&[op + 2], *d, Rm::Mem(*b, *x)),
                    (Op::R(r), Op::I(v)) if m == "mov" => a.imm(*r, *v),
                    (_, Op::I(v)) if d.rm().is_some() && i32::try_from(*v).is_ok() => {
                        if m == "mov" { a.modrm(&[0xC7], 0, d.rm().unwrap()) } else { a.modrm(&[0x81], ext, d.rm().unwrap()) }
                        a.b(&(*v as i32).to_le_bytes());
                    }
                    _ => return self.err(format!("invalid operands for '{m}'")),
                }
            }
            _ => return self.err(format!("unsupported asm instruction '{m}'")),
        }
        Ok(())
    }

    /// bounds-checked and leaves rcx base rax index
    fn elem(&mut self, n: &str, i: &Expr) -> Result<Ty, String> {
        let (o, t) = self.var(n)?;
        let Ty::Arr(et, len) = t else { return self.err(format!("'{n}' is not an array")) };
        if !num(&self.expr(i)?) {
            return self.err("array index must be an integer");
        }
        self.a.b(&[0x48, 0x3D]);
        self.a.b(&(len as i32).to_le_bytes());
        self.a.j(JAE, self.oob);
        self.a.mem(0x8D, RCX, RBP, o);
        Ok(*et)
    }

    fn expr(&mut self, e: &Expr) -> Result<Ty, String> {
        Ok(match e {
            Expr::Int(n) => {
                self.a.imm(RAX, *n);
                lit(*n)
            }
            Expr::Bool(b) => {
                self.a.imm(RAX, *b as i64);
                Ty::Bool
            }
            Expr::Str(s) => {
                let s = self.str(s);
                self.a.ext(LEA, s);
                Ty::Str
            }
            Expr::Var(n) => {
                let (o, t) = self.var(n)?;
                if matches!(t, Ty::Arr(..)) {
                    return self.err(format!("array '{n}' can't be used as a value"));
                }
                self.a.mem(0x8B, RAX, RBP, o);
                t
            }
            Expr::Index(n, i) => {
                let t = self.elem(n, i)?;
                self.a.b(&[0x48, 0x8B, 0x04, 0xC1]); // mov rax [rcx+rax*8]
                t
            }
            Expr::Un("-", x) => {
                let t = self.expr(x)?;
                if !num(&t) {
                    return self.err("'-' needs a number");
                }
                self.op(&[0x48, 0xF7, 0xD8], &t);
                t
            }
            Expr::Un(_, x) => {
                if self.expr(x)? != Ty::Bool {
                    return self.err("'!' needs a bool");
                }
                self.a.b(&[0x83, 0xF0, 0x01]);
                Ty::Bool
            }
            Expr::Bin("&&" | "||", ..) => {
                let (f, end) = (self.a.label(), self.a.label());
                self.branch(e, false, f)?;
                self.a.imm(RAX, 1);
                self.a.j(JMP, end);
                self.a.bind(f);
                self.a.b(&[0x31, 0xC0]);
                self.a.bind(end);
                Ty::Bool
            }
            Expr::Bin(op, l, r) => {
                if let Some(cc) = self.compare(op, l, r)? {
                    self.a.b(&[0x0F, cc, 0xC0, 0x0F, 0xB6, 0xC0]); // setcc al movzx eax al
                    return Ok(Ty::Bool);
                }
                let (lt, rt) = self.operands(l, r)?;
                if !num(&lt) || !num(&rt) {
                    return self.err(format!("can't apply '{op}' to {lt} and {rt}"));
                }
                let res = if lt == Ty::Long || rt == Ty::Long { Ty::Long } else { Ty::Int };
                match *op {
                    "+" => self.op(&[0x48, 0x01, 0xC8], &res),
                    "-" => self.op(&[0x48, 0x29, 0xC8], &res),
                    "*" => self.op(&[0x48, 0x0F, 0xAF, 0xC1], &res),
                    _ => {
                        let (div, end) = (self.a.label(), self.a.label());
                        // constant divisors other than 0 and -1 need no checks
                        if !matches!(**r, Expr::Int(k) if k != 0 && k != -1) {
                            self.a.b(&[0x48, 0x85, 0xC9]);
                            self.a.j(JZ, self.div0);
                            // skip idiv on -1 since it faults on min
                            self.a.b(&[0x48, 0x83, 0xF9, 0xFF]);
                            self.a.j(JNZ, div);
                            if *op == "/" {
                                self.op(&[0x48, 0xF7, 0xD8], &res);
                            } else {
                                self.a.b(&[0x31, 0xC0]);
                            }
                            self.a.j(JMP, end);
                        }
                        self.a.bind(div);
                        let long = res == Ty::Long;
                        let idiv: &[u8] = if long { &[0x48, 0x99, 0x48, 0xF7, 0xF9] } else { &[0x99, 0xF7, 0xF9] };
                        self.a.b(idiv);
                        match (*op == "%", long) {
                            (true, true) => self.a.mov_rr(RAX, RDX),
                            (true, false) => self.a.b(&[0x48, 0x63, 0xC2]), // movsxd rax edx
                            (false, false) => self.a.b(SXD),
                            _ => {}
                        }
                        self.a.bind(end);
                    }
                }
                res
            }
            Expr::Call(n, args) => return self.call(n, args),
        })
    }

    fn call(&mut self, n: &str, args: &[Expr]) -> Result<Ty, String> {
        if n == "panic" {
            if args.len() != 1 || self.expr(&args[0])? != Ty::Str {
                return self.err("panic takes one string");
            }
            self.a.push(RAX);
            self.print_str("panic: ");
            self.a.pop(RAX);
            self.a.j(CALL, self.pstr);
            let nl = self.str("\n");
            self.a.ext(LEA, nl);
            self.a.j(JMP, self.die);
            return Ok(Ty::Void);
        }
        if n == "print" || n == "println" {
            if args.len() > 1 || (n == "print" && args.is_empty()) {
                return self.err(format!("{n} takes one argument"));
            }
            if let Some(a) = args.first() {
                match self.expr(a)? {
                    Ty::Int | Ty::Long => self.a.j(CALL, self.pint),
                    Ty::Str => self.a.j(CALL, self.pstr),
                    Ty::Bool => {
                        let (f, t, skip) = (self.str("false"), self.str("true"), self.a.label());
                        self.a.b(&[0x48, 0x85, 0xC0]);
                        self.a.ext(LEA, f);
                        self.a.j(JZ, skip);
                        self.a.ext(LEA, t);
                        self.a.bind(skip);
                        self.a.j(CALL, self.pstr);
                    }
                    t => return self.err(format!("can't print {t}")),
                }
            }
            if n == "println" {
                self.print_str("\n");
            }
            return Ok(Ty::Void);
        }
        let Some(&(f, label)) = self.fns.get(n) else { return self.err(format!("unknown function '{n}'")) };
        if args.len() != f.params.len() {
            return self.err(format!("'{n}' takes {} arguments", f.params.len()));
        }
        for (a, (pt, _)) in args.iter().zip(&f.params) {
            let t = self.expr(a)?;
            self.fit(pt, &t)?;
            self.a.push(RAX);
        }
        let Some(lib) = &f.lib else {
            if self.queued.insert(&f.name) {
                self.todo.push(f);
            }
            self.a.j(CALL, label);
            if !args.is_empty() {
                self.a.b(&[0x48, 0x81, 0xC4]);
                self.a.b(&(8 * args.len() as u32).to_le_bytes());
            }
            return Ok(f.ret.clone());
        };
        if self.unsafe_ == 0 {
            return self.err(format!("calling extern '{n}' requires an unsafe block"));
        }
        // shortcut register args only for now
        let regs: &[u8] = if self.win { &[RCX, RDX, R8, R9] } else { &[RDI, RSI, RDX, RCX, R8, R9] };
        if args.len() > regs.len() {
            return self.err(format!("extern calls support at most {} arguments", regs.len()));
        }
        for &r in regs[..args.len()].iter().rev() {
            self.a.pop(r);
        }
        let lib = match lib.as_str() {
            "c" if self.win => "msvcrt.dll",
            "c" => "libc.so.6",
            l => l,
        };
        let imp = self.import(lib, n);
        ccall(&mut self.a, imp);
        match f.ret {
            Ty::Int => self.a.b(SXD),
            Ty::Bool => self.a.b(&[0x0F, 0xB6, 0xC0]),
            _ => {}
        }
        Ok(f.ret.clone())
    }

    /// emits only used routines with callers first
    fn runtime(&mut self) {
        const K32: &str = "kernel32.dll";
        let msgs = ["panic: index out of bounds\n", "panic: division by zero\n", "panic: integer overflow\n"];
        for (l, m) in [self.oob, self.div0, self.ovf].into_iter().zip(msgs) {
            if self.a.used(l) {
                let m = self.str(m);
                self.a.bind(l);
                self.a.ext(LEA, m);
                self.a.j(JMP, self.die);
            }
        }
        let a = &mut self.a;
        if a.used(self.die) {
            // rax message then exit 101
            a.bind(self.die);
            a.j(CALL, self.pstr);
            a.imm(RCX, 101);
            a.j(JMP, self.quit);
        }
        if a.used(self.pint) {
            // rax value
            a.bind(self.pint);
            a.push(RBP);
            a.mov_rr(RBP, RSP);
            a.b(&[0x48, 0x83, 0xEC, 0x20]);
            a.mov_rr(RSI, RBP);
            a.mov_rr(R9, RAX);
            let (pos, dig, done) = (a.label(), a.label(), a.label());
            a.b(&[0x48, 0x85, 0xC0]);
            a.j(JNS, pos);
            a.b(&[0x48, 0xF7, 0xD8]);
            a.bind(pos);
            a.imm(R8, 10);
            a.bind(dig);
            // one digit per loop
            a.b(&[0x31, 0xD2, 0x49, 0xF7, 0xF0, 0x80, 0xC2, 0x30, 0x48, 0xFF, 0xCE, 0x88, 0x16, 0x48, 0x85, 0xC0]);
            a.j(JNZ, dig);
            a.b(&[0x4D, 0x85, 0xC9]);
            a.j(JNS, done);
            a.b(&[0x48, 0xFF, 0xCE, 0xC6, 0x06, 0x2D]);
            a.bind(done);
            a.mov_rr(RDX, RBP);
            a.b(&[0x48, 0x29, 0xF2]);
            a.j(CALL, self.write);
            a.b(&[0xC9, 0xC3]);
        }
        if a.used(self.pstr) {
            // rax nul-terminated string
            a.bind(self.pstr);
            a.mov_rr(RSI, RAX);
            a.mov_rr(RDX, RAX);
            let (top, end) = (a.label(), a.label());
            a.bind(top);
            a.b(&[0x80, 0x3A, 0x00]);
            a.j(JZ, end);
            a.b(&[0x48, 0xFF, 0xC2]);
            a.j(JMP, top);
            a.bind(end);
            a.b(&[0x48, 0x29, 0xF2]);
            a.j(JMP, self.write);
        }
        if self.a.used(self.write) {
            let crt = if self.win { "msvcrt.dll" } else { "libc.so.6" };
            // keep c stdio in order with ours
            let flush = self.imports.iter().any(|i| i.0 == crt).then(|| self.import(crt, "fflush"));
            let io = self.win.then(|| (self.import(K32, "GetStdHandle"), self.import(K32, "WriteFile")));
            let a = &mut self.a;
            // rsi ptr rdx len to stdout
            a.bind(self.write);
            if let Some(f) = flush {
                a.push(RSI);
                a.push(RDX);
                a.b(&[0x31, 0xC9, 0x31, 0xFF]); // fflush null
                ccall(a, f);
                a.pop(RDX);
                a.pop(RSI);
            }
            match io {
                Some((gsh, wf)) => {
                    a.push(RBP);
                    a.mov_rr(RBP, RSP);
                    a.push(RSI);
                    a.push(RDX);
                    a.b(&[0x48, 0x83, 0xE4, 0xF0, 0x48, 0x83, 0xEC, 0x30, 0x48, 0xC7, 0xC1, 0xF5, 0xFF, 0xFF, 0xFF]);
                    a.ext(&[0xFF, 0x15], Target::Import(gsh));
                    a.mov_rr(RCX, RAX);
                    a.mem(0x8B, RDX, RBP, -8);
                    a.mem(0x8B, R8, RBP, -16);
                    a.mem(0x8D, R9, RSP, 40);
                    a.b(&[0x31, 0xC0]);
                    a.mem(0x89, RAX, RSP, 32);
                    a.ext(&[0xFF, 0x15], Target::Import(wf));
                    a.b(&[0xC9, 0xC3]);
                }
                None => a.b(&[0xB8, 1, 0, 0, 0, 0xBF, 1, 0, 0, 0, 0x0F, 0x05, 0xC3]),
            }
        }
        let quit = if self.win {
            Some(self.import(K32, "ExitProcess"))
        } else if self.imports.is_empty() {
            None
        } else {
            Some(self.import("libc.so.6", "exit")) // flushes stdio
        };
        let a = &mut self.a;
        // rcx exit code
        a.bind(self.quit);
        a.mov_rr(RDI, RCX);
        match quit {
            Some(i) => ccall(a, i),
            None => a.b(&[0xB8, 0xE7, 0, 0, 0, 0x0F, 0x05]), // exit_group
        }
    }
}
