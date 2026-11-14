//! Test support (never product): the textual scan behind the `boundary` tests.
//!
//! It is **architectural, not a security barrier**. It reads the Rust text under `crates/*/src`
//! and answers one question: which of that text is *not provably test-only*? Everything else is
//! the production text the gate rules are checked against.
//!
//! What it does, and no more:
//!
//! * lexes each file with the real Rust lexer (`proc_macro2`: raw strings, `'{'` against
//!   lifetimes, nested comments, delimiters are its job, not a brace counter's);
//! * drops comments and doc attributes;
//! * drops an item only when its `cfg` is **provably false in a build without `test`**
//!   (`cfg(test)`, `cfg(all(test, ...))`, `cfg(not(not(test)))`, an inner `#![cfg(test)]`). Any
//!   other predicate (`any(test, feature = "x")`, `not(test)`, `feature = "x"`, a malformed one)
//!   is ambiguous and **stays in** the production text;
//! * resolves out-of-line modules (`mod x;`, `#[path]`, inline nesting) from the crate roots, so a
//!   file is test-only because a real `#[cfg(test)] mod` declares it, **never because of its
//!   name**. A file nothing declares (an orphan) is scanned as production.
//!
//! What it does not do: expand macros, follow `include!`, evaluate `cfg_attr`, see `#[test]`
//! functions outside a `cfg(test)` item as test-only, or look at anything outside `crates/*/src`.
//! A heuristic item end (below) may leave test text in (safe) but never removes more than the
//! item's own tokens when the item is one the rules know.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fs;
use std::path::Path;
use std::str::FromStr;

use proc_macro2::{Delimiter, Group, TokenStream, TokenTree};

/// A source file reduced to its production text. `code` has no whitespace: it is token text, so
/// a needle without spaces (`TransitionAdmission::Accept`) matches however the file is formatted.
pub struct Source {
    /// Path from `crates/`, forward slashes.
    pub path: String,
    pub code: String,
}

impl Source {
    pub fn count(&self, needle: &str) -> usize {
        self.code.matches(needle).count()
    }
}

/// The three-valued answer of a `cfg` predicate in a build without `test`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Tri {
    False,
    True,
    Unknown,
}

impl Tri {
    fn negate(self) -> Self {
        match self {
            Self::False => Self::True,
            Self::True => Self::False,
            Self::Unknown => Self::Unknown,
        }
    }
}

fn split_commas(stream: TokenStream) -> Vec<Vec<TokenTree>> {
    let mut parts = vec![Vec::new()];
    for token in stream {
        match &token {
            TokenTree::Punct(punct) if punct.as_char() == ',' => parts.push(Vec::new()),
            _ => parts.last_mut().unwrap().push(token),
        }
    }
    parts.retain(|part| !part.is_empty());
    parts
}

/// `test` is false; every other atom, and every shape this does not know, is `Unknown`.
fn eval_predicate(tokens: &[TokenTree]) -> Tri {
    let Some(TokenTree::Ident(name)) = tokens.first() else {
        return Tri::Unknown;
    };
    match tokens.get(1) {
        None if name == "test" => Tri::False,
        Some(TokenTree::Group(args))
            if tokens.len() == 2 && args.delimiter() == Delimiter::Parenthesis =>
        {
            let args: Vec<Tri> = split_commas(args.stream())
                .iter()
                .map(|part| eval_predicate(part))
                .collect();
            if name == "all" {
                if args.contains(&Tri::False) {
                    Tri::False
                } else if args.iter().all(|value| *value == Tri::True) {
                    Tri::True
                } else {
                    Tri::Unknown
                }
            } else if name == "any" {
                if args.contains(&Tri::True) {
                    Tri::True
                } else if !args.is_empty() && args.iter().all(|value| *value == Tri::False) {
                    Tri::False
                } else {
                    Tri::Unknown
                }
            } else if name == "not" && args.len() == 1 {
                args[0].negate()
            } else {
                Tri::Unknown
            }
        }
        _ => Tri::Unknown,
    }
}

