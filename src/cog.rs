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

/// The engine's cog message names and their ids, from the retail binary's own table.
///
/// `sithCog_Startup` (`JK.EXE:004de070`) registers every one of these as a keyword with
/// `sithCog_AddIntSymbol`: `activate` 1, `activated` **1 as well** (an alias), `startup` 3, `timer` 4,
/// `blocked` 5, `entered` 6 … `taken` 26, `user0` 27 … `user7` 34, `newplayer` 35, `fire` 36,
/// `join` 37, `leave` 38, `splash` 39, `trigger` 40. So `SendMessage(GetMasterCOG(), user0)` is
/// `SendMessage(cog, 27)` on the stack, and a receiver matches its `message:` list by id rather than by
/// position. `pickedup` is a shipped script's own name and appears in no retail table, so it — and
/// anything else unlisted — stays on the positional fallback in [`Cog::label_for_message_id`].
pub fn message_id(name: &str) -> Option<i32> {
    MESSAGE_IDS
        .iter()
        .find(|(known, _)| known.eq_ignore_ascii_case(name))
        .map(|(_, id)| *id)
}

/// The same table as data, because `sithCog_Startup` registers **every** one of these as a keyword: a
/// script can write `SendMessage(cog, user0)` without declaring `message user0` at all, so the VM needs
/// them in its globals rather than bound per script.
pub const MESSAGE_IDS: &[(&str, i32)] = &[
    ("activate", 1),
    ("activated", 1),
    ("removed", 2),
    ("startup", 3),
    ("timer", 4),
    ("blocked", 5),
    ("entered", 6),
    ("exited", 7),
    ("crossed", 8),
    ("sighted", 9),
    ("damaged", 10),
    ("arrived", 11),
    ("killed", 12),
    ("pulse", 13),
    ("touched", 14),
    ("created", 15),
    ("loading", 16),
    ("selected", 17),
    ("deselected", 18),
    ("autoselect", 19),
    ("changed", 20),
    ("deactivated", 21),
    ("shutdown", 22),
    ("respawn", 23),
    ("aievent", 24),
    ("skill", 25),
    ("taken", 26),
    ("user0", 27),
    ("user1", 28),
    ("user2", 29),
    ("user3", 30),
    ("user4", 31),
    ("user5", 32),
    ("user6", 33),
    ("user7", 34),
    ("newplayer", 35),
    ("fire", 36),
    ("join", 37),
    ("leave", 38),
    ("splash", 39),
    ("trigger", 40),
];

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
    /// `mask=`: the thing types whose messages reach this link, one bit per type
    /// (`sithCogParse_ParseSymbolRef`, `0x4fdae0`: default `0x401`, the world and the player).
    pub mask: u32,
    /// `nolink`: link id -1, which `sithCog_LinkCog` (`0x4df610`) never records.
    pub nolink: bool,
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
    /// The answers a class script's `skill:` handler gives: `bin -> ReturnEx(value)`.
    ///
    /// `sithCogFunctionThing_SkillTarget` (`0x5047e0`) pops two things and two flexes, checks the
    /// victim's class cog, and calls `sithCog_SendMessageEx(cog, 25, 3, ...)`. Message id 25 is
    /// `skill`, so the damage a force power does is the **victim's class script** answering:
    /// `actor_pm.cog` returns 10 for grip (31) and 5 for deadly sight (34) — "base damage that is
    /// taken by this actor" — which `force_grip.cog` multiplies by rank/2, and `kyle.cog` returns
    /// 1 for a successful grip on a player (it applies `2 * rank` itself).
    pub fn skill_answers(&self) -> Vec<(i32, f32)> {
        fn returnex(stmt: &Stmt, out: &mut Option<f32>) {
            match stmt {
                Stmt::Call(name, args) if name.eq_ignore_ascii_case("returnex") => {
                    if let Some(Expr::Num(v)) = args.first() {
                        *out = Some(*v as f32);
                    }
                }
                Stmt::Block(list) => {
                    for s in list {
                        returnex(s, out);
                    }
                }
                Stmt::If(_, then, other) => {
                    returnex(then, out);
                    if out.is_none()
                        && let Some(other) = other
                    {
                        returnex(other, out);
                    }
                }
                _ => {}
            }
        }
        let mut out = Vec::new();
        let Some(body) = self.label("skill") else { return out };
        for stmt in body {
            let Stmt::If(Expr::Bin(BinOp::Eq, lhs, rhs), then, _) = stmt else { continue };
            let (Expr::Var(var), Expr::Num(bin)) = (&**lhs, &**rhs) else { continue };
            if !var.eq_ignore_ascii_case("bin") {
                continue;
            }
            let mut value = None;
            returnex(then, &mut value);
            if let Some(value) = value {
                out.push((*bin as i32, value));
            }
        }
        out
    }

    pub fn label(&self, name: &str) -> Option<&Vec<Stmt>> {
        let want = name.to_ascii_lowercase();
        self.labels.iter().find(|(n, _)| *n == want).map(|(_, s)| s)
    }

    pub fn messages(&self) -> impl Iterator<Item = &Symbol> {
        self.symbols.iter().filter(|s| s.kind == SymKind::Message)
    }

    /// The label a numeric message id names, for the pumps: the receiver's `message:` list is matched
    /// by engine id, and a name the enum does not cover still falls back to the old positional reading
    /// (`message: list index == id`), which is how this port delivered messages before the ids existed.
    pub fn label_for_message_id(&self, id: usize) -> Option<String> {
        self.messages()
            .find(|m| message_id(&m.name) == Some(id as i32))
            .or_else(|| self.messages().nth(id))
            .map(|m| m.name.clone())
    }

    /// The label a message name resolves to: `#N` through [`Self::label_for_message_id`], or the name
    /// itself. `None` when the script has no such label, so a pump can skip the script outright.
    pub fn label_for_name(&self, name: &str) -> Option<String> {
        let label = match name.strip_prefix('#').and_then(|n| n.parse::<usize>().ok()) {
            Some(id) => self.label_for_message_id(id)?,
            None => name.to_string(),
        };
        // A **message name** the script does not also declare as a label still resolves by id:
        // `MESSAGE_IDS` gives `activate` and `activated` the same id (1), and `00_doorswitch.cog`
        // declares `message activated` with an `activated:` label only — so the use action's
        // `activate` never reached the door until this fallback. A script that *does* declare
        // `activate` wins above, unchanged.
        if self.label(&label).is_some() {
            return Some(label);
        }
        let id = usize::try_from(message_id(&label)?).ok()?;
        self.label_for_message_id(id)
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
        // A `message` symbol in an expression is the engine's message **id**, not a variable:
        // `sithCogParse` gives `message user0` the value `SITH_MESSAGE_USER0` (27), and that number is
        // what `SendMessage(GetMasterCOG(), user0)` pushes on the stack.
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

    /// `base[i]` is the symbol declared `i` slots after `base`: `sithCogExec_Execute` (`0x4e1f60`)
    /// opcode 4 pushes the base's symbol ID plus the index, and `sithCogParse_AddSymbol` (`0x4fd260`)
    /// numbers a cog's symbols in declaration order (`sithCogParse_GetSymbolByID`, `0x4fd3d0`, reads
    /// that slot). `02_endlevel.cog`'s `numYeesEasy[GetDifficulty()]` reads `numYeesMed`/`numYeesHard`
    /// this way. `00_secretareas.cog`'s `secret0[i]` lands on `secret<i>` because those are declared in
    /// order; their `linkid` is the message bit `GetSenderId()` returns, not an element number.
    fn indexed_name(&self, name: &str, index: i64) -> Option<String> {
        let found = self
            .symbols
            .iter()
            .position(|s| s.name.eq_ignore_ascii_case(name))
            .and_then(|slot| usize::try_from(slot as i64 + index).ok())
            .and_then(|slot| self.symbols.get(slot))
            .map(|s| s.name.clone());
        if std::env::var_os("BK_INDEX_TRACE").is_some() {
            eprintln!("index: {name}[{index}] -> {found:?}");
        }
        found
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
    // `stdConffile_ReadArgsFromStr` (`0x431550`) splits on `", \t\n\r"`, so `mask=0x8,linkid=0` is
    // two arguments.
    let ext: Vec<&str> = parts.flat_map(|p| p.split(',')).filter(|p| !p.is_empty()).collect();
    let local = ext.iter().any(|p| p.eq_ignore_ascii_case("local"));
    let flag = |key: &str| ext.iter().find_map(|p| p.strip_prefix(key));
    let linkid = flag("linkid=").and_then(|v| v.parse().ok());
    let nolink = ext.iter().any(|p| p.eq_ignore_ascii_case("nolink"));
    let mask = flag("mask=").and_then(scanf_hex).unwrap_or(0x401);
    cog.symbols.push(Symbol { kind, name: name.to_ascii_lowercase(), init, local, linkid, mask, nolink });
    Ok(())
}

/// `sscanf("%x")`, which `ParseSymbolRef` reads `mask=` with: an optional sign and `0x`, then hex
/// digits; a negative value wraps (`mask=-1` is every type).
fn scanf_hex(text: &str) -> Option<u32> {
    let (negative, digits) = match text.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, text.strip_prefix('+').unwrap_or(text)),
    };
    let digits = digits.strip_prefix("0x").or_else(|| digits.strip_prefix("0X")).unwrap_or(digits);
    let value = u32::from_str_radix(digits, 16).ok()?;
    Some(if negative { value.wrapping_neg() } else { value })
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

    /// `sithCogExec_PopInt` (`0x4e25c0`), which every condition, `!` and the integer operators
    /// (`sithCogExec_IntererOps`, `0x4e3630`) read their operands through: a flex is floored, an int
    /// passes, any other value is `0xffffffff` (-1). Decomp's `ROUND` hides the mode: each site in
    /// `JK.EXE` is `fldcw [0x860802]; fld; fistp; fldcw [0x860800]`, and `stdStartup` (`0x426bb0`)
    /// builds `0x860802` with high byte 7 (RC=01, round down). `Nil` (a symbol or verb result this
    /// VM does not have) stays 0. Kept in `i64` like the rest of this VM's integers, so a bit-31 flag
    /// keeps its bits instead of saturating.
    fn int(&self) -> i64 {
        match self {
            Value::Num(n) => n.floor() as i64,
            Value::Nil => 0,
            Value::Str(_) | Value::Vec(_) => -1,
        }
    }

    /// `sithCogExec_PopFlex` (`0x4e24f0`), the arithmetic and comparison operands
    /// (`sithCogExec_FloatOps`, `0x4e3870`): a 32-bit float; anything that is not a number reads
    /// `_DAT_00522fb4`, which is `0.0f` in `JK.EXE`'s `.rdata`.
    fn flex(&self) -> f32 {
        match self {
            Value::Num(n) => *n as f32,
            Value::Nil | Value::Str(_) | Value::Vec(_) => 0.0,
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
    /// `ReturnEx(value)`: the value a message handler answers with. `sithCogExec_ReturnEx` stores it
    /// and the caller reads it back after the label runs — that is how a `damaged` handler says how
    /// much damage it will actually take (`force_absorb.cog` returns the reduced amount).
    pub ret: Option<Value>,
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
    /// This cog's own index in the world's cog list — `pCog->idx`, which `GetSelfCog` returns and
    /// scripts compare against `GetMasterCog()`. The VM holds it because the world does not know
    /// which of its scripts is calling.
    pub cog_index: i32,
    /// The script this VM runs (the file name), set by whoever creates it: `cog_index` alone cannot
    /// name the script behind a verb trace, and a started cog's index is not its place in the level's
    /// table.
    pub script_name: String,
    /// This cog's pulse, as the engine keeps it (`SITH_COG_PULSE_SET`): `SetPulse(seconds)` sets
    /// the interval and the next due time, and every cog tick that is due sends the `pulse` message.
    pub pulse: Option<(f32, f32)>,
    /// `sithCogFunction_SetTimer` (`FUN_00506340`): the cog's **single** timer slot at `+0x20`, due
    /// at `now + seconds`, cleared when it fires by `sithCog_ProcessCog` (`0x4e0300`) and sent as
    /// message 4 with no sender id or parameters. A later `SetTimer` overwrites it. Scripts that need
    /// an id or parameters use `SetTimerEx`, which queues an event instead.
    pub timer: Option<f32>,
    /// This cog's one-shot timers (`SITH_COG_TIMER_SET`), each firing the `timer` message once when
    /// due. `SetTimerEx(seconds, id, val, val2)` also stores the id the handler reads back through
    /// `GetSenderId()` and the two parameter words — the shipped actor scripts branch on exactly
    /// that id (`timer:` in `actor_<class>.cog`: id 0 resets a count, id 1 credits the level's
    /// population bin). A **list**, not one slot: `kyle.cog`'s `init_kyle` arms the 1.5 s respawn
    /// invulnerability clear (id 1) and its `startup` then arms the bubbles timer (id 2) in the same
    /// frame, and with one slot the overwrite left the player `SetActorFlags(player, 0x8)` frozen for
    /// the whole level (the door capture swallowed every bolt).
    pub timers: Vec<(f32, i64, [f64; 2])>,
    /// Seconds this cog has been running, which is what pulse and timer deadlines are measured against.
    pub now: f32,
    /// When a `sleep` in the interrupted label may continue.
    pub sleep_until: f32,
    /// `(label, statement path, due)`: what a `sleep` interrupted. The engine keeps the same thing as
    /// `script_running == 2` plus a saved instruction pointer (§16.10), and the saved pointer is the
    /// statement that slept, not the statement containing it — so the path records the descent through
    /// `if`/`else` and plain blocks (`[i]` = statement `i` of the label, `[i, j]` = statement `j` inside
    /// statement `i`'s block) and a resume re-enters at the exact statement.
    pub resume: Option<(String, Vec<(u32, u8)>, f32)>,
    /// The descent from the label body to the statement being run: `(statement index, branch)`, where
    /// the branch is 0 for a block body and 1/2 for an `if` then/else. `sleep` records it.
    path: Vec<(u32, u8)>,
    /// Set by `sleep`; the call sites turn it into the `SLEEPING` unwind.
    suspended: bool,
    /// Set when a resume re-enters the label: the next `sleep` was already waited out.
    sleep_skip: bool,
}

/// The sentinel a `sleep` unwinds with, so `run_label` can tell "waiting" from a script error.
pub const SLEEPING: &str = "::sleeping";

/// The statements a resume descends into: the `then` arm (1), the `else` arm (2), or a plain block (0).
fn descent(stmt: &Stmt, branch: u8) -> Option<&[Stmt]> {
    match (stmt, branch) {
        (Stmt::Block(body), 0) => Some(body),
        (Stmt::If(_, then, _), 1) => Some(block_of(then)),
        (Stmt::If(_, _, Some(els)), 2) => Some(block_of(els)),
        _ => None,
    }
}

/// The statements of an `if` arm. A branch written without braces (`if (x) Sleep(1);`, or the `if` of
/// an `else if`) is a one-statement block, so it gets its own path entry: run in place, a nested `if`
/// overwrote its parent's branch and a resume took the wrong arm (`04_EndLevel.cog`'s `entered:`).
fn block_of(stmt: &Stmt) -> &[Stmt] {
    match stmt {
        Stmt::Block(body) => body,
        other => std::slice::from_ref(other),
    }
}



impl Vm {
    pub fn new() -> Self {
        // `sithCog_Startup` registers every message name as a **keyword** with its id, so a script can
        // say `SendMessage(cog, user0)` without declaring anything: `user0` is 27 before any `.cog` text
        // is read. These are globals, not per-script symbols.
        let vars = MESSAGE_IDS
            .iter()
            .map(|(name, id)| ((*name).to_string(), Value::Num(*id as f64)))
            .collect();
        Self {
            vars,
            trace: Trace::default(),
            ret: None,
            depth: 0,
            budget: 2_000_000,
            world: None,
            sender: 0,
            sender_ref: 0,
            sender_type: 0,
            source: 0,
            cog_index: -1,
            script_name: String::new(),
            pulse: None,
            timer: None,
            timers: Vec::new(),
            now: 0.0,
            sleep_until: 0.0,
            resume: None,
            path: Vec::new(),
            suspended: false,
            sleep_skip: false,

        }
    }

    /// Give the VM a world to act on. Without one, world verbs answer 0 (which is
    /// how the parser and language are tested in isolation).
    pub fn with_world(world: crate::runtime::World) -> Self {
        Self { world: Some(world), ..Self::new() }
    }

    /// A script variable, or else one of `sithCog_Startup`'s global message keywords (`user0` is 27):
    /// `03_goals.cog` sends `user0` without declaring it.
    pub fn var(&self, name: &str) -> Value {
        let name = name.to_ascii_lowercase();
        match self.vars.get(&name) {
            Some(value) => value.clone(),
            None => message_id(&name).map_or(Value::Nil, |id| Value::Num(f64::from(id))),
        }
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
    /// Take every timer that is due at `now`, earliest first, and leave the rest armed.
    ///
    /// A cog may hold more than one timer (`kyle.cog` arms its 1.5 s respawn-invulnerability clear and
    /// its bubbles timer in the same frame), so the slot is a list and a due batch is collected in
    /// deadline order -- a handler that re-arms sees the later timers still pending.
    pub fn take_due_timers(&mut self) -> Vec<(i64, [f64; 2])> {
        let now = self.now;
        let mut due: Vec<(f32, i64, [f64; 2])> = Vec::new();
        // `SetTimer`'s single slot fires with no sender id or parameters
        // (`sithCog_ProcessCog` sends `SendMessage(cog, 4, 0, 0, 0, 0, 0)`), so a handler that reads
        // `GetSenderId()` sees 0 — `actor_fn.cog`'s `timer:` takes its id-0 branch and credits nothing.
        if let Some(at) = self.timer
            && now >= at
        {
            due.push((at, 0, [0.0; 2]));
            self.timer = None;
        }
        let (fired, pending): (Vec<_>, Vec<_>) =
            self.timers.drain(..).partition(|(at, _, _)| now >= *at);
        self.timers = pending;
        due.extend(fired);
        due.sort_by(|a, b| a.0.total_cmp(&b.0));
        due.into_iter().map(|(_, id, params)| (id, params)).collect()
    }

    /// Write the message's sender onto this VM, as `sithCog_SendMessageEx` (`0x4dedc0`) does when it
    /// dispatches. The verbs read fixed slots (their bodies in `JK.EXE`, registered by
    /// `sithCogFunction_Startup`, `0x505400`): `GetSenderId` (`0x505cb0`) `+0x24`, the link id;
    /// `GetSenderRef` (`0x505cd0`) `+0x28` and `GetSenderType` (`0x505cf0`) `+0x2c`, the **source
    /// index and type**, the sector, surface or thing the link names; `GetSourceRef` (`0x505d10`)
    /// `+0x30`, the **thing that caused it** (-1 for the world). A message queued with a link id
    /// alone leaves the rest alone. Returns the fields as they were, for
    /// [`Vm::restore_message_sender`] after the handler.
    pub fn set_message_sender(
        &mut self,
        link: Option<i64>,
        identity: Option<&crate::runtime::SourceSender>,
    ) -> (i64, i64, i64, i64) {
        let own = (self.sender, self.sender_ref, self.sender_type, self.source);
        self.sender = link.map_or(0, |s| s);
        if let Some(identity) = identity {
            self.sender_ref = identity.source as i64;
            self.sender_type = identity.source_type as i64;
            self.source = identity.sender as i64;
        }
        own
    }

    /// Put back the sender fields a delivery replaced, so the next message does not inherit them.
    pub fn restore_message_sender(&mut self, own: (i64, i64, i64, i64)) {
        (self.sender, self.sender_ref, self.sender_type, self.source) = own;
    }

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
            // A `ReturnEx` belongs to the run that set it: the sender reads it right after this
            // label, so the next one starts clean.
            self.ret = None;
        }
        if self.depth > 24 {
            return Err(format!("call depth limit in `{label}`"));
        }
        // Resume where a `sleep` stopped this label, or start at the top.
        let want = label.to_ascii_lowercase();
        // Only a run of the **sleeping label** may consume the pending resume. Taking it for a
        // different label dropped it for good, so a `sleep` in `startup` never resumed once the
        // level's other messages were delivered - which is why `01_initialry.cog`'s
        // `Sleep(0.5)` never reached its `AISetMovePos` (round 307).
        let resume_path = match self.resume.as_ref() {
            Some((name, ..)) if *name == want => {
                let (name, path, due) = self.resume.take().expect("checked above");
                if self.now < due {
                    self.resume = Some((name, path, due));
                    return Err(SLEEPING.into());
                }
                self.sleep_skip = true;
                Some(path)
            }
            _ => None,
        };
        self.depth += 1;
        let started = std::time::Instant::now();
        let mut result = Ok(Value::Nil);
        // A `call` runs this label inside the caller's statement: the path is per label, or the
        // caller's descent would prefix the callee's resume point and send it to the wrong statement.
        let outer = std::mem::take(&mut self.path);
        let run = match resume_path {
            Some(path) => self.run_path(cog, body, &path),
            None => self.run_from(cog, body, 0),
        };
        match run {
            Ok(Flow::Return(v)) => result = Ok(v),
            Ok(_) => {}
            Err(e) if e == SLEEPING => {
                // Resume at the statement that slept; its `sleep` is transparent the first time back.
                // A `sleep` inside a loop records only the loop statement (`run_path` stops there),
                // because the loop's condition and step have to run before the body again.
                let path = std::mem::take(&mut self.path);
                self.resume = Some((want.clone(), path, self.sleep_until));
            }
            Err(e) => result = Err(e),
        }
        self.path = outer;
        self.depth -= 1;
        // `ReturnEx` is the handler's answer even though the script ends with a bare `Return`: the
        // engine's caller reads the cog's stored value, so a label that stored one returns it.
        if matches!(result, Ok(Value::Nil)) && !self.sleeping() {
            if let Some(value) = self.ret.take() {
                result = Ok(value);
            }
        }
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

    /// Run `body` from `start`, recording the descent so a `sleep` inside can resume in place.
    ///
    /// The recording is what makes `sleep` faithful: the engine saves the statement pointer, so a
    /// wait inside an `if` continues with the statement after it, while recording only the enclosing
    /// top-level statement used to re-run everything before the wait. Statements that are not blocks
    /// (`if (x) Sleep(1);`) still resume at the enclosing statement — its condition has to run.
    fn run_from(&mut self, cog: &Cog, body: &[Stmt], start: usize) -> Result<Flow, String> {
        for i in start..body.len() {
            self.path.push((i as u32, 0));
            let run = self.exec(cog, &body[i]);
            // A sleeping statement keeps its place; every other outcome leaves the descent.
            if !matches!(&run, Err(e) if e == SLEEPING) {
                self.path.pop();
            }
            match run? {
                Flow::Next => {}
                done => return Ok(done),
            }
        }
        Ok(Flow::Next)
    }

    /// Resume a label at the statement `path` names, descending through `if`/`else` bodies and plain
    /// blocks. A path that points into a loop restarts at the loop statement: the loop's own condition
    /// and step are part of the statement, so the walker cannot skip them.
    fn run_path(&mut self, cog: &Cog, body: &[Stmt], path: &[(u32, u8)]) -> Result<Flow, String> {
        let Some(&(index, branch)) = path.first() else { return self.run_from(cog, body, 0) };
        let index = index as usize;
        if index >= body.len() {
            return self.run_from(cog, body, 0);
        }
        if path.len() == 1 {
            return self.run_from(cog, body, index);
        }
        let Some(inner) = descent(&body[index], branch) else {
            return self.run_from(cog, body, index);
        };
        self.path.push((index as u32, branch));
        let run = self.run_path(cog, inner, &path[1..]);
        if !matches!(&run, Err(e) if e == SLEEPING) {
            self.path.pop();
        }
        // The engine's saved statement pointer walks out of a finished arm into the rest of this
        // list: a `Sleep` as an arm's last statement used to end the label at the `if` itself, and
        // every statement after the enclosing `if` was skipped (#84).
        if matches!(run, Ok(Flow::Next)) {
            return self.run_from(cog, body, index + 1);
        }
        run
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
            Stmt::Block(body) => self.run_from(cog, body, 0),
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
                if c.int() != 0 {
                    if let Some(last) = self.path.last_mut() {
                        last.1 = 1;
                    }
                    self.run_from(cog, block_of(then), 0)
                } else if let Some(e) = els {
                    if let Some(last) = self.path.last_mut() {
                        last.1 = 2;
                    }
                    self.run_from(cog, block_of(e), 0)
                } else {
                    Ok(Flow::Next)
                }
            }
            Stmt::While(cond, body) => {
                for _ in 0..LOOP_BUDGET {
                    if self.eval(cog, cond)?.int() == 0 {
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
                    if self.eval(cog, cond)?.int() == 0 {
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
                    if self.eval(cog, cond)?.int() == 0 {
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
                // `0x4e1f60` opcode 0x1f: a `call` pushes a frame only while the cog's call stack is
                // below 4 (`param_1[0x24] < 4`), and is skipped otherwise. This VM's `depth` counts
                // the running label too, so the engine's stack depth is `depth - 1`.
                if self.depth <= 4 {
                    self.run_label(cog, label)?;
                }
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
                let v = self.eval(cog, e)?;
                match op {
                    // `0x4e1f60` opcode 0xe: `-PopFlex`, a flex.
                    UnOp::Neg => Value::Num(f64::from(-v.flex())),
                    // Opcode 0xd: `PopInt == 0`.
                    UnOp::Not => Value::Num(f64::from(u8::from(v.int() == 0))),
                    // No engine opcode; kept for the parser's sake.
                    UnOp::BitNot => Value::Num(!v.int() as f64),
                }
            }
            Expr::Assign(name, value) => {
                let v = self.eval(cog, value)?;
                self.set_var(name, v.clone());
                v
            }
            Expr::Bin(op, a, b) => {
                // Both operands are evaluated: the engine pushes both and the operator pops them.
                let x = self.eval(cog, a)?;
                let y = self.eval(cog, b)?;
                let int = |v: bool| Value::Num(f64::from(u8::from(v)));
                // `sithCogExec_IntererOps` (`0x4e3630`, opcodes 0x14-0x19): rounded integer operands.
                let (i, j) = (x.int(), y.int());
                // `sithCogExec_FloatOps` (`0x4e3870`, opcodes 8-0x13): 32-bit float operands.
                let (a, b) = (x.flex(), y.flex());
                match op {
                    BinOp::And => int(i != 0 && j != 0),
                    BinOp::Or => int(i != 0 || j != 0),
                    BinOp::Ne => int(i != j),
                    BinOp::BitAnd => Value::Num((i & j) as f64),
                    BinOp::BitOr => Value::Num((i | j) as f64),
                    BinOp::BitXor => Value::Num((i ^ j) as f64),
                    BinOp::Add => Value::Num(f64::from(a + b)),
                    BinOp::Sub => Value::Num(f64::from(a - b)),
                    BinOp::Mul => Value::Num(f64::from(a * b)),
                    BinOp::Div => Value::Num(f64::from(if b == 0.0 { 0.0 } else { a / b })),
                    // Opcode 0xc calls the CRT's `fmod` (`FUN_0051504a`); a zero divisor stays 0 here.
                    BinOp::Mod => Value::Num(f64::from(if b == 0.0 { 0.0 } else { a % b })),
                    BinOp::Eq => int(a == b),
                    BinOp::Lt => int(a < b),
                    BinOp::Le => int(a <= b),
                    BinOp::Gt => int(a > b),
                    BinOp::Ge => int(a >= b),
                    // No engine opcode; kept for the parser's sake.
                    BinOp::Shl => Value::Num(i.wrapping_shl(j as u32) as f64),
                    BinOp::Shr => Value::Num(i.wrapping_shr(j as u32) as f64),
                }
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
        // arms an interval that sends this cog's `pulse` message, `SetTimer(seconds)` arms the
        // single-slot `timer`, and `SetTimerEx` queues a timer event with an id and two parameters.
        // `sithCogFunction.c` keeps all three on the cog struct.
        match name.to_ascii_lowercase().as_str() {
            "setpulse" => {
                let value = args.first().map(|v| v.num()).unwrap_or(0.0) as f32;
                self.pulse = (value > 0.0).then(|| (value, self.now + value));
                return Value::Nil;
            }

            // `ReturnEx(value)`: the value the handler answers with. The engine stores it on the cog
            // (`sithCogExec_ReturnEx`) and the sender reads it once the label has run; the scripts
            // still write a bare `Return` after it, so it does **not** end the label.
            "returnex" => {
                self.ret = Some(args.first().cloned().unwrap_or(Value::Nil));
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
            // `sithCogFunction_SetTimer` (`FUN_00506340`) pops one value and arms the cog's single
            // timer slot; it carries no id or parameters (`sithCog_ProcessCog` `0x4e0300`).
            "settimer" => {
                let value = args.first().map(|v| v.num()).unwrap_or(0.0) as f32;
                self.timer = (value > 0.0).then(|| self.now + value);
                return Value::Nil;
            }
            "settimerex" => {
                let value = args.first().map(|v| v.num()).unwrap_or(0.0) as f32;
                let id = args.get(1).map(|v| v.num() as i64).unwrap_or(0);
                let params = [
                    args.get(2).map(|v| v.num()).unwrap_or(0.0),
                    args.get(3).map(|v| v.num()).unwrap_or(0.0),
                ];
                if value > 0.0 {
                    self.timers.push((self.now + value, id, params));
                }
                return Value::Nil;
            }
            _ => {}
        }
        if let Some(world) = &self.world {
            if crate::runtime::Runtime::handles(name) {
                let mut world = world.borrow_mut();
                // Engine verbs can see which cog is calling them (`pCog`): `SendMessage` records that
                // index as the sender, and the receiver reads it back through `GetSenderID`.
                world.caller_cog = self.cog_index;
                world.caller_script.clone_from(&self.script_name);
                return world.call(name, &args);
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
            // `sithCogFunction_GetSelfCog` is `pCog->idx`, and the master scripts compare it with
            // `GetMasterCog()`, so it has to be this VM's own index rather than the world's answer.
            "getselfcog" => Value::Num(self.cog_index as f64),
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
            // `PrintInt(value)`: the integer form of `Print`. `00_simple_elev` is the only script that
            // calls it, and a missing verb was an unanswered call rather than a printed number.
            "printint" => {
                self.trace.prints.push(format!("{}", args.first().map(|v| v.num() as i64).unwrap_or(0)));
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

thread_local! {
    static SEED_STATE: std::cell::Cell<u32> = const { std::cell::Cell::new(0x1234_5678) };
}

/// The engine's `rand()`, which is one 32-bit LCG state at **`0x0054F750`**
/// (`sithCog_g_pRandomSeed`): every AI roll and every script `Rand()` draws from it. The retail
/// capture records that word at each sample (`route_door.tsv`'s `rng` column), so the port can start
/// a run from the engine's own state instead of a made-up seed.
pub fn pseudo_random() -> f64 {
    SEED_STATE.with(|s| {
        let x = s.get().wrapping_mul(214_013).wrapping_add(2_531_011);
        s.set(x);
        // `rand()` returns `(state >> 16) & 0x7fff`; the engine's callers divide by `_DAT_0052307c`
        // (32768). `frandom()` is that divided by the same constant.
        f64::from((x >> 16) & 0x7fff) / 32_768.0
    })
}

/// Set the engine's `rand()` state (the captured word at `0x0054F750`). `jitter_aim` and the AI
/// cadence read the same stream the retail engine advanced, so a seeded run continues the captured
/// sequence rather than inventing one.
pub fn set_random_seed(state: u32) {
    SEED_STATE.with(|s| s.set(state));
}

/// The engine's current `rand()` state: what the capture's `rng` column reads.
pub fn random_state() -> u32 {
    SEED_STATE.with(|s| s.get())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The engine's `rand()` is the LCG at `0x0054F750`, and the committed capture records its word
    /// at the door origin (811474043) and at every later sample. Stepping that state six times has
    /// to reach 1423890337, the next word the capture wrote, or the port is not advancing the
    /// engine's stream at all.
    #[test]
    fn the_a_i_random_stream_is_the_captures_lcg() {
        set_random_seed(811_474_043);
        let mut states = Vec::new();
        for _ in 0..6 {
            pseudo_random();
            states.push(random_state());
        }
        assert_eq!(
            states,
            vec![
                3_289_249_106,
                1_226_606_285,
                1_292_271_196,
                1_303_876_527,
                2_504_482_742,
                1_423_890_337,
            ],
            "the AI random stream is not the engine's LCG at 0x0054F750"
        );
        // The draw is `(state >> 16) & 0x7fff` over 32768, the engine's `rand()`/32768.
        set_random_seed(811_474_043);
        assert_eq!(pseudo_random(), f64::from((3_289_249_106u32 >> 16) & 0x7fff) / 32_768.0);
        // Leave the shared stream where every other test expects it.
        set_random_seed(0x1234_5678);
    }

    /// `kyle.cog` arms two timers in one frame — the 1.5 s respawn-invulnerability clear (id 1) and
    /// the bubbles check (id 2) — and with a single slot the startup's write erased the clear, which
    /// left the player `SetActorFlags(player, 0x8)` frozen for the whole level.
    #[test]
    fn two_timers_armed_in_one_frame_both_fire_in_due_order() {
        let mut vm = Vm::new();
        vm.timers.push((2.0, 2, [0.0, 0.0]));
        vm.timers.push((1.5, 1, [7.0, 0.0]));
        assert!(vm.take_due_timers().is_empty(), "a timer fired before it was due");
        vm.now = 1.6;
        assert_eq!(vm.take_due_timers(), vec![(1, [7.0, 0.0])]);
        assert_eq!(vm.timers.len(), 1, "the later timer was dropped");
        vm.now = 2.0;
        assert_eq!(vm.take_due_timers(), vec![(2, [0.0, 0.0])]);
        assert!(vm.timers.is_empty());
    }

    /// `SetTimer` (`FUN_00506340`) arms the cog's one slot, not a list entry: a later call overwrites
    /// it, and `sithCog_ProcessCog` (`0x4e0300`) sends `timer` with no sender id. Level 1's two
    /// `nwoman1` actors arm `SetTimer(1.0, 1, 0, 0)` from `created:`; the engine's handler reads id 0
    /// and credits nothing, which is why retail's population bin is 10.5 and not 12.5.
    #[test]
    fn set_timer_is_one_slot_and_fires_without_a_sender_id() {
        let mut vm = Vm::new();
        vm.call_verb("SetTimer", vec![Value::Num(2.0)]);
        vm.call_verb("SetTimer", vec![Value::Num(1.0)]);
        assert!(vm.take_due_timers().is_empty(), "a timer fired before it was due");
        vm.now = 1.5;
        assert_eq!(vm.take_due_timers(), vec![(0, [0.0, 0.0])]);
        assert!(vm.timer.is_none(), "the fired timer was not cleared");
        // A cancel and an overwrite both leave at most one slot.
        vm.call_verb("SetTimer", vec![Value::Num(2.0)]);
        vm.call_verb("SetTimer", vec![Value::Num(0.0)]);
        assert!(vm.timer.is_none(), "a negative time did not cancel the timer");
        vm.now = 10.0;
        assert!(vm.take_due_timers().is_empty());
    }

    /// `sithCog_SendMessageEx` (`0x4dedc0`) fills the slots the sender verbs read (bodies in `JK.EXE`):
    /// `GetSenderId` the link id, `GetSenderRef`/`GetSenderType` the object the link names (`+0x28`,
    /// `+0x2c`), `GetSourceRef` the thing that caused the message (`+0x30`), -1 for the world.
    /// `03_goals.cog` tests `GetSenderRef() == goal2sector0` in `entered:`, so the sector must be there.
    #[test]
    fn sender_verbs_name_the_linked_object_and_source_the_cause() {
        use crate::runtime::{SECTOR_SOURCE, SourceSender, THING_SOURCE};
        let script = parse(
            "symbols\nmessage activate\nend\ncode\nactivate:\n\tPrint(GetSenderId());\n\tPrint(GetSenderRef());\n\
             \tPrint(GetSenderType());\n\tPrint(GetSourceRef());\n\treturn;\nend\n",
        )
        .expect("parses");
        let mut vm = Vm::new();
        // Body 7 used door thing 4, linked as id 2.
        let own = vm.set_message_sender(Some(2), Some(&SourceSender::new(THING_SOURCE, 4, Some(7))));
        vm.run_label(&script, "activate").expect("the handler runs");
        assert_eq!(vm.trace.prints, vec!["2", "4", "3", "7"]);
        vm.restore_message_sender(own);
        assert_eq!(
            (vm.sender, vm.sender_ref, vm.sender_type, vm.source),
            (0, 0, 0, 0),
            "the sender fields were not restored after the handler"
        );
        // The player (thing 1) entered sector 232, linked as id 0; the world's message has no cause.
        let own = vm.set_message_sender(Some(0), Some(&SourceSender::new(SECTOR_SOURCE, 232, Some(1))));
        vm.run_label(&script, "activate").expect("the handler runs");
        assert_eq!(vm.trace.prints[4..], ["0", "232", "5", "1"]);
        vm.restore_message_sender(own);
        let own = vm.set_message_sender(Some(2), Some(&SourceSender::new(THING_SOURCE, 4, None)));
        vm.run_label(&script, "activate").expect("the handler runs");
        assert_eq!(vm.trace.prints[8..], ["2", "4", "3", "-1"]);
        vm.restore_message_sender(own);
        // A port push with a link id alone leaves the rest as it was.
        vm.sender_ref = 9;
        let own = vm.set_message_sender(Some(5), None);
        assert_eq!((vm.sender, vm.sender_ref), (5, 9));
        vm.restore_message_sender(own);
        assert_eq!((vm.sender, vm.sender_ref, vm.sender_type), (0, 9, 0));
    }

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

    /// The engine saves the *statement* pointer, so a `sleep` inside a branch continues with the
    /// statement after the wait. Recording only the enclosing top-level statement re-ran the branch's
    /// prefix, which double-counted every assignment before the wait.
    #[test]
    fn a_sleep_inside_a_branch_resumes_after_the_wait() {
        let cog = parse(
            "symbols\nfloat n=0\nfloat flag=0\nend\ncode\nstartup:\n\tif (1)\n\t{\n\t\tn = n + 1;\n\t\tSleep(0.5);\n\t\tflag = 1;\n\t}\n\treturn;\nend\n",
        )
        .expect("parses");
        let mut vm = Vm::new();
        vm.run_label(&cog, "startup").expect("reaches the sleep");
        assert!(vm.sleeping(), "`sleep` did not suspend the label");
        assert_eq!(vm.var("n"), Value::Num(1.0));
        vm.now += 1.0;
        vm.run_label(&cog, "startup").expect("resumes");
        assert_eq!(vm.var("n"), Value::Num(1.0), "the branch prefix ran twice");
        assert_eq!(vm.var("flag"), Value::Num(1.0), "the wait resumed at the wrong statement");
    }

    /// `04_EndLevel.cog`'s `entered:` sleeps in the third arm of an `else if` chain. The resume must
    /// come back to that arm: it ran the first one (`endthelevel = 0`) and level 4 never ended (#49).
    #[test]
    fn a_sleep_inside_an_else_if_arm_resumes_in_that_arm() {
        let cog = parse(
            "symbols\nmessage startup\nend\ncode\nstartup:\n\tid = 2; x = 1; y = 0;\n\
             \tif (id == 1) { x = 0; } else if (id == 3) { x = 2; } else if (id == 2) { Sleep(0.5); y = 1; }\n\
             \treturn;\nend\n",
        )
        .expect("parses");
        let mut vm = Vm::new();
        vm.run_label(&cog, "startup").expect("reaches the sleep");
        assert!(vm.sleeping(), "`sleep` did not suspend the label");
        vm.now += 1.0;
        vm.run_label(&cog, "startup").expect("resumes");
        assert_eq!((vm.var("x"), vm.var("y")), (Value::Num(1.0), Value::Num(1.0)));
    }

    /// A `sleep` inside a `call`ed label resumes inside that label: the caller's own position must not
    /// leak into the callee's resume point.
    #[test]
    fn a_sleep_inside_a_called_label_resumes_after_its_wait() {
        let cog = parse(
            "symbols\nfloat x=0\nend\ncode\nstartup:\n\tx = 0;\n\tx = 0;\n\tcall wait_a_bit;\n\treturn;\nwait_a_bit:\n\tSleep(0.5);\n\tx = 1;\n\treturn;\nend\n",
        )
        .expect("parses");
        let mut vm = Vm::new();
        vm.run_label(&cog, "startup").expect("reaches the sleep");
        assert_eq!(vm.sleeping_label(), Some("wait_a_bit"));
        vm.now += 1.0;
        vm.run_label(&cog, "wait_a_bit").expect("resumes");
        assert_eq!(vm.var("x"), Value::Num(1.0), "the callee resumed past its own continuation");
    }

    /// A `Sleep` as an `if` arm's **last** statement must resume at the statement after the whole
    /// `if` in the parent list — not exit the parent list. #84: the resume skipped every statement
    /// that followed the enclosing `if`.
    #[test]
    fn a_sleep_at_an_if_arms_end_resumes_in_the_parent_list() {
        let cog = parse(
            "symbols\nfloat tail=0\nfloat tail2=0\nend\ncode\nstartup:\n\tif (1)\n\t{\n\t\tSleep(0.5);\n\t}\n\ttail = 1;\n\ttail2 = 2;\n\treturn;\nend\n",
        )
        .expect("parses");
        let mut vm = Vm::new();
        vm.run_label(&cog, "startup").expect("reaches the sleep");
        assert!(vm.sleeping(), "`sleep` did not suspend the label");
        vm.now += 1.0;
        vm.run_label(&cog, "startup").expect("resumes");
        assert_eq!(vm.var("tail"), Value::Num(1.0), "the statement after the `if` never ran");
        assert_eq!(vm.var("tail2"), Value::Num(2.0), "the parent list ended early");
    }

    /// A pending `sleep` belongs to its own label: running a **different** message label while the
    /// cog sleeps must not consume it. The load path delivers every message right after `startup`,
    /// so this is what dropped `01_initialry.cog`'s `Sleep(0.5)` continuation (round 307).
    #[test]
    fn another_label_running_while_a_sleep_is_pending_does_not_drop_the_resume() {
        let cog = parse(
            "symbols\nfloat n=0\nend\ncode\nstartup:\n\tn = 1;\n\tSleep(0.5);\n\tn = 2;\n\treturn;\npulse:\n\treturn;\nend\n",
        )
        .expect("parses");
        let mut vm = Vm::new();
        vm.run_label(&cog, "startup").expect("reaches the sleep");
        assert!(vm.sleeping(), "`sleep` did not suspend the label");
        vm.run_label(&cog, "pulse").expect("runs a different label");
        assert_eq!(
            vm.sleeping_label(),
            Some("startup"),
            "running another label consumed the pending sleep"
        );
        vm.now += 1.0;
        vm.run_label(&cog, "startup").expect("resumes");
        assert_eq!(vm.var("n"), Value::Num(2.0), "the sleep never finished");
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

    /// `GetSelfCog` is `pCog->idx`, and the shipped master/sidekick scripts compare it with
    /// `GetMasterCog()`, so the VM has to answer with its own index rather than a constant 0.
    #[test]
    fn a_cog_answers_with_its_own_index() {
        let cog = parse("symbols\nmessage startup\nend\ncode\nstartup:\n\tPrint(GetSelfCog());\n\treturn;\nend\n")
            .expect("parses");
        let mut vm = Vm::new();
        vm.cog_index = 7;
        vm.run_label(&cog, "startup").expect("runs");
        assert_eq!(vm.trace.prints, vec!["7".to_string()]);
        // A VM nobody indexed answers -1, which is the engine's "no cog" index.
        assert_eq!(Vm::new().call_verb("GetSelfCog", Vec::new()), Value::Num(-1.0));
    }

    /// The retail keyword table (`sithCog_Startup`) gives each message name a `SITH_MESSAGE_*` id, the
    /// parser stores that id in the symbol, and the pumps match the receiver by id — so
    /// `SendMessage(cog, user0)` reaches `message user0` wherever the receiver declares it.
    #[test]
    fn a_message_name_is_its_engine_id() {
        assert_eq!(message_id("user0"), Some(27));
        assert_eq!(message_id("USER7"), Some(34));
        // Retail registers `activated` against id 1 as well, alongside `activate`.
        assert_eq!(message_id("activated"), Some(1));
        assert_eq!(message_id("pickedup"), None);
        let cog = parse(
            "symbols\nmessage timer\nmessage user0\nend\ncode\ntimer:\n\treturn;\nuser0:\n\treturn;\nend\n",
        )
        .expect("parses");
        let mut vm = Vm::new();
        cog.bind(&mut vm, &[]);
        assert_eq!(vm.var("user0"), Value::Num(27.0));
        // By id: `user0` is declared second, so a positional reading would answer `timer`.
        assert_eq!(cog.label_for_message_id(27).as_deref(), Some("user0"));
        assert_eq!(cog.label_for_message_id(4).as_deref(), Some("timer"));
        // A name with no engine id still falls back to the positional reading.
        assert_eq!(cog.label_for_message_id(0).as_deref(), Some("timer"));
    }

    /// `sithCog_Startup` registers the message names globally, so a script that never declares
    /// `message user0` still reads it as 27 (`03_goals.cog`'s `SendMessageEx(GetMasterCOG(), user0, …)`).
    #[test]
    fn an_undeclared_message_name_reads_its_engine_id() {
        let cog = parse("symbols\nmessage entered\nend\ncode\nentered:\n\treturn;\nend\n").expect("parses");
        let mut vm = Vm::new();
        cog.bind(&mut vm, &[]);
        assert_eq!(vm.var("user0"), Value::Num(27.0));
        assert_eq!(vm.var("nothing_declared"), Value::Nil);
    }

    #[test]
    fn print_reaches_the_trace() {
        let cog = parse("symbols\nmessage startup\nend\ncode\nstartup:\n\tPrint(\"door shut\");\n\treturn;\nend\n")
            .expect("parses");
        let mut vm = Vm::new();
        vm.run_label(&cog, "startup").expect("runs");
        assert_eq!(vm.trace.prints, vec!["door shut".to_string()]);
    }

    /// `sithCogParse_ParseSymbolRef` (`0x4fdae0`): mask `0x401` unless `mask=` (read with `%x`, so
    /// `-1` is every type) says otherwise; `nolink` is link id -1.
    #[test]
    fn a_symbol_reads_its_mask_and_nolink() {
        let cog = parse(
            "symbols\nsector a\nthing b mask=-1\nsurface c mask=0x4\nsector d nolink\n\
             surface e mask=0x8,linkid=3\nend\ncode\nstartup:\n\treturn;\nend\n",
        )
        .expect("parses");
        let got: Vec<(u32, bool, Option<u32>)> =
            cog.symbols.iter().map(|s| (s.mask, s.nolink, s.linkid)).collect();
        assert_eq!(
            got,
            vec![(0x401, false, None), (u32::MAX, false, None), (0x4, false, None), (0x401, true, None), (0x8, false, Some(3))]
        );
    }

    /// `array[index]` is the symbol declared `index` slots after the base (`0x4e1f60` opcode 4). The
    /// shipped scripts declare their array elements with **bit** link ids (`secret1 linkid=1`,
    /// `secret2 linkid=2`, `secret3 linkid=4`, …) because the link id is what `entered:` tests
    /// against `GetSenderId()`, so resolving by link id put `secret0[4]` on `secret3` and made level 1
    /// count 8 secret areas instead of 6.
    #[test]
    fn an_array_index_is_the_declaration_slot_not_its_link_id() {
        let cog = parse(
            "symbols\nsector secret0 nolink local\nsector secret1 linkid=1\nsector secret2 linkid=2\n\
             sector secret3 linkid=4\nsector secret4 linkid=8\nend\ncode\nstartup:\n\treturn;\nend\n",
        )
        .expect("parses");
        assert_eq!(cog.indexed_name("secret0", 3).as_deref(), Some("secret3"));
        assert_eq!(cog.indexed_name("secret0", 4).as_deref(), Some("secret4"));
        assert_eq!(cog.indexed_name("secret0", 5), None, "no element 5 exists");
    }

    /// `02_endlevel.cog`'s difficulty table: the elements are named, not numbered, so only the slot
    /// rule reads them. The name rule left `countYees` at 0 and level 2 could never end (issue #35).
    #[test]
    fn an_array_of_named_symbols_reads_by_slot() {
        let cog = parse(
            "symbols\nmessage startup\nint countYees=0 local\nint numYeesEasy=3 local\n\
             int numYeesMed=4 local\nint numYeesHard=6 local\nend\ncode\nstartup:\n\
             \tcountYees = numYeesEasy[1];\n\treturn;\nend\n",
        )
        .expect("parses");
        let mut vm = Vm::new();
        cog.bind(&mut vm, &[]);
        vm.run_label(&cog, "startup").expect("runs");
        assert_eq!(vm.var("countyees").num(), 4.0);
    }

    /// Each line's expected value is the engine's (`0x4e1f60`/`0x4e3630`/`0x4e3870`): conditions, `!`
    /// and `!= && || & | ^` read floored integers (`PopInt`), and arithmetic and the other
    /// comparisons run in 32-bit floats. Truncation or round-to-nearest give the opposite on every line.
    #[test]
    fn operators_read_operands_the_way_the_engine_pops_them() {
        let cog = parse(
            "symbols\nmessage startup\nflex r0 local\nflex r1 local\nflex r2 local\nflex r3 local\n\
             flex r4 local\nflex r5 local\nflex r6 local\nend\ncode\nstartup:\n\
             \tif (0.9) r0 = 1; else r0 = 2;\n\tr1 = !0.9;\n\tr2 = 0.9 != 0;\n\tr3 = -0.4 & 1;\n\
             \tr4 = 1.6 != 1;\n\tr5 = 0.1 * 3 == 0.3;\n\tr6 = 0.9 && 1;\n\treturn;\nend\n",
        )
        .expect("parses");
        let mut vm = Vm::new();
        cog.bind(&mut vm, &[]);
        vm.run_label(&cog, "startup").expect("runs");
        let got: Vec<f64> = (0..7).map(|k| vm.var(&format!("r{k}")).num()).collect();
        assert_eq!(got, vec![2.0, 1.0, 0.0, 1.0, 0.0, 1.0, 0.0]);
    }
}