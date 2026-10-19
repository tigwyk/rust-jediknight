//! `.cog` — the level scripting language.
//!
//! Shipped as **plain text**, not bytecode: 540 files, a C-like language the engine
//! parses at level load (`sithCogParse.c` and a YACC grammar, per the module strings
//! in the EXE). Two sections:
//!
//! ```text
//! symbols
//! message    startup
//! message    activate
//! thing    door
//! thing    player    local
//! sound    locked0=i00ky73t.wav    local
//! flex    movespeed=8.0
//! int    locked=0    local
//! end
//!
//! code
//! startup:
//!     doorsector = GetThingSector(door);
//!     if (IsMoving(door)) return;
//!     call opendoor;
//!     return;
//! end
//! ```
//!
//! `message` symbols are the entry points: the engine calls the label of the same
//! name when the thing the COG is attached to receives that message (`startup`,
//! `activate`, `arrived`, `timer`, …). The rest are variables.
//!
//! This module parses that language and interprets it against a verb table. Verbs
//! the reconstruction does not implement return 0 and are recorded by name, so a
//! script runs to completion and its unimplemented surface is measurable instead of
//! silently wrong.

use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SymKind {
    Message,
    Thing,
    Sector,
    Surface,
    Sound,
    Template,
    Keyframe,
    Material,
    AiClass,
    Model,
    Cog,
    Int,
    Float,
    Flex,
    Vector,
    Str,
}