struct Attr {
    start: usize,
    end: usize,
    inner: bool,
    body: Vec<TokenTree>,
}

fn attr_at(tokens: &[TokenTree], start: usize) -> Option<Attr> {
    let TokenTree::Punct(hash) = tokens.get(start)? else {
        return None;
    };
    if hash.as_char() != '#' {
        return None;
    }
    let (inner, at) = match tokens.get(start + 1)? {
        TokenTree::Punct(bang) if bang.as_char() == '!' => (true, start + 2),
        _ => (false, start + 1),
    };
    let TokenTree::Group(group) = tokens.get(at)? else {
        return None;
    };
    (group.delimiter() == Delimiter::Bracket).then(|| Attr {
        start,
        end: at + 1,
        inner,
        body: group.stream().into_iter().collect(),
    })
}

impl Attr {
    fn is_doc(&self) -> bool {
        matches!(self.body.first(), Some(TokenTree::Ident(name)) if name == "doc")
    }

    /// `Some(value)` for a `cfg(...)` attribute.
    fn cfg(&self) -> Option<Tri> {
        let [TokenTree::Ident(name), TokenTree::Group(args)] = self.body.as_slice() else {
            return None;
        };
        if name != "cfg" || args.delimiter() != Delimiter::Parenthesis {
            return None;
        }
        let parts = split_commas(args.stream());
        Some(match parts.as_slice() {
            [predicate] => eval_predicate(predicate),
            _ => Tri::Unknown,
        })
    }

    /// The file named by `#[path = "..."]`.
    fn path(&self) -> Option<String> {
        let [
            TokenTree::Ident(name),
            TokenTree::Punct(eq),
            TokenTree::Literal(text),
        ] = self.body.as_slice()
        else {
            return None;
        };
        if name != "path" || eq.as_char() != '=' {
            return None;
        }
        let text = text.to_string();
        text.strip_prefix('"')?
            .strip_suffix('"')
            .map(str::to_string)
    }

    fn test_only(&self) -> bool {
        self.cfg() == Some(Tri::False)
    }
}

/// Where the item that starts at `from` ends (exclusive). Delimited groups are single tokens, so
/// a `;` inside `[u8; 4]` or a `}` in a string cannot end an item. An item the rules do not know
/// ends early, which leaves text in (safe), never removes a neighbour.
fn item_end(tokens: &[TokenTree], from: usize) -> usize {
    let ident = |at: usize| match tokens.get(at) {
        Some(TokenTree::Ident(name)) => Some(name.to_string()),
        _ => None,
    };
    let mut at = from;
    if ident(at).as_deref() == Some("pub") {
        at += 1;
        if matches!(tokens.get(at), Some(TokenTree::Group(group)) if group.delimiter() == Delimiter::Parenthesis)
        {
            at += 1;
        }
    }
    loop {
        match ident(at).as_deref() {
            Some("unsafe" | "async" | "default" | "safe") => at += 1,
            Some("extern") => {
                at += 1;
                if matches!(tokens.get(at), Some(TokenTree::Literal(_))) {
                    at += 1;
                }
            }
            Some("const")
                if matches!(
                    ident(at + 1).as_deref(),
                    Some("fn" | "unsafe" | "async" | "extern")
                ) =>
            {
                at += 1;
            }
            _ => break,
        }
    }
    let keyword = ident(at);
    let has_body = matches!(
        keyword.as_deref(),
        Some(
            "fn" | "impl" | "trait" | "struct" | "enum" | "union" | "mod" | "macro_rules" | "macro"
        )
    );
    let const_block = keyword.as_deref() == Some("const")
        && matches!(tokens.get(at + 1), Some(TokenTree::Group(group)) if group.delimiter() == Delimiter::Brace);
    let ends_at_semicolon = !const_block
        && matches!(
            keyword.as_deref(),
            Some("const" | "static" | "type" | "use" | "let" | "crate")
        );
    for (index, token) in tokens.iter().enumerate().skip(from) {
        match token {
            TokenTree::Punct(punct) if punct.as_char() == ';' => return index + 1,
            TokenTree::Punct(punct)
                if punct.as_char() == ',' && !has_body && !ends_at_semicolon =>
            {
                return index + 1;
            }
            TokenTree::Group(group)
                if group.delimiter() == Delimiter::Brace && !ends_at_semicolon =>
            {
                return index + 1;
            }
            _ => {}
        }
    }
    tokens.len()
}

