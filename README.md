# ferro

a small standalone compiled language (About 300-400kb on windows).

> experimental (v0.1)

## build

```sh
cargo build --release
ferro hello.fe # current os
ferro hello.fe --target linux -o hello
ferro hello.fe --target windows -o hello.exe
```

## example

```csharp
int fib(int n) {
    if (n < 2) { return n; }
    return fib(n - 1) + fib(n - 2);
}

void main() {
    int[10] a;
    int i = 0;
    while (i < 10) {
        a[i] = fib(i);
        i = i + 1;
    }
    println(a[9]);
}
```

more in [examples/hello.fe](examples/hello.fe).

## language

- types: `int` (32-bit), `long` (64-bit), `bool`, `string`, `void`, `T[N]`
- statements: variables, `if` / `else`, `while`, `return`, `unsafe { }`, `asm { }`
- operators: `+ - * / %`, `== != < <= > >=`, `&& || !`
- built-ins: `print`, `println`, `panic`
- std: `abs`, `min`, `max`, `clamp`, `pow`, `assert`
- `main` is `void main()` or `int main()`

## multiple files

```csharp
import "math.fe";   // path is relative to this file
import "lib/io.fe";

void main() { println(sq(7)); }
```

everything in an imported file is visible. each file only loads once, so circular imports are fine.

## safety

- no pointers or manual memory
- variables always start initialized
- array indexes are bounds-checked
- overflow and divide by zero are caught
- stack overflow crashes cleanly

a failed check prints `panic: <reason>` and exits with 101 (bc rust). calling c and inline asm need `unsafe`.

## c/c++

```csharp
extern "c" int puts(string s); // "c" = msvcrt.dll on windows, libc.so.6 on linux
extern "tuff.dll" long add(long a, long b);

void main() {
    unsafe {
        int a = 69;
        int b = 420;
        puts("hello from C");
        add(a, b);
    }
}
```

uses the normal c abi. `string` is passed as `char*`. c++ works through `extern "C"` functions. max 4 args on windows, 6 on linux.

## inline asm

```csharp
long x = 40;
unsafe {
    asm {
        mov rax, [x]; // [name] = a ferro variable
        add rax, 2;
        mov [x], rax;
    }
}
```

- operands: 64-bit registers, numbers (`0x` hex works), `[reg]`, `[reg + n]`, `[variable]`
- instructions: `mov add sub and or xor cmp lea imul inc dec neg not push pop syscall nop int3 cqo`
- `db 0x0F, 0x05;` for raw bytes
- don't touch `rsp` / `rbp`. no labels or jumps yet.

## not yet

`for`, `break`, structs, floats, string stuff, arrays as params, namespaces.
