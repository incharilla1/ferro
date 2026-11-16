pub const RAX: u8 = 0;
pub const RCX: u8 = 1;
pub const RDX: u8 = 2;
pub const RSP: u8 = 4;
pub const RBP: u8 = 5;
pub const RSI: u8 = 6;
pub const RDI: u8 = 7;
pub const R8: u8 = 8;
pub const R9: u8 = 9;
pub const R12: u8 = 12;

pub const LEA: &[u8] = &[0x48, 0x8D, 0x05]; // lea rax [rip+x]
pub const CALL: &[u8] = &[0xE8];
pub const JMP: &[u8] = &[0xE9];
pub const JZ: &[u8] = &[0x0F, 0x84];
pub const JNZ: &[u8] = &[0x0F, 0x85];
pub const JAE: &[u8] = &[0x0F, 0x83];
pub const JNS: &[u8] = &[0x0F, 0x89];
pub const JO: &[u8] = &[0x0F, 0x80];

#[derive(Clone, Copy)]
pub enum Rm {
    Reg(u8),
    Mem(u8, i32),
}

pub enum Target {
    Data(usize),
    Import(usize),
}

#[derive(Default)]
pub struct Asm {
    pub code: Vec<u8>,
    labels: Vec<usize>,
    fix: Vec<(usize, usize)>,
    pub ext: Vec<(usize, Target)>,
}

impl Asm {
    pub fn b(&mut self, b: &[u8]) {
        self.code.extend_from_slice(b)
    }
    pub fn label(&mut self) -> usize {
        self.labels.push(usize::MAX);
        self.labels.len() - 1
    }
    pub fn bind(&mut self, l: usize) {
        self.labels[l] = self.code.len()
    }
    pub fn j(&mut self, op: &[u8], l: usize) {
        self.b(op);
        self.fix.push((self.code.len(), l));
        self.b(&[0; 4])
    }
    pub fn ext(&mut self, op: &[u8], t: Target) {
        self.b(op);
        self.ext.push((self.code.len(), t));
        self.b(&[0; 4])
    }
    /// rex.w op modrm with r as reg or opcode ext
    pub fn modrm(&mut self, op: &[u8], r: u8, rm: Rm) {
        let (Rm::Reg(b) | Rm::Mem(b, _)) = rm;
        self.b(&[0x48 | (r >> 3) << 2 | b >> 3]);
        self.b(op);
        match rm {
            Rm::Reg(b) => self.b(&[0xC0 | (r & 7) << 3 | b & 7]),
            Rm::Mem(b, d) => {
                self.b(&[0x80 | (r & 7) << 3 | b & 7]);
                if b & 7 == 4 {
                    self.b(&[0x24]);
                }
                self.b(&d.to_le_bytes())
            }
        }
    }
    pub fn mov_rr(&mut self, d: u8, s: u8) {
        self.modrm(&[0x89], s, Rm::Reg(d))
    }
    /// 0x8b load 0x89 store 0x8d lea
    pub fn mem(&mut self, op: u8, r: u8, base: u8, disp: i32) {
        self.modrm(&[op], r, Rm::Mem(base, disp))
    }
    /// opcode plus register
    fn plus_r(&mut self, op: u8, r: u8) {
        if r >= 8 {
            self.b(&[0x41]);
        }
        self.b(&[op + (r & 7)])
    }
    pub fn push(&mut self, r: u8) {
        self.plus_r(0x50, r)
    }
    pub fn pop(&mut self, r: u8) {
        self.plus_r(0x58, r)
    }
    /// shortest mov r imm
    pub fn imm(&mut self, r: u8, v: i64) {
        if let Ok(u) = u32::try_from(v) {
            self.plus_r(0xB8, r);
            self.b(&u.to_le_bytes())
        } else if let Ok(i) = i32::try_from(v) {
            self.modrm(&[0xC7], 0, Rm::Reg(r));
            self.b(&i.to_le_bytes())
        } else {
            self.b(&[0x48 | r >> 3, 0xB8 + (r & 7)]);
            self.b(&v.to_le_bytes())
        }
    }
    pub fn used(&self, l: usize) -> bool {
        self.fix.iter().any(|f| f.1 == l)
    }
    pub fn finish(&mut self) {
        for &(p, l) in &self.fix {
            let v = self.labels[l] as i32 - (p + 4) as i32;
            self.code[p..p + 4].copy_from_slice(&v.to_le_bytes());
        }
    }
}

pub fn align(v: usize, a: usize) -> usize {
    v.div_ceil(a) * a
}

/// patch rip-relative refs once addresses are known
pub fn link(code: &mut [u8], ext: &[(usize, Target)], text: u64, addr: impl Fn(&Target) -> u64) {
    for (p, t) in ext {
        let v = (addr(t) as i64 - (text + *p as u64 + 4) as i64) as i32;
        code[*p..*p + 4].copy_from_slice(&v.to_le_bytes());
    }
}
