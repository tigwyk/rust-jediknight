//! A minimal PNG writer: 8-bit RGB, no compression (deflate *stored* blocks).
//!
//! Only here so a decoded cutscene frame can be looked at without pulling in an image
//! dependency. Everything else in the project reads shipped formats; this is the one place
//! that writes a picture.

use std::io::Write;
use std::path::Path;

fn crc32(bytes: &[u8]) -> u32 {
    let mut table = [0u32; 256];
    for (i, slot) in table.iter_mut().enumerate() {
        let mut c = i as u32;
        for _ in 0..8 {
            c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
        }
        *slot = c;
    }
    let mut crc = 0xFFFF_FFFFu32;
    for byte in bytes {
        crc = table[((crc ^ *byte as u32) & 0xFF) as usize] ^ (crc >> 8);
    }
    crc ^ 0xFFFF_FFFF
}

fn adler32(bytes: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for byte in bytes {
        a = (a + *byte as u32) % 65521;
        b = (b + a) % 65521;
    }
    (b << 16) | a
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    let mut body = Vec::with_capacity(4 + data.len());
    body.extend_from_slice(kind);
    body.extend_from_slice(data);
    out.extend_from_slice(&body);
    out.extend_from_slice(&crc32(&body).to_be_bytes());
}

/// Write `rgb` (3 bytes a pixel, row-major) as a PNG.
pub fn write_rgb(path: &Path, width: u32, height: u32, rgb: &[u8]) -> std::io::Result<()> {
    assert_eq!(rgb.len(), (width * height * 3) as usize, "rgb length must match the size");
    // Raw scanlines, each prefixed with filter type 0.
    let mut raw = Vec::with_capacity((height * (width * 3 + 1)) as usize);
    for row in 0..height as usize {
        raw.push(0);
        let at = row * width as usize * 3;
        raw.extend_from_slice(&rgb[at..at + width as usize * 3]);
    }
    // zlib stream with stored blocks: 65535 bytes maximum each.
    let mut zlib = vec![0x78, 0x01];
    let mut chunks = raw.chunks(65535).peekable();
    while let Some(block) = chunks.next() {
        zlib.push(if chunks.peek().is_none() { 1 } else { 0 });
        zlib.extend_from_slice(&(block.len() as u16).to_le_bytes());
        zlib.extend_from_slice(&(!(block.len() as u16)).to_le_bytes());
        zlib.extend_from_slice(block);
    }
    zlib.extend_from_slice(&adler32(&raw).to_be_bytes());

    let mut out = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.extend_from_slice(&[8, 2, 0, 0, 0]);
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(&mut out, b"IDAT", &zlib);
    chunk(&mut out, b"IEND", &[]);
    let mut file = std::fs::File::create(path)?;
    file.write_all(&out)
}

/// Read a `P6` binary PPM back, for the frames the 32-bit helper writes.
///
/// The header is `P6`, width, height, `255`, each separated by whitespace, then raw bytes —
/// so only the first few dozen bytes are text and the rest is pixels.
pub fn read_ppm(bytes: &[u8]) -> Option<(u32, u32, Vec<u8>)> {
    let (mut width, mut height) = (None, None);
    let mut at = 0usize;
    let mut field = 0;
    while at < bytes.len() && field < 4 {
        while at < bytes.len() && bytes[at].is_ascii_whitespace() {
            at += 1;
        }
        let start = at;
        while at < bytes.len() && !bytes[at].is_ascii_whitespace() {
            at += 1;
        }
        let token = std::str::from_utf8(&bytes[start..at]).ok()?;
        match field {
            0 if token != "P6" => return None,
            1 => width = token.parse().ok(),
            2 => height = token.parse().ok(),
            3 if token != "255" => return None,
            _ => {}
        }
        field += 1;
    }
    // Exactly one whitespace byte separates the last header token from the pixels.
    let header = at + 1;
    let (width, height) = (width?, height?);
    let pixels = bytes.get(header..)?.to_vec();
    (pixels.len() >= (width * height * 3) as usize).then_some((width, height, pixels))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_a_png_and_reads_a_ppm() {
        let path = std::env::temp_dir().join("bk-png-test.png");
        let rgb = vec![255u8, 0, 0, 0, 255, 0, 0, 0, 255, 255, 255, 255];
        write_rgb(&path, 2, 2, &rgb).expect("write");
        let bytes = std::fs::read(&path).expect("read back");
        assert_eq!(&bytes[0..8], &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]);
        assert_eq!(&bytes[12..16], b"IHDR");
        // The file ends with an empty IEND chunk: length 0, `IEND`, then its CRC.
        assert_eq!(&bytes[bytes.len() - 12..bytes.len() - 8], &[0, 0, 0, 0]);
        assert_eq!(&bytes[bytes.len() - 8..bytes.len() - 4], b"IEND");
        std::fs::remove_file(&path).ok();

        // The PPM the helper writes: a 2x1 image of one red pixel.
        let ppm = b"P6\n2 1\n255\n\xff\x00\x00\x00\xff\x00";
        let (w, h, pixels) = read_ppm(ppm).expect("ppm");
        assert_eq!((w, h), (2, 1));
        assert_eq!(pixels.len(), 6);
    }
}