impl SymKind {
    fn parse(word: &str) -> Option<Self> {
        Some(match word {
            "message" => SymKind::Message,
            "thing" => SymKind::Thing,
            "sector" => SymKind::Sector,
            "surface" => SymKind::Surface,
            "sound" => SymKind::Sound,
            "template" => SymKind::Template,
            "keyframe" => SymKind::Keyframe,
            "material" => SymKind::Material,
            "ai" => SymKind::AiClass,
            "model" => SymKind::Model,
            "cog" => SymKind::Cog,
            "int" => SymKind::Int,
            "float" => SymKind::Float,
            "flex" => SymKind::Flex,
            "vector" => SymKind::Vector,
            "string" => SymKind::Str,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone)]
pub struct Symbol {
    pub kind: SymKind,
    pub name: String,
    /// `name=value` initialiser, as written.
    pub init: Option<String>,
    /// `local` symbols are per-instance; the rest are shared.
    pub local: bool,
    /// `linkid=N`: which element of a linked array this symbol is. In a script,
    /// `door0[i]` means "the symbol of this array whose linkid is i".
    pub linkid: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    And,
    Or,
    BitAnd,
    BitOr,
    BitXor,
    Shl,
    Shr,
}

impl BinOp {
    fn tightness(self) -> u8 {
        match self {
            BinOp::Or => 1,
            BinOp::And => 2,
            BinOp::BitOr => 3,
            BinOp::BitXor => 4,
            BinOp::BitAnd => 5,
            BinOp::Eq | BinOp::Ne => 6,
            BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => 7,
            BinOp::Shl | BinOp::Shr => 8,
            BinOp::Add | BinOp::Sub => 9,
            BinOp::Mul | BinOp::Div | BinOp::Mod => 10,
        }
    }
}

#[derive(Debug, Clone)]
pub enum UnOp {
    Neg,
    Not,
    BitNot,
}

#[derive(Debug, Clone)]
pub enum Expr {
    Num(f64),
    Str(String),
    Var(String),
    /// `name[index]` — the engine's indexed symbols resolve to consecutive files.
    Index(String, Box<Expr>),
    Call(String, Vec<Expr>),
    Bin(BinOp, Box<Expr>, Box<Expr>),
    Un(UnOp, Box<Expr>),
    /// Assignment used as a value, which the shipped scripts do (`if (bargh = 0)`).
    Assign(String, Box<Expr>),
}

#[derive(Debug, Clone)]
pub enum Stmt {
    /// `target [index] = value`
    Assign(String, Option<Box<Expr>>, Expr),
    /// A verb call used as a statement.
    Call(String, Vec<Expr>),
    If(Expr, Box<Stmt>, Option<Box<Stmt>>),
    While(Expr, Box<Stmt>),
    /// `for (init; cond; step) body` — C-style, and common in the shipped scripts
    /// (`for (i=0; i<=3; i=i+1)`).
    For(Box<Stmt>, Expr, Box<Stmt>, Box<Stmt>),
    DoWhile(Box<Stmt>, Expr),
    Return(Option<Expr>),
    Break,
    /// `call <label>;`
    Gosub(String),
    Block(Vec<Stmt>),
}

#[derive(Debug, Default)]
pub struct Cog {
    pub symbols: Vec<Symbol>,
    /// Entry points and subroutines, in file order.
    pub labels: Vec<(String, Vec<Stmt>)>,
}

impl Cog {
    pub fn label(&self, name: &str) -> Option<&Vec<Stmt>> {
        let want = name.to_ascii_lowercase();
        self.labels.iter().find(|(n, _)| *n == want).map(|(_, s)| s)
    }

    pub fn messages(&self) -> impl Iterator<Item = &Symbol> {
        self.symbols.iter().filter(|s| s.kind == SymKind::Message)
    }

    /// Bind a placed instance's values to this script.
    ///
    /// The level stores one row per instance: the script name followed by a value for
    /// every *non-message, non-local* symbol in declaration order. So
    /// `0: 00_door.cog 4 -1 -1 -1 8.0 2.0 0.5` fills `thing door0`=4,
    /// `thing door1..3`=-1, `float moveSpeed`=8.0, `sleepTime`=2.0, `lightValue`=0.5,
    /// while `local` symbols keep their initialisers. A `-1` leaves a thing or sector
    /// unbound.
    pub fn bind(&self, vm: &mut Vm, values: &[String]) {
        let mut next = values.iter();
        for sym in &self.symbols {
            // Locals keep their own state; messages are entry points, not data.
            if sym.local || sym.kind == SymKind::Message {
                continue;
            }
            let Some(raw) = next.next() else { break };
            let value = match sym.kind {
                SymKind::Thing | SymKind::Sector | SymKind::Surface | SymKind::Cog => {
                    Value::Num(raw.parse().unwrap_or(-1.0))
                }
                SymKind::Int => Value::Num(raw.parse().unwrap_or(0.0)),
                SymKind::Float | SymKind::Flex => Value::Num(raw.parse().unwrap_or(0.0)),
                SymKind::Vector => Value::Num(0.0),
                // `sound`/`template`/`material`/`keyframe`/`ai`/`model` name a file.
                _ => Value::Str(raw.clone()),
            };
            vm.set_var(&sym.name, value);
        }
        // Locals still need their declared initialisers, which are not all numbers:
        // `sound locked0=i00ky73t.wav local` is a filename, and a vector literal is
        // single-quoted (`'0.0 1.0 0.0'`). Parsing them as numbers silently turned
        // every sound name into 0.
        for sym in &self.symbols {
            if !sym.local {
                continue;
            }
            let Some(init) = &sym.init else { continue };
            let text = init.trim();
            let value = match text.parse::<f64>() {
                Ok(n) => Value::Num(n),
                Err(_) => {
                    let quoted = text.trim_matches(['\'', '"']);
                    let parts: Vec<f64> =
                        quoted.split_whitespace().filter_map(|p| p.parse().ok()).collect();
                    if text.starts_with('\'') && parts.len() == 3 {
                        Value::Vec([parts[0], parts[1], parts[2]])
                    } else {
                        Value::Str(quoted.to_string())
                    }
                }
            };
            vm.set_var(&sym.name, value);
        }
    }

    /// `door0` + index 2 → `door2`: the engine resolves an indexed symbol by link id.
    fn indexed_name(&self, name: &str, index: i64) -> Option<String> {
        let base = name.trim_end_matches(|c: char| c.is_ascii_digit());
        let want = index as u32;
        // Prefer an explicit `linkid=`; fall back to `<base><index>`.
        self.symbols
            .iter()
            .find(|s| s.linkid == Some(want) && s.name.starts_with(base))
            .map(|s| s.name.clone())
            .or_else(|| {
                let candidate = format!("{base}{index}");
                self.symbols.iter().any(|s| s.name == candidate).then_some(candidate)
            })
    }
}

pub fn parse(text: &str) -> Result<Cog, String> {
    let mut cog = Cog::default();
    let mut section = Section::None;
    let mut code_text = String::new();
    let mut saw_symbols = false;

    // The `symbols` section is line-oriented and its values are filenames and
    // numbers, so it is read as text; only the code section is tokenised.
    for raw in text.lines() {
        let without = raw.split_once('#').map(|(a, _)| a).unwrap_or(raw);
        let line = without.split_once("//").map(|(a, _)| a).unwrap_or(without).trim();
        if line.is_empty() {
            continue;
        }
        match line.to_ascii_lowercase().as_str() {
            "symbols" => {
                section = Section::Symbols;
                saw_symbols = true;
                continue;
            }
            "code" => {
                section = Section::Code;
                continue;
            }
            "end" => {
                section = Section::None;
                continue;
            }
            _ => {}
        }
        match section {
            Section::Symbols => parse_symbol_line(line, &mut cog)?,
            Section::Code => {
                // The original line, not the comment-stripped one: `#` and `//`
                // inside a quoted string are text, and only the tokeniser can tell.
                code_text.push_str(raw);
                code_text.push('\n');
            }
            Section::None => {}
        }
    }
    if !saw_symbols {
        return Err("no `symbols` section".into());
    }
    if code_text.is_empty() {
        return Err("no `code` section".into());
    }

    // ---- code ----
    let tokens = tokenize(&code_text)?;
    let mut i = 0;
    let mut current: Option<(String, Vec<Stmt>)> = None;
    while i < tokens.len() {
        if let (Some(Tok::Word(name)), Some(Tok::Punct(':'))) = (tokens.get(i), tokens.get(i + 1)) {
            if let Some((n, body)) = current.take() {
                cog.labels.push((n, body));
            }
            // COG is case-insensitive, and the shipped scripts are inconsistent about
            // it (`Startup:` vs `startup:`), so labels are held lowercase.
            current = Some((name.to_ascii_lowercase(), Vec::new()));
            i += 2;
            continue;
        }
        let body = current.get_or_insert_with(|| ("startup".to_string(), Vec::new()));
        body.1.push(parse_stmt(&tokens, &mut i)?);
    }
    if let Some((n, body)) = current.take() {
        cog.labels.push((n, body));
    }
    if cog.labels.is_empty() {
        return Err("no code labels".into());
    }
    Ok(cog)
}

enum Section {
    None,
    Symbols,
    Code,
}

/// One `symbols` line: `<type> <name>[=<init>] [local]`.
fn parse_symbol_line(line: &str, cog: &mut Cog) -> Result<(), String> {
    let mut parts = line.split_whitespace();
    let Some(type_word) = parts.next() else { return Ok(()) };
    let Some(kind) = SymKind::parse(&type_word.to_ascii_lowercase()) else {
        // Unknown declaration types are skipped rather than refused: the shipped
        // scripts are the spec, and one stray type must not hide the rest of a
        // level's scripts.
        return Ok(());
    };
    let Some(decl) = parts.next() else {
        return Err(format!("`{type_word}` declaration without a name"));
    };
    let (name, init) = match decl.split_once('=') {
        Some((n, v)) => (n, Some(v.to_string())),
        None => (decl, None),
    };
    let local = parts.any(|p| p.eq_ignore_ascii_case("local"));
    let linkid = line
        .split_whitespace()
        .find_map(|p| p.strip_prefix("linkid="))
        .and_then(|v| v.parse().ok());
    cog.symbols.push(Symbol { kind, name: name.to_ascii_lowercase(), init, local, linkid });
    Ok(())
}

fn parse_stmt(tokens: &[Tok], i: &mut usize) -> Result<Stmt, String> {
    let Some(tok) = tokens.get(*i) else {
        return Err("unexpected end of code".into());
    };
    match tok {
        Tok::Punct('{') => {
            *i += 1;
            let mut body = Vec::new();
            while *i < tokens.len() && tokens[*i] != Tok::Punct('}') {
                body.push(parse_stmt(tokens, i)?);
            }
            *i += 1;
            Ok(Stmt::Block(body))
        }
        Tok::Punct(';') => {
            *i += 1;
            Ok(Stmt::Block(Vec::new()))
        }
        // COG is case-insensitive: the shipped scripts use `For`/`for`, `Print`/`print`
        // and `thinglight`/`ThingLight` interchangeably.
        Tok::Word(raw_word) => match raw_word.to_ascii_lowercase().as_str() {
            "if" => {
                *i += 1;
                let cond = parse_paren_expr(tokens, i)?;
                let then = Box::new(parse_stmt(tokens, i)?);
                let mut els = None;
                if let Some(Tok::Word(e)) = tokens.get(*i) {
                    if e == "else" {
                        *i += 1;
                        els = Some(Box::new(parse_stmt(tokens, i)?));
                    }
                }
                Ok(Stmt::If(cond, then, els))
            }
            "while" => {
                *i += 1;
                let cond = parse_paren_expr(tokens, i)?;
                let body = Box::new(parse_stmt(tokens, i)?);
                Ok(Stmt::While(cond, body))
            }
            "for" => {
                *i += 1;
                if !matches!(tokens.get(*i), Some(Tok::Punct('('))) {
                    return Err("expected `(` after `for`".into());
                }
                *i += 1;
                let init = Box::new(parse_stmt(tokens, i)?);
                let cond = parse_expr(tokens, i, 0)?;
                if matches!(tokens.get(*i), Some(Tok::Punct(';'))) {
                    *i += 1;
                }
                let step = Box::new(parse_stmt(tokens, i)?);
                if !matches!(tokens.get(*i), Some(Tok::Punct(')'))) {
                    return Err("expected `)` after `for` header".into());
                }
                *i += 1;
                let body = Box::new(parse_stmt(tokens, i)?);
                Ok(Stmt::For(init, cond, step, body))
            }
            "do" => {
                *i += 1;
                let body = Box::new(parse_stmt(tokens, i)?);
                if let Some(Tok::Word(w)) = tokens.get(*i) {
                    if w != "while" {
                        return Err(format!("expected `while` after `do`, got `{w}`"));
                    }
                }
                *i += 1;
                let cond = parse_paren_expr(tokens, i)?;
                if matches!(tokens.get(*i), Some(Tok::Punct(';'))) {
                    *i += 1;
                }
                Ok(Stmt::DoWhile(body, cond))
            }
            "return" => {
                *i += 1;
                if matches!(tokens.get(*i), Some(Tok::Punct(';'))) {
                    *i += 1;
                    return Ok(Stmt::Return(None));
                }
                let e = parse_expr(tokens, i, 0)?;
                if matches!(tokens.get(*i), Some(Tok::Punct(';'))) {
                    *i += 1;
                }
                Ok(Stmt::Return(Some(e)))
            }
            "break" => {
                *i += 1;
                if matches!(tokens.get(*i), Some(Tok::Punct(';'))) {
                    *i += 1;
                }
                Ok(Stmt::Break)
            }
            "call" => {
                *i += 1;
                let name = match tokens.get(*i) {
                    Some(Tok::Word(n)) => n.clone(),
                    _ => return Err("`call` without a label".into()),
                };
                *i += 1;
                if matches!(tokens.get(*i), Some(Tok::Punct(';'))) {
                    *i += 1;
                }
                Ok(Stmt::Gosub(name))
            }
            _ => {
                // Assignment or a verb call used as a statement.
                let name = raw_word.clone();
                *i += 1;
                let mut index = None;
                if matches!(tokens.get(*i), Some(Tok::Punct('['))) {
                    *i += 1;
                    index = Some(Box::new(parse_expr(tokens, i, 0)?));
                    if !matches!(tokens.get(*i), Some(Tok::Punct(']'))) {
                        return Err("unterminated `[` in assignment".into());
                    }
                    *i += 1;
                }
                if matches!(tokens.get(*i), Some(Tok::Punct('='))) {
                    *i += 1;
                    let value = parse_expr(tokens, i, 0)?;
                    if matches!(tokens.get(*i), Some(Tok::Punct(';'))) {
                        *i += 1;
                    }
                    return Ok(Stmt::Assign(name, index, value));
                }
                if matches!(tokens.get(*i), Some(Tok::Punct('('))) {
                    *i += 1;
                    let args = parse_args(tokens, i)?;
                    if matches!(tokens.get(*i), Some(Tok::Punct(';'))) {
                        *i += 1;
                    }
                    return Ok(Stmt::Call(name, args));
                }
                // A bare value is legal (e.g. `Rand();` assembled oddly); ignore.
                if matches!(tokens.get(*i), Some(Tok::Punct(';'))) {
                    *i += 1;
                }
                Ok(Stmt::Block(Vec::new()))
            }
        },
        other => Err(format!("unexpected token {other:?} at statement start")),
    }
}

fn parse_paren_expr(tokens: &[Tok], i: &mut usize) -> Result<Expr, String> {
    if !matches!(tokens.get(*i), Some(Tok::Punct('('))) {
        return Err(format!("expected `(` near {:?}", context(tokens, *i)));
    }
    *i += 1;
    let e = parse_expr(tokens, i, 0)?;
    if !matches!(tokens.get(*i), Some(Tok::Punct(')'))) {
        return Err(format!("expected `)` near {:?}", context(tokens, *i)));
    }
    *i += 1;
    Ok(e)
}

/// A few tokens around a position, for error messages that can be acted on.
fn context(tokens: &[Tok], i: usize) -> Vec<String> {
    let start = i.saturating_sub(4);
    tokens[start..(i + 3).min(tokens.len())]
        .iter()
        .map(|t| t.render())
        .collect()
}

fn parse_args(tokens: &[Tok], i: &mut usize) -> Result<Vec<Expr>, String> {
    let mut args = Vec::new();
    if matches!(tokens.get(*i), Some(Tok::Punct(')'))) {
        *i += 1;
        return Ok(args);
    }
    loop {
        args.push(parse_expr(tokens, i, 0)?);
        match tokens.get(*i) {
            Some(Tok::Punct(',')) => {
                *i += 1;
            }
            Some(Tok::Punct(')')) => {
                *i += 1;
                return Ok(args);
            }
            other => return Err(format!("expected `,` or `)` in arguments, got {other:?}")),
        }
    }
}

fn parse_expr(tokens: &[Tok], i: &mut usize, min: u8) -> Result<Expr, String> {
    let mut lhs = parse_unary(tokens, i)?;
    // `name = value` inside an expression: the shipped scripts use it as a
    // condition (`if (bargh = 0)`), so it is accepted rather than rejected.
    if min == 0 {
        if let (Expr::Var(name), Some(Tok::Punct('='))) = (&lhs, tokens.get(*i)) {
            let name = name.clone();
            *i += 1;
            let rhs = parse_expr(tokens, i, 0)?;
            return Ok(Expr::Assign(name, Box::new(rhs)));
        }
    }
    while let Some(op) = tokens.get(*i).and_then(bin_op) {
        if op.tightness() < min {
            break;
        }
        *i += 1;
        let rhs = parse_expr(tokens, i, op.tightness() + 1)?;
        lhs = Expr::Bin(op, Box::new(lhs), Box::new(rhs));
    }
    Ok(lhs)
}

fn parse_unary(tokens: &[Tok], i: &mut usize) -> Result<Expr, String> {
    match tokens.get(*i) {
        Some(Tok::Punct('-')) => {
            *i += 1;
            Ok(Expr::Un(UnOp::Neg, Box::new(parse_unary(tokens, i)?)))
        }
        Some(Tok::Punct('!')) => {
            *i += 1;
            Ok(Expr::Un(UnOp::Not, Box::new(parse_unary(tokens, i)?)))
        }
        Some(Tok::Punct('~')) => {
            *i += 1;
            Ok(Expr::Un(UnOp::BitNot, Box::new(parse_unary(tokens, i)?)))
        }
        Some(Tok::Punct('(')) => {
            *i += 1;
            let e = parse_expr(tokens, i, 0)?;
            if !matches!(tokens.get(*i), Some(Tok::Punct(')'))) {
                return Err("expected `)`".into());
            }
            *i += 1;
            Ok(e)
        }
        Some(Tok::Num(n)) => {
            *i += 1;
            Ok(Expr::Num(*n))
        }
        Some(Tok::Str(s)) => {
            *i += 1;
            Ok(Expr::Str(s.clone()))
        }
        Some(Tok::Word(w)) => {
            let name = w.clone();
            *i += 1;
            if matches!(tokens.get(*i), Some(Tok::Punct('('))) {
                *i += 1;
                let args = parse_args(tokens, i)?;
                return Ok(Expr::Call(name, args));
            }
            if matches!(tokens.get(*i), Some(Tok::Punct('['))) {
                *i += 1;
                let idx = parse_expr(tokens, i, 0)?;
                if !matches!(tokens.get(*i), Some(Tok::Punct(']'))) {
                    return Err("expected `]`".into());
                }
                *i += 1;
                return Ok(Expr::Index(name, Box::new(idx)));
            }
            Ok(Expr::Var(name))
        }
        other => Err(format!("unexpected token {other:?} in expression")),
    }
}

fn bin_op(tok: &Tok) -> Option<BinOp> {
    Some(match tok {
        Tok::Punct('+') => BinOp::Add,
        Tok::Punct('-') => BinOp::Sub,
        Tok::Punct('*') => BinOp::Mul,
        Tok::Punct('/') => BinOp::Div,
        Tok::Punct('%') => BinOp::Mod,
        Tok::Punct('&') => BinOp::BitAnd,
        Tok::Punct('|') => BinOp::BitOr,
        Tok::Punct('^') => BinOp::BitXor,
        Tok::Op("==") => BinOp::Eq,
        Tok::Op("!=") => BinOp::Ne,
        Tok::Op("<") => BinOp::Lt,
        Tok::Op("<=") => BinOp::Le,
        Tok::Op(">") => BinOp::Gt,
        Tok::Op(">=") => BinOp::Ge,
        Tok::Op("&&") => BinOp::And,
        Tok::Op("||") => BinOp::Or,
        Tok::Op("<<") => BinOp::Shl,
        Tok::Op(">>") => BinOp::Shr,
        _ => return None,
    })
}

#[derive(Debug, Clone, PartialEq)]
pub enum Tok {
    Word(String),
    Str(String),
    Num(f64),
    /// Two-character operator, e.g. `==`.
    Op(&'static str),
    Punct(char),
}

impl Tok {
    fn render(&self) -> String {
        match self {
            Tok::Word(w) => w.clone(),
            Tok::Str(s) => format!("\"{s}\""),
            Tok::Num(n) => n.to_string(),
            Tok::Op(o) => (*o).to_string(),
            Tok::Punct(c) => c.to_string(),
        }
    }
}

fn tokenize(text: &str) -> Result<Vec<Tok>, String> {
    let bytes: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        // Comments: `#` to end of line, `//` to end of line, `/* … */`.
        if c == '#' || (c == '/' && bytes.get(i + 1) == Some(&'/')) {
            while i < bytes.len() && bytes[i] != '\n' {
                i += 1;
            }
            continue;
        }
        if c == '/' && bytes.get(i + 1) == Some(&'*') {
            i += 2;
            while i + 1 < bytes.len() && !(bytes[i] == '*' && bytes[i + 1] == '/') {
                i += 1;
            }
            i = (i + 2).min(bytes.len());
            continue;
        }
        if c.is_ascii_digit() || (c == '.' && bytes.get(i + 1).is_some_and(|d| d.is_ascii_digit())) {
            let start = i;
            while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == '.') {
                i += 1;
            }
            let raw: String = bytes[start..i].iter().collect();
            let n = match raw.strip_prefix("0x").or_else(|| raw.strip_prefix("0X")) {
                Some(hex) => i64::from_str_radix(hex, 16)
                    .map(|v| v as f64)
                    .map_err(|_| format!("bad hex number `{raw}`"))?,
                None => raw.parse::<f64>().map_err(|_| format!("bad number `{raw}`"))?,
            };
            out.push(Tok::Num(n));
            continue;
        }
        if c.is_ascii_alphabetic() || c == '_' {
            let start = i;
            while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == '_') {
                i += 1;
            }
            out.push(Tok::Word(bytes[start..i].iter().collect()));
            continue;
        }
        // Strings: COG uses `"…"` for text and `'…'` for vector literals such as
        // `'0.0 0.0 0.0'`, so both delimiters are equivalent here.
        if c == '"' || c == '\'' {
            let quote = c;
            i += 1;
            let mut s = String::new();
            while i < bytes.len() && bytes[i] != quote {
                s.push(bytes[i]);
                i += 1;
            }
            i += 1;
            out.push(Tok::Str(s));
            continue;
        }
        // Operators, longest first.
        let two: String = bytes[i..(i + 2).min(bytes.len())].iter().collect();
        let op = match two.as_str() {
            "==" => Some("=="),
            "!=" => Some("!="),
            "<=" => Some("<="),
            ">=" => Some(">="),
            "&&" => Some("&&"),
            "||" => Some("||"),
            "<<" => Some("<<"),
            ">>" => Some(">>"),
            _ => None,
        };
        if let Some(op) = op {
            out.push(Tok::Op(op));
            i += 2;
            continue;
        }
        match c {
            '<' | '>' => out.push(Tok::Op(if c == '<' { "<" } else { ">" })),
            '(' | ')' | '{' | '}' | '[' | ']' | ';' | ',' | '=' | ':' => out.push(Tok::Punct(c)),
            '+' | '-' | '*' | '/' | '%' | '!' | '~' | '&' | '|' | '^' => out.push(Tok::Punct(c)),
            // Anything else (stray control characters in the shipped files) is skipped.
            _ => {}
        }
        i += 1;
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// interpretation
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Nil,
    Num(f64),
    Str(String),
    Vec([f64; 3]),
}

impl Value {
    pub fn num(&self) -> f64 {
        match self {
            Value::Num(n) => *n,
            Value::Vec(v) => v[0],
            Value::Nil => 0.0,
            Value::Str(s) => s.parse().unwrap_or(0.0),
        }
    }

