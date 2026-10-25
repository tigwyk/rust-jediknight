//! Reading the PPM the 32-bit SMK helper writes.
//!
//! `tools/smkdec` links the install's 32-bit `SMACKW32.DLL` and has no dependencies, so it dumps
//! frames as `P6` binary PPM rather than PNG; this is the reader for that byte format.

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
    fn reads_the_helpers_ppm() {
        // The PPM the helper writes: a 2x1 image, one red pixel and one green.
        let ppm = b"P6\n2 1\n255\n\xff\x00\x00\x00\xff\x00";
        let (w, h, pixels) = read_ppm(ppm).expect("ppm");
        assert_eq!((w, h), (2, 1));
        assert_eq!(pixels.len(), 6);
        assert_eq!(&pixels[..3], &[0xff, 0x00, 0x00]);
    }
}
