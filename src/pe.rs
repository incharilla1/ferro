use crate::asm::{align, link, Target};
use crate::codegen::Out;

const BASE: u64 = 0x1_4000_0000;
const TEXT: usize = 0x1000;

pub fn write(o: Out) -> Vec<u8> {
    let (mut code, mut d) = (o.code, o.data);
    let data_rva = TEXT + align(code.len(), 0x1000);

    let mut dlls: Vec<(&str, Vec<usize>)> = vec![];
    for (i, (lib, _)) in o.imports.iter().enumerate() {
        match dlls.iter_mut().find(|x| x.0 == lib) {
            Some(x) => x.1.push(i),
            None => dlls.push((lib, vec![i])),
        }
    }
    d.resize(align(d.len(), 8), 0);
    let desc = d.len();
    d.resize(desc + (dlls.len() + 1) * 20, 0);
    let mut slot = vec![0; o.imports.len()];
    let mut thunks = vec![];
    for (_, idx) in &dlls {
        let ilt = d.len();
        let iat = ilt + (idx.len() + 1) * 8;
        d.resize(iat + (idx.len() + 1) * 8, 0);
        for (k, &i) in idx.iter().enumerate() {
            slot[i] = iat + 8 * k;
        }
        thunks.push((ilt, iat));
    }
    for (j, ((lib, idx), (ilt, iat))) in dlls.iter().zip(thunks).enumerate() {
        let name = d.len();
        d.extend(lib.as_bytes());
        d.push(0);
        for (k, &i) in idx.iter().enumerate() {
            d.resize(align(d.len(), 2), 0);
            let hn = ((data_rva + d.len()) as u64).to_le_bytes();
            d.extend([0, 0]);
            d.extend(o.imports[i].1.as_bytes());
            d.push(0);
            d[ilt + 8 * k..][..8].copy_from_slice(&hn);
            d[iat + 8 * k..][..8].copy_from_slice(&hn);
        }
        for (off, v) in [(0, ilt), (12, name), (16, iat)] {
            d[desc + 20 * j + off..][..4].copy_from_slice(&((data_rva + v) as u32).to_le_bytes());
        }
    }
    link(&mut code, &o.ext, BASE + TEXT as u64, |t| {
        BASE + (data_rva
            + match t {
                Target::Data(x) => *x,
                Target::Import(i) => slot[*i],
            }) as u64
    });

    let (text_raw, data_raw) = (align(code.len(), 0x200), align(d.len(), 0x200));
    let mut h: Vec<u8> = b"PE\0\0".to_vec();
    let u16s = |h: &mut Vec<u8>, v: &[u16]| v.iter().for_each(|x| h.extend(x.to_le_bytes()));
    let u32s = |h: &mut Vec<u8>, v: &[usize]| v.iter().for_each(|x| h.extend((*x as u32).to_le_bytes()));
    u16s(&mut h, &[0x8664, 2]); // x64 2 sections
    h.extend([0; 12]);
    u16s(&mut h, &[0xF0, 0x22, 0x20B, 0]); // opt header size flags pe32+
    u32s(&mut h, &[text_raw, data_raw, 0, TEXT, TEXT]); // sizes entry code base
    h.extend(BASE.to_le_bytes());
    u32s(&mut h, &[0x1000, 0x200]); // section and file align
    u16s(&mut h, &[6, 0, 0, 0, 6, 0]); // versions
    u32s(&mut h, &[0, data_rva + align(d.len(), 0x1000), 0x200, 0]); // image and header size
    u16s(&mut h, &[3, 0x8100]); // console nx
    for v in [0x100000u64, 0x1000, 0x100000, 0x1000] {
        h.extend(v.to_le_bytes()); // stack and heap
    }
    u32s(&mut h, &[0, 16]);
    // only the import directory
    for i in 0..16 {
        u32s(&mut h, &if i == 1 { [data_rva + desc, (dlls.len() + 1) * 20] } else { [0, 0] });
    }
    for (name, vsz, rva, raw, ptr, ch) in [
        (b".text\0\0\0", code.len(), TEXT, text_raw, 0x200, 0x6000_0020u32),
        (b".data\0\0\0", d.len(), data_rva, data_raw, 0x200 + text_raw, 0xC000_0040),
    ] {
        h.extend(name);
        u32s(&mut h, &[vsz, rva, raw, ptr]);
        h.extend([0; 12]);
        h.extend(ch.to_le_bytes());
    }

    let mut f = vec![0u8; 0x200];
    f[..2].copy_from_slice(b"MZ");
    f[0x3C] = 0x40;
    f[0x40..0x40 + h.len()].copy_from_slice(&h);
    f.extend(&code);
    f.resize(0x200 + text_raw, 0);
    f.extend(&d);
    f.resize(0x200 + text_raw + data_raw, 0);
    f
}