    fn truthy(&self) -> bool {
        match self {
            Value::Num(n) => *n != 0.0,
            Value::Nil => false,
            Value::Str(s) => !s.is_empty(),
            Value::Vec(_) => true,
        }
    }
}

/// What a verb call did, so callers can log the unimplemented surface.
#[derive(Default)]
pub struct Trace {
    /// Verb name -> call count.
    pub calls: HashMap<String, usize>,
    /// Verbs neither the world nor the builtins implement, and their call counts.
    pub unimplemented: std::collections::BTreeMap<String, usize>,
    /// Text written by `Print`.
    pub prints: Vec<String>,
}

/// Iterations a single loop may run before the script is treated as waiting on an
/// engine event. COG code is full of `while (1) { … Sleep(0.1); }` state machines.
const LOOP_BUDGET: u32 = 20_000;

/// Statements one `run_label` may execute before it is treated as still waiting.
///
/// Sized so a runaway `while (1)` costs a fraction of a frame instead of the level: 20,000 statements
/// is well over what any shipped label needs to reach its next wait.
const LABEL_BUDGET: u64 = 20_000;

/// Set to 1 by `--pull` to count statements, so a label that will not return can be classified:
/// a climbing count is a loop (the budget bounds it) and a frozen one is a verb blocking (nothing
/// does). Off by default — one atomic increment per statement is not free.
pub static HEARTBEAT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// See [HEARTBEAT].
pub static STMTS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Marker for "this script is waiting on the engine", not broken.
pub const WAITING: &str = "waiting on an engine event";

/// A COG running against a verb table.
pub struct Vm {
    vars: HashMap<String, Value>,
    /// Values the *engine* can read back from the world (thing positions, sectors).
    pub trace: Trace,
    depth: u32,
    /// Statements left before the run is abandoned. COG scripts are event-driven and
    /// many end in `while (1)` waiting for a timer or message, so a budget is the
    /// only thing that keeps a headless run from spinning forever.
    budget: u64,
    /// The world the verbs act on, when there is one.
    world: Option<crate::runtime::World>,
    /// Who sent the message being handled, as the engine's cog keeps it: the sender's
    /// thing index (`senderId`), its ref, and its type. `GetSenderRef`/`GetSenderID`/
    /// `GetSenderType` read these. Zero means "not from a thing".
    pub sender: i64,
    pub sender_ref: i64,
    pub sender_type: i64,
    /// The thing the script instance is placed on (`GetSourceRef`); 0 when it is
    /// level-wide, which is every instance this level's parse exposes.
    pub source: i64,
    /// This cog's pulse, as the engine keeps it (`SITH_COG_PULSE_SET`): `SetPulse(seconds)` sets
    /// the interval and the next due time, and every cog tick that is due sends the `pulse` message.
    pub pulse: Option<(f32, f32)>,
    /// This cog's one-shot timer (`SITH_COG_TIMER_SET`): the `timer` message fires once when due.
    pub timer: Option<f32>,
    /// Seconds this cog has been running, which is what pulse and timer deadlines are measured against.
    pub now: f32,
    /// When a `sleep` in the interrupted label may continue.
    pub sleep_until: f32,
    /// `(label, statement index, due)`: what a `sleep` interrupted. The engine keeps the same thing as
    /// `script_running == 2` plus a saved instruction pointer (§16.10).
    pub resume: Option<(String, usize, f32)>,
    /// Set by `sleep`; the call sites turn it into the `SLEEPING` unwind.
    suspended: bool,
    /// Set when a resume re-enters the label: the next `sleep` was already waited out.
    sleep_skip: bool,
}

/// The sentinel a `sleep` unwinds with, so `run_label` can tell "waiting" from a script error.
const SLEEPING: &str = "::sleeping";



impl Vm {
    pub fn new() -> Self {
        Self {
            vars: HashMap::new(),
            trace: Trace::default(),
            depth: 0,
            budget: 2_000_000,
            world: None,
            sender: 0,
            sender_ref: 0,
            sender_type: 0,
            source: 0,
            pulse: None,
            timer: None,
            now: 0.0,
            sleep_until: 0.0,
            resume: None,
            suspended: false,
            sleep_skip: false,

        }
    }

