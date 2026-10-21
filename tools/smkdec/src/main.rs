//! Decode a Smacker cutscene through the install's own `SMACKW32.DLL`.
//!
//! Usage: `smkdec <dll> <file> <out-prefix> [frames]`
//!
//! Writes `<out-prefix>NNN.raw` (8-bit indices, width*height each) and `<out-prefix>meta.txt`
//! (width, height, frames). The DLL is 32-bit, so this binary must be too — that is the whole
//! reason it exists as a separate process.

use std::ffi::CString;

#[link(name = "kernel32")]
unsafe extern "system" {
    fn LoadLibraryW(name: *const u16) -> isize;
    fn GetProcAddress(module: isize, name: *const u8) -> *const ();
    fn FreeLibrary(module: isize) -> i32;
    fn GetLastError() -> u32;
}

// `SmackBufferOpen` needs a real window handle; the desktop is enough of one.
#[link(name = "user32")]
unsafe extern "system" {
    fn GetDesktopWindow() -> u32;
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 4 {
        eprintln!("usage: smkdec <dll> <file> <out-prefix> [frames]");
        std::process::exit(2);
    }
    let (dll, file, prefix) = (&args[1], &args[2], &args[3]);
    let want: usize = args.get(4).and_then(|v| v.parse().ok()).unwrap_or(3);
    // Flags/extra are overridable so the SDK constants can be probed instead of guessed.
    let flags: u32 = args
        .get(5)
        .and_then(|v| u32::from_str_radix(v.trim_start_matches("0x"), 16).ok())
        .unwrap_or(0x000f_e000);
    let extra: i32 = args.get(6).and_then(|v| v.parse().ok()).unwrap_or(-1);
    // `start`: skip this many frames before decoding, so a long film can be decoded in pieces.
    let start: usize = args.get(7).and_then(|v| v.parse().ok()).unwrap_or(0);

    let header = std::fs::read(file).expect("read the smk");
    assert_eq!(&header[0..3], b"SMK", "not a Smacker file");
    let word = |at: usize| u32::from_le_bytes(header[at..at + 4].try_into().unwrap());
    let (width, height, frames) = (word(4), word(8), word(12));

    let wide: Vec<u16> = dll.encode_utf16().chain(std::iter::once(0)).collect();
    let library = unsafe { LoadLibraryW(wide.as_ptr()) };
    if library == 0 {
        eprintln!("LoadLibraryW failed (error {})", unsafe { GetLastError() });
        std::process::exit(1);
    }
    // The DLL exports its functions stdcall-decorated (`_SmackOpen@12`), so both spellings are
    // tried: the plain SDK name first, then the decorated one with the argument bytes.
    let resolve = |name: &str, stack: u32| -> isize {
        for candidate in [name.to_string(), format!("_{name}@{stack}")] {
            let cname = CString::new(candidate).unwrap();
            let address = unsafe { GetProcAddress(library, cname.as_ptr() as *const u8) };
            if !address.is_null() {
                return address as isize;
            }
        }
        panic!("SMACKW32.DLL has no export for {name}");
    };
    let open: unsafe extern "system" fn(*const u8, u32, i32) -> *mut u8 =
        unsafe { std::mem::transmute(resolve("SmackOpen", 12)) };
    let to_buffer: unsafe extern "system" fn(*mut u8, u32, u32, u32, u32, *mut u8, u32) =
        unsafe { std::mem::transmute(resolve("SmackToBuffer", 28)) };
    let do_frame: unsafe extern "system" fn(*mut u8) -> u32 =
        unsafe { std::mem::transmute(resolve("SmackDoFrame", 4)) };
    let next_frame: unsafe extern "system" fn(*mut u8) =
        unsafe { std::mem::transmute(resolve("SmackNextFrame", 4)) };
    let close: unsafe extern "system" fn(*mut u8) =
        unsafe { std::mem::transmute(resolve("SmackClose", 4)) };
    // `SmackBufferOpen(HWND, mode, w, h, pitch, height)` returns a 0x478-byte struct:
    // `SmackBufferOpen@24` @ 0x4020e0. Mode 2 allocates its own pixel buffer at `+0x448`
    // (`puVar10[0x112]`) and returns without creating a window, and it keeps a 256-entry
    // palette at `+0x3C` as BGRX dwords.
    let buffer_open: unsafe extern "system" fn(u32, u16, u16, u16, u16, u16) -> *mut u8 =
        unsafe { std::mem::transmute(resolve("SmackBufferOpen", 24)) };

    let path = CString::new(file.as_bytes()).unwrap();
    // JK.EXE's own arguments (`FUN_00426730`): flags 0xfe000, extra -1 (`SmackAUTOEXTRA`),
    // which is what makes the DLL allocate its frame ring.
    let handle = unsafe { open(path.as_ptr() as *const u8, flags, extra) };
    if handle.is_null() {
        eprintln!("SmackOpen refused {file}");
        std::process::exit(1);
    }
    let mut buffer = vec![0u8; (width * height) as usize];
    // The palette lives in a `SmackBuf`, and `SmackBufferOpen` refuses both a 640-wide frame
    // (`param_3 < 0x141`) and a null window handle. A real window — the desktop — with the
    // largest size it accepts is enough to own a palette, if the DLL updates it for frames it
    // decodes elsewhere.
    let desktop = unsafe { GetDesktopWindow() };
    // `_SmackBufferOpen@24` case 0 also caps the height at 0xf0 (240), so the pair it accepts is
    // a small preview: 320x240 at most.
    let mut palette_buf: *mut u8 = std::ptr::null_mut();
    for (w, h) in [(320u16, 240u16), (160, 120), (64, 64)] {
        for mode in [0u16, 1, 2, 3] {
            let opened = unsafe { buffer_open(desktop, mode, w, h, w, h) };
            if !opened.is_null() {
                eprintln!("SmackBufferOpen ok: mode {mode}, {w}x{h}");
                palette_buf = opened;
                break;
            }
        }
        if !palette_buf.is_null() {
            break;
        }
    }
    if palette_buf.is_null() {
        eprintln!("SmackBufferOpen refused every size and mode");
    }
    // Decode indices into our own buffer; the SmackBuf above, if it exists, is only a home for
    // the palette the DLL keeps while it decodes.
    unsafe { to_buffer(handle, 0, 0, width, height, buffer.as_mut_ptr(), 0) };

    std::fs::write(
        format!("{prefix}meta.txt"),
        format!("{width}x{height} {frames} frames\n"),
    )
    .expect("write meta");

    let mut decoded = 0usize;
    let mut hashes: Vec<u64> = Vec::new();
    let mut first_ink: Option<usize> = None;
    // Skip to the requested frame with the SDK's own seek (`SmackGoto@8`): it finds the nearest
    // keyframe and decodes forward from there, which is what makes chunked decoding cheap.
    if start > 0 {
        let goto: unsafe extern "system" fn(*mut u8, u32) -> u32 =
            unsafe { std::mem::transmute(resolve("SmackGoto", 8)) };
        let ok = unsafe { goto(handle, start as u32) };
        if ok == 0 {
            eprintln!("SmackGoto refused frame {start}");
        }
    }
    decoded = 0;
    while decoded < want {
        if unsafe { do_frame(handle) } != 0 {
            break;
        }
        let mut hash = 0xcbf2_9ce4_8422_2325u64;
        for byte in &buffer {
            hash ^= *byte as u64;
            hash = hash.wrapping_mul(0x100_0000_01b3);
        }
        let path = format!("{prefix}{decoded:03}.raw");
        std::fs::write(&path, &buffer).expect("write frame");
        if std::env::var_os("SMKDEC_TRACE").is_some() && decoded < 3 {
            eprintln!("wrote {path} ({} bytes)", buffer.len());
        }
        if buffer.iter().any(|b| *b != 0) && first_ink.is_none() {
            first_ink = Some(decoded);
        }
        hashes.push(hash);
        // The frame as a PPM, so it can be looked at. The palette is NOT here: the DLL keeps
        // one only for `SmackBufferOpen` buffers, and that API refuses a 640-wide frame
        // (`_SmackBufferOpen@24` @ 0x4020e0 only takes `param_3 < 0x141`), so what is written
        // is the 8-bit indices as grey. Colour needs the Smacker palette chunk, which this
        // helper does not parse yet.
        let mut ppm = Vec::with_capacity((width * height * 3) as usize + 32);
        ppm.extend_from_slice(format!("P6\n{width} {height}\n255\n").as_bytes());
        for row in 0..height as usize {
            for col in 0..width as usize {
                let index = buffer[row * width as usize + col];
                ppm.extend_from_slice(&[index, index, index]);
            }
        }
        std::fs::write(format!("{prefix}{decoded:03}.ppm"), &ppm).expect("write ppm");
        // The SmackBuf holds the palette the DLL last applied, at +0x3C as BGRA entries
        // (`SmackBufferNewPalette` @ 0x401920 reads exactly there). Dump it per frame.
        if !palette_buf.is_null() {
            let base = palette_buf as usize + 0x3c;
            let palette = unsafe { std::slice::from_raw_parts(base as *const u8, 1024) };
            std::fs::write(format!("{prefix}palette{decoded:03}.bin"), palette)
                .expect("write palette");
        }
        println!("frame {decoded}: {hash:016x}");
        decoded += 1;
        unsafe { next_frame(handle) };
    }
    unsafe { close(handle) };
    unsafe { FreeLibrary(library) };
    let mut distinct = hashes.clone();
    distinct.sort_unstable();
    distinct.dedup();
    println!(
        "decoded {decoded} frame(s) at {width}x{height}, {} of them distinct, first non-blank at {:?}, {frames} in the file",
        distinct.len(),
        first_ink
    );
}