/// An out-of-line module declaration found in a file.
struct Decl {
    /// The inline modules it sits in (`mod a { mod b; }` is `[a]` for `b`).
    chain: Vec<String>,
    name: String,
    path: Option<String>,
    /// Declared inside something provably test-only.
    test_only: bool,
}

#[derive(Default)]
struct Walk {
    chain: Vec<String>,
    decls: Vec<Decl>,
}

fn is_ident(token: &TokenTree, word: &str) -> bool {
    matches!(token, TokenTree::Ident(name) if name == word)
}

/// Walks a token sequence: drops comments' doc attributes and provably test-only items, records
/// module declarations (also inside dropped items, flagged `test_only`), recurses into groups.
fn process(tokens: Vec<TokenTree>, walk: &mut Walk, mut dead: bool) -> Vec<TokenTree> {
    let mut out = Vec::new();
    let mut at = 0;
    // `#![cfg(test)]` at the head of a file or module body makes the whole sequence test-only.
    while let Some(attr) = attr_at(&tokens, at).filter(|attr| attr.inner) {
        if attr.test_only() {
            dead = true;
        } else if !attr.is_doc() {
            out.extend(tokens[attr.start..attr.end].iter().cloned());
        }
        at = attr.end;
    }
    let mut pending_path: Option<String> = None;
    let mut after_pub = false;
    while at < tokens.len() {
        // A doc attribute anywhere (an inner `//!` after other text) is a comment, not code.
        if let Some(attr) = attr_at(&tokens, at).filter(|attr| attr.inner && attr.is_doc()) {
            at = attr.end;
            continue;
        }
        if let Some(first) = attr_at(&tokens, at).filter(|attr| !attr.inner) {
            let mut attrs = vec![first];
            while let Some(next) = attr_at(&tokens, attrs.last().unwrap().end).filter(|a| !a.inner)
            {
                attrs.push(next);
            }
            let item = attrs.last().unwrap().end;
            if attrs.iter().any(Attr::test_only) {
                // Walk the dropped item only to record the modules it declares; its `#[path]`
                // travels with it, its `cfg` does not (it would drop the item again).
                let end = item_end(&tokens, item);
                let mut dropped: Vec<TokenTree> = attrs
                    .iter()
                    .filter(|attr| attr.path().is_some())
                    .flat_map(|attr| tokens[attr.start..attr.end].iter().cloned())
                    .collect();
                dropped.extend(tokens[item..end].iter().cloned());
                process(dropped, walk, true);
                at = end;
                pending_path = None;
                after_pub = false;
                continue;
            }
            for attr in attrs.iter().filter(|attr| !attr.is_doc()) {
                out.extend(tokens[attr.start..attr.end].iter().cloned());
            }
            pending_path = attrs.iter().find_map(Attr::path);
            after_pub = false;
            at = item;
            continue;
        }
        let token = &tokens[at];
        let was_pub = std::mem::take(&mut after_pub);
        match token {
            TokenTree::Ident(word) if word == "mod" => {
                let name = match tokens.get(at + 1) {
                    Some(TokenTree::Ident(name)) => Some(name.to_string()),
                    _ => None,
                };
                match (name, tokens.get(at + 2)) {
                    (Some(name), Some(TokenTree::Punct(semi))) if semi.as_char() == ';' => {
                        walk.decls.push(Decl {
                            chain: walk.chain.clone(),
                            name,
                            path: pending_path.take(),
                            test_only: dead,
                        });
                        out.extend(tokens[at..at + 3].iter().cloned());
                        at += 3;
                    }
                    (Some(name), Some(TokenTree::Group(body)))
                        if body.delimiter() == Delimiter::Brace =>
                    {
                        walk.chain.push(name);
                        let inner = process(body.stream().into_iter().collect(), walk, dead);
                        walk.chain.pop();
                        out.extend(tokens[at..at + 2].iter().cloned());
                        out.push(TokenTree::Group(Group::new(
                            Delimiter::Brace,
                            inner.into_iter().collect(),
                        )));
                        pending_path = None;
                        at += 3;
                    }
                    _ => {
                        pending_path = None;
                        out.push(token.clone());
                        at += 1;
                    }
                }
            }
            TokenTree::Group(group) => {
                let inner = process(group.stream().into_iter().collect(), walk, dead);
                out.push(TokenTree::Group(Group::new(
                    group.delimiter(),
                    inner.into_iter().collect(),
                )));
                if !(was_pub && group.delimiter() == Delimiter::Parenthesis) {
                    pending_path = None;
                }
                at += 1;
            }
            _ => {
                if is_ident(token, "pub") {
                    after_pub = true;
                } else {
                    pending_path = None;
                }
                out.push(token.clone());
                at += 1;
            }
        }
    }
    if dead {
        out.clear();
    }
    out
}