    /// Give the VM a world to act on. Without one, world verbs answer 0 (which is
    /// how the parser and language are tested in isolation).
    pub fn with_world(world: crate::runtime::World) -> Self {
        Self { world: Some(world), ..Self::new() }
    }

    pub fn var(&self, name: &str) -> Value {
        self.vars.get(&name.to_ascii_lowercase()).cloned().unwrap_or(Value::Nil)
    }

    pub fn set_var(&mut self, name: &str, value: Value) {
        self.vars.insert(name.to_ascii_lowercase(), value);
    }


    /// Whether this cog is in the middle of a `sleep`: it should be resumed, not re-entered.
    pub fn sleeping(&self) -> bool {
        self.resume.as_ref().is_some_and(|(_, _, due)| self.now < *due)
    }

    /// The label a `sleep` interrupted, if any.
    pub fn sleeping_label(&self) -> Option<&str> {
        self.resume.as_ref().map(|(name, _, _)| name.as_str())
    }

    /// Run one label to completion. Errors are returned rather than panicking: a
    /// single bad script must not take the whole level down.
    pub fn run_label(&mut self, cog: &Cog, label: &str) -> Result<Value, String> {
        let Some(body) = cog.label(label) else {
            return Err(format!("no label `{label}`"));
        };
        // Every label gets its own budget, and the budget is *small enough to bound a frame*.
        //
        // The engine's cogs are coroutines: a `while (1)` label yields and resumes when its message
        // arrives, so it can sit in a loop forever without costing the frame. This VM cannot suspend
        // mid-label, so the same script is cut at `LABEL_BUDGET` statements and reported as waiting —
        // and the budget is refilled here, or one runaway label would leave the VM dead for the rest
        // of the level (`budget` used to be a one-shot 2,000,000 for the whole life of the cog).
        //
        // **One refill, at the outermost call.** `cog\force_pull.cog`'s `activated` hung the harness
        // for as long as it was left running: it recurses through `SendMessage` into itself, and every
        // nested `run_label` refilled the budget, so the 20,000-statement cut never fired. The limit is
        // per *frame*, not per call, which is what it has to be to bound anything.
        if self.depth == 0 {
            self.budget = std::env::var("BK_COG_BUDGET")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(LABEL_BUDGET);
        }
        if self.depth > 24 {
            return Err(format!("call depth limit in `{label}`"));
        }
        // Resume where a `sleep` stopped this label, or start at the top.
        let want = label.to_ascii_lowercase();
        let start = match self.resume.take() {
            Some((name, at, due)) if name == want => {
                if self.now < due {
                    self.resume = Some((name, at, due));
                    return Err(SLEEPING.into());
                }
                self.sleep_skip = true;
                at
            }
            _ => 0,
        };
        self.depth += 1;
        let started = std::time::Instant::now();
        let mut result = Ok(Value::Nil);
        for i in start..body.len() {
            match self.exec(cog, &body[i]) {
                Ok(Flow::Next) => {}
                Ok(Flow::Break) => break,
                Ok(Flow::Return(v)) => {
                    result = Ok(v);
                    break;
                }
                Err(e) => {
                    if e == SLEEPING {
                        // Resume at the statement that slept; its `sleep` is transparent the first
                        // time back, so a top-level wait continues past it and a `while` loop's
                        // next iteration still runs its body.
                        self.resume = Some((want.clone(), i, self.sleep_until));
                        result = Ok(Value::Nil);
                    } else {
                        result = Err(e);
                    }
                    break;
                }
            }
        }
        self.depth -= 1;
        // `BK_COG_TRACE`: name any label that cost a noticeable slice of a frame, which is how a
        // script that loops until its budget runs out shows up without a profiler.
        if std::env::var_os("BK_COG_TRACE").is_some() {
            let ms = started.elapsed().as_secs_f32() * 1000.0;
            if ms > 5.0 {
                println!("cog: `{label}` took {ms:.0} ms");
            }
        }
        result
    }

