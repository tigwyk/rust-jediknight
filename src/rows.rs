//! Helpers for the engine's plain-text list formats.
//!
//! `.jkl`, `.3do` and `.key` are all ASCII with `<index>:` rows, `#` comments and
//! a literal `end` terminator. The declared row counts in their headers are
//! allocation hints, not row counts (see `docs/formats.md`), so both parsers
//! share one stop-at-`end` reader.

pub fn fields(line: &str) -> Vec<&str> {
    line.split_whitespace().collect()
}

/// True when the line is an `<index>:` row.
pub fn is_row(line: &str) -> bool {
    match line.find(':') {
        Some(p) if p > 0 => line[..p].bytes().all(|b| b.is_ascii_digit()),
        _ => false,
    }
}

fn is_terminator(line: &str) -> bool {
    line.eq_ignore_ascii_case("end") || line.to_ascii_lowercase().starts_with("section")
}

/// Skip leading blank/comment lines, then collect `<index>:` rows.
pub fn read_rows<'a>(lines: &[&'a str], i: &mut usize, limit: usize) -> Vec<&'a str> {
    skip_leading(lines, i);
    let mut out = Vec::new();
    while out.len() < limit && *i < lines.len() {
        let line = lines[*i].trim();
        if !is_row(line) {
            if is_terminator(line) {
                *i += 1;
            }
            break;
        }
        out.push(line);
        *i += 1;
    }
    // Invariant: a read always makes progress, so no caller can spin on a header
    // whose declared count does not match the rows that follow.
    if out.is_empty() && *i < lines.len() && !is_terminator(lines[*i].trim()) {
        *i += 1;
    }
    out
}

/// Collect bare-name rows (no `<index>:` prefix), as the SOUNDS list uses.
pub fn read_names<'a>(lines: &[&'a str], i: &mut usize, limit: usize) -> Vec<&'a str> {
    skip_leading(lines, i);
    let mut out = Vec::new();
    while out.len() < limit && *i < lines.len() {
        let line = lines[*i].trim();
        if line.is_empty() || line.starts_with('#') || is_terminator(line) {
            if is_terminator(line) {
                *i += 1;
            }
            break;
        }
        if line.split_whitespace().count() != 1 {
            break;
        }
        out.push(line);
        *i += 1;
    }
    out
}

/// Collect free-form rows that carry several whitespace-separated fields and no
/// index prefix, as the TEMPLATES list uses.
pub fn read_block<'a>(lines: &[&'a str], i: &mut usize, limit: usize) -> Vec<&'a str> {
    skip_leading(lines, i);
    let mut out = Vec::new();
    while out.len() < limit && *i < lines.len() {
        let line = lines[*i].trim();
        if line.is_empty() || line.starts_with('#') || is_terminator(line) {
            break;
        }
        out.push(line);
        *i += 1;
    }
    out
}

fn skip_leading(lines: &[&str], i: &mut usize) {
    while *i < lines.len() {
        let line = lines[*i].trim();
        if line.is_empty() || line.starts_with('#') {
            *i += 1;
        } else {
            break;
        }
    }
}

/// Parse up to `want` floats starting at field `skip`.
pub fn floats_in(line: &str, skip: usize, want: usize) -> Vec<f32> {
    fields(line)
        .into_iter()
        .skip(skip)
        .take(want)
        .filter_map(|s| s.parse().ok())
        .collect()
}

/// Split comma-separated integer pairs while tolerating spaces around the comma.
///
/// `.jkl` writes `42,47` but `.3do` writes `0,  0`, so splitting on `,` inside
/// each whitespace token (rather than on whitespace alone) keeps the order.
pub fn ints(line_fields: &[&str]) -> Vec<i32> {
    let mut out = Vec::new();
    for tok in line_fields {
        for part in tok.split(',') {
            if let Ok(v) = part.trim().parse::<i32>() {
                out.push(v);
            }
        }
    }
    out
}