/// A lexed file: its production text and the modules it declares.
pub struct Parsed {
    pub code: String,
    decls: Vec<Decl>,
}

/// Lexes `text`. A file the lexer rejects is an error (the scan fails closed, it never skips).
pub fn parse(text: &str) -> Result<Parsed, String> {
    let stream = TokenStream::from_str(text).map_err(|error| format!("not lexable: {error}"))?;
    let mut walk = Walk::default();
    let out = process(stream.into_iter().collect(), &mut walk, false);
    let code = out
        .into_iter()
        .collect::<TokenStream>()
        .to_string()
        .split_whitespace()
        .collect();
    Ok(Parsed {
        code,
        decls: walk.decls,
    })
}

/// The production text of a single file (what `parse` keeps).
pub fn production_text(text: &str) -> String {
    parse(text).expect("lexable sample").code
}

// ---- the module graph ------------------------------------------------------------------------

fn parent(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(dir, _)| dir)
}

fn join(base: &str, rest: &str) -> String {
    if base.is_empty() {
        rest.to_string()
    } else {
        format!("{base}/{rest}")
    }
}

fn normalize(path: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            other => parts.push(other),
        }
    }
    parts.join("/")
}

/// The files a declaration may name, with whether each is a `mod.rs`-style owner of a directory.
fn candidates(file: &str, owns_dir: bool, decl: &Decl) -> Vec<(String, bool)> {
    let stem = file
        .rsplit('/')
        .next()
        .and_then(|name| name.strip_suffix(".rs"))
        .unwrap_or("");
    let own = if owns_dir {
        parent(file).to_string()
    } else {
        join(parent(file), stem)
    };
    let inline = decl
        .chain
        .iter()
        .fold(own, |acc, module| join(&acc, module));
    match &decl.path {
        // `#[path]` is relative to the file's directory, or to the inline modules' directory.
        Some(path) => {
            let base = if decl.chain.is_empty() {
                parent(file).to_string()
            } else {
                inline
            };
            vec![(normalize(&join(&base, path)), true)]
        }
        None => vec![
            (join(&inline, &format!("{}.rs", decl.name)), false),
            (join(&inline, &format!("{}/mod.rs", decl.name)), true),
        ],
    }
}