    fn exec_block(&mut self, cog: &Cog, body: &[Stmt]) -> Result<Value, String> {
        for stmt in body {
            match self.exec(cog, stmt)? {
                Flow::Next => {}
                Flow::Return(v) => return Ok(v),
                Flow::Break => return Ok(Value::Nil),
            }
        }
        Ok(Value::Nil)
    }

    fn exec(&mut self, cog: &Cog, stmt: &Stmt) -> Result<Flow, String> {
        if self.budget == 0 {
            return Err(WAITING.into());
        }
        self.budget -= 1;
        // `--pull` needs to know whether a label that will not return is *looping* (this climbs) or
        // *blocked in a verb* (this freezes) — the budget bounds the first and nothing bounds the
        // second. Off by default: one relaxed increment per statement is not free.
        if HEARTBEAT.load(std::sync::atomic::Ordering::Relaxed) > 0 {
            STMTS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
        match stmt {
            Stmt::Block(body) => {
                for s in body {
                    match self.exec(cog, s)? {
                        Flow::Next => {}
                        done => return Ok(done),
                    }
                }
                Ok(Flow::Next)
            }
            Stmt::Assign(name, index, value) => {
                let v = self.eval(cog, value)?;
                let v = match index {
                    // Indexed symbols select consecutive files; keep the index
                    // `door0[i] = …` writes the i-th linked symbol.
                    Some(idx) => {
                        let i = self.eval(cog, idx)?.num() as i64;
                        let target = cog
                            .indexed_name(name, i)
                            .unwrap_or_else(|| format!("{name}[{i}]"));
                        self.vars.insert(target, v);
                        return Ok(Flow::Next);
                    }
                    None => v,
                };
                self.vars.insert(name.to_ascii_lowercase(), v);
                Ok(Flow::Next)
            }
            Stmt::Call(name, args) => {
                let vals = args.iter().map(|a| self.eval(cog, a)).collect::<Result<Vec<_>, _>>()?;
                self.call_verb(name, vals);
                if std::mem::take(&mut self.suspended) {
                    return Err(SLEEPING.into());
                }
                Ok(Flow::Next)
            }
            Stmt::If(cond, then, els) => {
                let c = self.eval(cog, cond)?;
                if c.truthy() {
                    self.exec(cog, then)
                } else if let Some(e) = els {
                    self.exec(cog, e)
                } else {
                    Ok(Flow::Next)
                }
            }
            Stmt::While(cond, body) => {
                for _ in 0..LOOP_BUDGET {
                    if !self.eval(cog, cond)?.truthy() {
                        return Ok(Flow::Next);
                    }
                    match self.exec(cog, body)? {
                        Flow::Next => {}
                        Flow::Break => return Ok(Flow::Next),
                        done => return Ok(done),
                    }
                }
                Err(WAITING.into())
            }
            Stmt::For(init, cond, step, body) => {
                self.exec(cog, init)?;
                for _ in 0..LOOP_BUDGET {
                    if !self.eval(cog, cond)?.truthy() {
                        return Ok(Flow::Next);
                    }
                    match self.exec(cog, body)? {
                        Flow::Next => {}
                        Flow::Break => return Ok(Flow::Next),
                        done => return Ok(done),
                    }
                    self.exec(cog, step)?;
                }
                Err(WAITING.into())
            }
            Stmt::DoWhile(body, cond) => {
                for _ in 0..LOOP_BUDGET {
                    match self.exec(cog, body)? {
                        Flow::Next => {}
                        Flow::Break => return Ok(Flow::Next),
                        done => return Ok(done),
                    }
                    if !self.eval(cog, cond)?.truthy() {
                        return Ok(Flow::Next);
                    }
                }
                Err(WAITING.into())
            }
            Stmt::Return(v) => {
                let v = match v {
                    Some(e) => self.eval(cog, e)?,
                    None => Value::Nil,
                };
                Ok(Flow::Return(v))
            }
            Stmt::Break => Ok(Flow::Break),
            Stmt::Gosub(label) => {
                self.run_label(cog, label)?;
                Ok(Flow::Next)
            }
        }
    }

    fn eval(&mut self, cog: &Cog, expr: &Expr) -> Result<Value, String> {
        Ok(match expr {
            Expr::Num(n) => Value::Num(*n),
            Expr::Str(s) => Value::Str(s.clone()),
            Expr::Var(name) => self.var(name),
            Expr::Index(name, idx) => {
                let i = self.eval(cog, idx)?.num() as i64;
                match cog.indexed_name(name, i) {
                    Some(target) => self.var(&target),
                    None => Value::Nil,
                }
            }
            Expr::Call(name, args) => {
                let vals = args.iter().map(|a| self.eval(cog, a)).collect::<Result<Vec<_>, _>>()?;
                self.call_verb(name, vals)
            }
            Expr::Un(op, e) => {
                let v = self.eval(cog, e)?.num();
                match op {
                    UnOp::Neg => Value::Num(-v),
                    UnOp::Not => Value::Num((v == 0.0) as u8 as f64),
                    UnOp::BitNot => Value::Num(!(v as i64) as f64),
                }
            }
            Expr::Assign(name, value) => {
                let v = self.eval(cog, value)?;
                self.set_var(name, v.clone());
                v
            }
            Expr::Bin(op, a, b) => {
                let x = self.eval(cog, a)?;
                let y = self.eval(cog, b)?;
                // `&&`/`||` short-circuit, which scripts rely on to guard calls.
                if *op == BinOp::And {
                    return Ok(Value::Num((x.truthy() && y.truthy()) as u8 as f64));
                }
                if *op == BinOp::Or {
                    return Ok(Value::Num((x.truthy() || y.truthy()) as u8 as f64));
                }
                let (a, b) = (x.num(), y.num());
                Value::Num(match op {
                    BinOp::Add => a + b,
                    BinOp::Sub => a - b,
                    BinOp::Mul => a * b,
                    BinOp::Div => {
                        if b == 0.0 {
                            0.0
                        } else {
                            a / b
                        }
                    }
                    BinOp::Mod => {
                        if b == 0.0 {
                            0.0
                        } else {
                            a % b
                        }
                    }
                    BinOp::Eq => (a == b) as u8 as f64,
                    BinOp::Ne => (a != b) as u8 as f64,
                    BinOp::Lt => (a < b) as u8 as f64,
                    BinOp::Le => (a <= b) as u8 as f64,
                    BinOp::Gt => (a > b) as u8 as f64,
                    BinOp::Ge => (a >= b) as u8 as f64,
                    BinOp::BitAnd => ((a as i64) & (b as i64)) as f64,
                    BinOp::BitOr => ((a as i64) | (b as i64)) as f64,
                    BinOp::BitXor => ((a as i64) ^ (b as i64)) as f64,
                    BinOp::Shl => ((a as i64) << (b as u32 & 63)) as f64,
                    BinOp::Shr => ((a as i64) >> (b as u32 & 63)) as f64,
                    BinOp::And | BinOp::Or => unreachable!(),
                })
            }
        })
    }

    /// Verb dispatch.
    ///
    /// `Print` is the one that matters for verification: it is the engine's own
    /// debug output, so a script that runs to the point of printing has executed
    /// real game logic. The rest are either implemented (pure math, no world) or
    /// recorded and answered with 0.
    fn call_verb(&mut self, name: &str, args: Vec<Value>) -> Value {
        *self.trace.calls.entry(name.to_string()).or_default() += 1;
        // World-facing verbs go to the runtime first: it knows the things, sectors
        // and inventory the script is talking about. The pure builtins below run
        // when the runtime does not implement the verb.
        // Pulse and timer state belongs to the *cog*, not the world (§16.10): `SetPulse(seconds)`
        // arms an interval that sends this cog's `pulse` message, `SetTimer(seconds)` arms a
        // one-shot `timer`. `sithCogFunction.c` keeps both on the cog struct.
        match name.to_ascii_lowercase().as_str() {
            "setpulse" => {
                let value = args.first().map(|v| v.num()).unwrap_or(0.0) as f32;
                self.pulse = (value > 0.0).then(|| (value, self.now + value));
                return Value::Nil;
            }

            // `Sleep(seconds)` and `WaitForMessage()` suspend the cog, as `sithCog_ProcessCog` does:
            // the label stops at the call and `run_label` records where to pick it up. The first
            // `sleep` after a resume was already waited out, so it is transparent — otherwise
            // resuming *at* the sleeping statement re-armed the same wait forever.
            "sleep" | "waitformessage" => {
                if std::mem::take(&mut self.sleep_skip) {
                    return Value::Nil;
                }
                let seconds = args.first().map(|v| v.num()).unwrap_or(0.0) as f32;
                self.sleep_until = self.now + seconds.max(0.0);
                self.suspended = true;
                return Value::Nil;
            }
            "settimer" | "settimerex" => {
                let value = args.first().map(|v| v.num()).unwrap_or(0.0) as f32;
                self.timer = (value > 0.0).then_some(self.now + value);
                return Value::Nil;
            }
            _ => {}
        }
        if let Some(world) = &self.world {
            if crate::runtime::Runtime::handles(name) {
                return world.borrow_mut().call(name, &args);
            }
        }
        match name.to_ascii_lowercase().as_str() {
            // The message's sender, as the engine's cog stores it when a message is
            // delivered (`sithCogFunction_GetSenderID` reads `pCog->senderId`).
            // `GetSourceRef` is the thing the script instance is placed on. A `.jkl`
            // does not record that link in what this parser keeps (the engine stores it
            // in the thing's `cog=` field), so it answers 0: "no source thing", which is
            // also what a level-wide script has. Message senders default to 0 too, so a
            // script comparing the two still matches for engine-triggered events.
            "getsourceref" => Value::Num(self.source as f64),
            "jkgetlocalplayer" => match &self.world {
                Some(world) => world.borrow_mut().call("getlocalplayerthing", &[]),
                None => Value::Num(0.0),
            },
            "getsenderid" => Value::Num(self.sender as f64),
            "getsenderref" => Value::Num(self.sender_ref as f64),
            "getsendertype" => Value::Num(self.sender_type as f64),
            "print" => {
                let text: Vec<String> = args
                    .iter()
                    .map(|v| match v {
                        Value::Str(s) => s.clone(),
                        Value::Num(n) => format!("{n}"),
                        Value::Nil => "nil".into(),
                        Value::Vec(v) => format!("[{}, {}, {}]", v[0], v[1], v[2]),
                    })
                    .collect();
                self.trace.prints.push(text.join(" "));
                Value::Nil
            }
            "rand" => Value::Num(pseudo_random()),
            "getparam" => args.first().cloned().unwrap_or(Value::Nil),
            "floor" => Value::Num(args.first().map(|v| v.num().floor()).unwrap_or(0.0)),
            "ceil" => Value::Num(args.first().map(|v| v.num().ceil()).unwrap_or(0.0)),
            "abs" => Value::Num(args.first().map(|v| v.num().abs()).unwrap_or(0.0)),
            "sqrt" => Value::Num(args.first().map(|v| v.num().max(0.0).sqrt()).unwrap_or(0.0)),
            "vectorlen" => match args.first() {
                Some(Value::Vec(v)) => Value::Num((v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt()),
                _ => Value::Num(0.0),
            },
            "vectorset" => match (args.first(), args.get(1), args.get(2), args.get(3)) {
                (Some(Value::Str(name)), Some(x), Some(y), Some(z)) => {
                    self.vars.insert(name.to_ascii_lowercase(), Value::Vec([x.num(), y.num(), z.num()]));
                    Value::Nil
                }
                _ => Value::Num(0.0),
            },
            // Neither the world nor the builtins know this verb: record it so the
            // unimplemented surface stays measurable, and answer 0 like a null.
            _ => {
                *self.trace.unimplemented.entry(name.to_ascii_lowercase()).or_default() += 1;
                Value::Num(0.0)
            }
        }
    }
}

enum Flow {
    Next,
    Return(Value),
    Break,
}

/// Deterministic stand-in for the engine's RNG: scripts call `Rand()` to pick
/// variants, and reproducible runs are worth more than true randomness here.
fn pseudo_random() -> f64 {
    use std::cell::Cell;
    thread_local! {
        static SEED: Cell<u32> = const { Cell::new(0x1234_5678) };
    }
    SEED.with(|s| {
        let mut x = s.get();
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        s.set(x);
        (x % 10_000) as f64 / 10_000.0
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const DOOR: &str = r#"
# 1-way Door Script
symbols
message	startup
message	activate
message	timer

thing		door
thing		player	local
sector		inside
sector		doorsector	local
sound		locked0=i00ky73t.wav	local
flex		movespeed=8.0
int		locked=0	local
end

code
startup:
	doorsector = GetThingSector(door);
	SectorAdjoins(doorsector, 0);
	return;

activate:
	if (IsMoving(door)) return;
	if (GetThingSector(GetLocalPlayerThing()) == inside)
	{
		call opendoor;
	}
	else
	{
		locked = 1;
	}
	return;

opendoor:
	MoveToFrame(door, 1, movespeed);
	return;
end
"#;

    #[test]
    fn parses_the_shipped_door_script() {
        let cog = parse(DOOR).expect("parses");
        assert_eq!(cog.symbols.len(), 10);
        assert_eq!(cog.messages().count(), 3);
        assert!(cog.label("startup").is_some());
        assert!(cog.label("opendoor").is_some());
        let door = cog.symbols.iter().find(|s| s.name == "door").unwrap();
        assert_eq!(door.kind, SymKind::Thing);
        let locked = cog.symbols.iter().find(|s| s.name == "locked").unwrap();
        assert!(locked.local);
        assert_eq!(locked.init.as_deref(), Some("0"));
    }

    #[test]
    fn runs_a_label_and_calls_subroutines() {
        let cog = parse(DOOR).expect("parses");
        let mut vm = Vm::new();
        vm.set_var("inside", Value::Num(7.0));
        // `IsMoving` is unimplemented and answers 0, so `activate` reaches the
        // comparison and takes the `else` branch.
        vm.run_label(&cog, "activate").expect("runs");
        assert_eq!(vm.var("locked"), Value::Num(1.0));
        assert_eq!(vm.trace.calls.get("IsMoving").copied(), Some(1));
    }

    #[test]
    fn print_reaches_the_trace() {
        let cog = parse("symbols\nmessage startup\nend\ncode\nstartup:\n\tPrint(\"door shut\");\n\treturn;\nend\n")
            .expect("parses");
        let mut vm = Vm::new();
        vm.run_label(&cog, "startup").expect("runs");
        assert_eq!(vm.trace.prints, vec!["door shut".to_string()]);
    }
}