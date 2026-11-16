use crate::asm::{align, link, Target};
use crate::codegen::Out;

const BASE: u64 = 0x40_0000;
const INTERP: &[u8] = b"/lib64/ld-linux-x86-64.so.2\0";

pub fn write(o: Out) -> Vec<u8> {
    let (mut code, mut d) = (o.code, o.data);
    let n = o.imports.len();
    // no page padding between headers code and data
    let text = 64 + 56 * if n > 0 { 4 } else { 2 };
    let data_off = align(text + code.len(), 8);
    let dva = BASE + 0x1000 + data_off as u64; // next page same page offset
    // type flags offset vaddr size align
    let mut ph = vec![];
    let mut dynamic = None;
    let mut got = 0;
    if n > 0 {
        ph.push((3u32, 4u32, data_off + d.len(), dva + d.len() as u64, INTERP.len(), 1u64));
        d.extend(INTERP);
        let strtab = d.len();
        d.push(0);
        let mut libs: Vec<(&str, usize)> = vec![];
        for (lib, _) in &o.imports {
            if !libs.iter().any(|l| l.0 == lib) {
                libs.push((lib, d.len() - strtab));
                d.extend(lib.as_bytes());
                d.push(0);
            }
        }
        let mut names = vec![];
        for (_, name) in &o.imports {
            names.push(d.len() - strtab);
            d.extend(name.as_bytes());
            d.push(0);
        }
        let strsz = d.len() - strtab;
        d.resize(align(d.len(), 8), 0);
        let symtab = d.len();
        d.resize(symtab + 24, 0);
        for nm in names {
            d.extend((nm as u32).to_le_bytes());
            d.extend([0x12, 0, 0, 0]); // global func undefined
            d.extend([0; 16]);
        }
        // sysv hash with 1 bucket and no chains
        let hash = d.len();
        d.extend(1u32.to_le_bytes());
        d.extend((n as u32 + 1).to_le_bytes());
        d.resize(hash + 8 + 4 * (n + 2), 0);
        d.resize(align(d.len(), 8), 0);
        let rela = d.len();
        got = rela + 24 * n;
        for i in 0..n {
            for v in [dva + (got + 8 * i) as u64, ((i as u64 + 1) << 32) | 6, 0] {
                d.extend(v.to_le_bytes()); // glob_dat reloc
            }
        }
        d.resize(got + 8 * n, 0);
        let start = d.len();
        let mut dy: Vec<(u64, u64)> = libs.iter().map(|l| (1, l.1 as u64)).collect();
        dy.extend([
            (4, dva + hash as u64),
            (5, dva + strtab as u64),
            (6, dva + symtab as u64),
            (10, strsz as u64),
            (11, 24),
            (7, dva + rela as u64),
            (8, 24 * n as u64),
            (9, 24),
            (0, 0),
        ]);
        for (t, v) in dy {
            d.extend(t.to_le_bytes());
            d.extend(v.to_le_bytes());
        }
        dynamic = Some((2, 6, data_off + start, dva + start as u64, d.len() - start, 8));
    }
    link(&mut code, &o.ext, BASE + text as u64, |t| {
        dva + match t {
            Target::Data(x) => *x,
            Target::Import(i) => got + 8 * i,
        } as u64
    });
    ph.push((1, 5, 0, BASE, text + code.len(), 0x1000)); // text rx with headers
    ph.push((1, 6, data_off, dva, d.len(), 0x1000)); // data rw
    ph.extend(dynamic);

    let mut f = vec![0x7F, b'E', b'L', b'F', 2, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0];
    f.extend(2u16.to_le_bytes()); // executable
    f.extend(0x3Eu16.to_le_bytes()); // x86-64
    f.extend(1u32.to_le_bytes());
    for v in [BASE + text as u64, 64, 0] { // entry phdrs no shdrs
        f.extend(v.to_le_bytes());
    }
    f.extend(0u32.to_le_bytes());
    for v in [64u16, 56, ph.len() as u16, 64, 0, 0] {
        f.extend(v.to_le_bytes());
    }
    for (t, fl, off, va, sz, al) in ph {
        f.extend(t.to_le_bytes());
        f.extend(fl.to_le_bytes());
        for v in [off as u64, va, va, sz as u64, sz as u64, al] {
            f.extend(v.to_le_bytes());
        }
    }
    f.extend(&code);
    f.resize(data_off, 0);
    f.extend(&d);
    f
}