/// Every file reachable from the roots, following (`test_only` too) or not the test-only
/// declarations, plus the declarations that name no file.
fn follow(
    parsed: &BTreeMap<String, Parsed>,
    roots: &[String],
    through_tests: bool,
) -> (BTreeSet<String>, Vec<String>) {
    let mut owns: BTreeMap<String, bool> = BTreeMap::new();
    let mut queue: VecDeque<String> = VecDeque::new();
    for root in roots.iter().filter(|root| parsed.contains_key(*root)) {
        owns.insert(root.clone(), true);
        queue.push_back(root.clone());
    }
    let mut unresolved = Vec::new();
    while let Some(file) = queue.pop_front() {
        let owns_dir = owns[&file];
        for decl in &parsed[&file].decls {
            if decl.test_only && !through_tests {
                continue;
            }
            let found = candidates(&file, owns_dir, decl)
                .into_iter()
                .find(|(path, _)| parsed.contains_key(path));
            match found {
                Some((path, mod_rs)) => {
                    if !owns.contains_key(&path) {
                        owns.insert(path.clone(), mod_rs);
                        queue.push_back(path);
                    }
                }
                None => unresolved.push(format!("{file}: mod {}", decl.name)),
            }
        }
    }
    (owns.into_keys().collect(), unresolved)
}

/// What the scan decided about a set of files.
pub struct Scan {
    /// The production text: every file a production declaration reaches, plus the orphans.
    pub sources: Vec<Source>,
    /// Files only a provably test-only declaration reaches.
    pub test_only: Vec<String>,
    /// Files no declaration reaches. They are scanned as production (never skipped).
    pub orphans: Vec<String>,
    /// Production `mod x;` declarations that name no file.
    pub unresolved: Vec<String>,
}

/// `files` maps a path (forward slashes) to its text; `roots` are the crate roots.
pub fn scan(files: &BTreeMap<String, String>, roots: &[String]) -> Result<Scan, String> {
    let mut parsed = BTreeMap::new();
    for (path, text) in files {
        parsed.insert(
            path.clone(),
            parse(text).map_err(|error| format!("{path}: {error}"))?,
        );
    }
    let (production, unresolved) = follow(&parsed, roots, false);
    let (declared, _) = follow(&parsed, roots, true);
    let orphans: Vec<String> = parsed
        .keys()
        .filter(|path| !declared.contains(*path))
        .cloned()
        .collect();
    let test_only = declared.difference(&production).cloned().collect();
    let sources = parsed
        .into_iter()
        .filter(|(path, _)| production.contains(path) || orphans.contains(path))
        .map(|(path, parsed)| Source {
            path,
            code: parsed.code,
        })
        .collect();
    Ok(Scan {
        sources,
        test_only,
        orphans,
        unresolved,
    })
}

/// The crate roots among `files` (`lib.rs`, `main.rs`, `bin/*.rs`, `bin/*/main.rs`).
pub fn crate_roots(files: &BTreeMap<String, String>) -> Vec<String> {
    files
        .keys()
        .filter(|path| {
            let parts: Vec<&str> = path.split('/').collect();
            match parts.as_slice() {
                [_, "src", "lib.rs" | "main.rs"] => true,
                [_, "src", "bin", name] => name.ends_with(".rs"),
                [_, "src", "bin", _, "main.rs"] => true,
                _ => false,
            }
        })
        .cloned()
        .collect()
}

fn walk(dir: &Path, base: &Path, found: &mut BTreeMap<String, String>) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            walk(&path, base, found);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            let relative = path
                .strip_prefix(base)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            found.insert(relative, fs::read_to_string(&path).unwrap());
        }
    }
}

/// Every `.rs` file under `crates/*/src`, keyed from `crates/`.
pub fn read_crates() -> BTreeMap<String, String> {
    let crates = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf();
    let mut found = BTreeMap::new();
    for member in fs::read_dir(&crates).unwrap() {
        let src = member.unwrap().path().join("src");
        if src.is_dir() {
            walk(&src, &crates, &mut found);
        }
    }
    found
}

/// The scan of the repository as it stands.
pub fn scan_repository() -> Scan {
    let files = read_crates();
    let roots = crate_roots(&files);
    scan(&files, &roots)
        .unwrap_or_else(|error| panic!("the production scan failed closed: {error}"))
}
