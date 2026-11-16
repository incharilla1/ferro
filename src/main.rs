mod asm;
mod codegen;
mod elf;
mod lexer;
mod parser;
mod pe;

use std::collections::HashSet;
use std::path::{Component, Path, PathBuf};
use std::{env, fs, process::exit};

fn fail(m: &str) -> ! {
    eprintln!("error: {m}");
    exit(1)
}

/// drop . and resolve .. without touching disk
fn clean(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir if matches!(out.components().next_back(), Some(Component::Normal(_))) => {
                out.pop();
            }
            c => out.push(c),
        }
    }
    out
}

/// load a file and its imports once each
fn load(path: &Path, seen: &mut HashSet<PathBuf>, out: &mut Vec<parser::Func>) -> Result<(), String> {
    let path = clean(path);
    let name = path.display().to_string();
    let full = path.canonicalize().map_err(|e| format!("{name}: {e}"))?;
    if !seen.insert(full.clone()) {
        return Ok(());
    }
    let src = fs::read_to_string(&full).map_err(|e| format!("{name}: {e}"))?;
    let (imports, funcs) = lexer::lex(&src).and_then(parser::parse).map_err(|e| format!("{name}:{e}"))?;
    for i in imports {
        load(&path.parent().unwrap().join(i), seen, out)?;
    }
    out.extend(funcs.into_iter().map(|mut f| {
        f.file = name.clone();
        f
    }));
    Ok(())
}

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    let (mut input, mut out, mut win) = (None, None, cfg!(windows));
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "-o" => out = it.next().cloned(),
            "--target" => {
                win = match it.next().map(String::as_str) {
                    Some("windows") => true,
                    Some("linux") => false,
                    _ => fail("--target must be windows or linux"),
                }
            }
            _ => input = Some(a.clone()),
        }
    }
    let Some(input) = input else { fail("usage: ferro <file.fe> [-o out] [--target windows|linux]") };
    let mut f = vec![];
    load(Path::new(&input), &mut HashSet::new(), &mut f).unwrap_or_else(|e| fail(&e));
    let (_, std) = lexer::lex(include_str!("std.fe")).and_then(parser::parse).expect("std.fe");
    let std: Vec<_> = std.into_iter().filter(|s| !f.iter().any(|u| u.name == s.name)).collect();
    f.extend(std.into_iter().map(|mut s| {
        s.file = "std".into();
        s
    }));
    let o = codegen::compile(&f, win).unwrap_or_else(|e| fail(&e));
    let out = out.unwrap_or_else(|| {
        Path::new(&input).with_extension(if win { "exe" } else { "" }).to_string_lossy().into_owned()
    });
    if out == input {
        fail("output would overwrite the source file; use -o");
    }
    fs::write(&out, if win { pe::write(o) } else { elf::write(o) }).unwrap_or_else(|e| fail(&format!("{out}: {e}")));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&out, fs::Permissions::from_mode(0o755)).unwrap_or_else(|e| fail(&format!("{out}: {e}")));
    }
}
