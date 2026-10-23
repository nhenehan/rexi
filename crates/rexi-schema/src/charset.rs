//! Works out whether the `xs:pattern` facets of a string type give a restricted
//! character set ([EXI 7.1.10.1](https://www.w3.org/TR/exi/#restrictedCharSet)).
//!
//! The rules come from
//! [EXI Appendix E](https://www.w3.org/TR/exi/#regexToCharset). The character set
//! of a pattern is every character the pattern can match, with quantifiers
//! ignored. A type's set is the union of the sets of its patterns. The set is
//! restricted when it has fewer than 256 characters and all of them are in the
//! Basic Multilingual Plane (at most U+FFFF).
//!
//! The parser here follows the regular expression grammar of
//! [XML Schema Part 2, Appendix F](https://www.w3.org/TR/xmlschema-2/#regexs), but
//! it only collects characters. It does not check that a value matches. Anything
//! that can match a large set of characters (`.`, `[^...]`, `\d`, `\w`, `\p{..}`
//! and the like) makes the set not restricted right away.
//!
//! Character references such as `&#x41;` are not handled here: the XML parser has
//! already turned them into characters.

/// What the patterns of one type say about its character set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CharSet {
    /// A restricted character set with this many characters (fewer than 256, all
    /// in the BMP). It can be 0, for example for an empty pattern.
    Restricted(usize),
    /// The set is large, or has characters outside the BMP. Values are plain
    /// strings.
    NotRestricted,
    /// A pattern could not be read. The caller should not guess.
    Unreadable,
}

/// Highest code point in the Basic Multilingual Plane.
const BMP_MAX: u32 = 0xFFFF;

/// A restricted set has fewer characters than this.
const LIMIT: u64 = 256;

/// The character set of a type whose own patterns are `patterns`. Several
/// patterns on one type mean "any of them", so their sets are joined.
pub fn analyze<S: AsRef<str>>(patterns: &[S]) -> CharSet {
    let mut all = Ranges::default();
    let mut unreadable = false;
    for p in patterns {
        match pattern_set(p.as_ref()) {
            Ok(set) => all.extend(set),
            Err(Stop::NotRestricted) => return CharSet::NotRestricted,
            Err(Stop::Unreadable) => unreadable = true,
        }
    }
    if unreadable {
        return CharSet::Unreadable;
    }
    all.normalize();
    classify(&all)
}

/// Decides the answer for a finished set.
fn classify(set: &Ranges) -> CharSet {
    if set.0.iter().any(|&(_, hi)| hi > BMP_MAX) {
        return CharSet::NotRestricted;
    }
    let n = set.count();
    if n < LIMIT {
        CharSet::Restricted(n as usize)
    } else {
        CharSet::NotRestricted
    }
}

/// Why parsing stopped early.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stop {
    /// The pattern uses something with a large set of characters.
    NotRestricted,
    /// The pattern is not a valid XSD regular expression.
    Unreadable,
}

type Parse<T> = Result<T, Stop>;

/// A set of characters, held as inclusive code point ranges. After
/// [`Ranges::normalize`] the ranges are sorted and do not touch or overlap.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Ranges(Vec<(u32, u32)>);

impl Ranges {
    fn add(&mut self, lo: u32, hi: u32) {
        self.0.push((lo, hi));
    }

    fn add_char(&mut self, c: char) {
        let c = c as u32;
        self.add(c, c);
    }

    fn extend(&mut self, other: Ranges) {
        self.0.extend(other.0);
    }

    /// Sorts the ranges and merges ones that overlap or sit next to each other.
    fn normalize(&mut self) {
        self.0.sort_unstable();
        let mut out: Vec<(u32, u32)> = Vec::with_capacity(self.0.len());
        for &(lo, hi) in &self.0 {
            match out.last_mut() {
                Some(last) if lo <= last.1.saturating_add(1) => last.1 = last.1.max(hi),
                _ => out.push((lo, hi)),
            }
        }
        self.0 = out;
    }

    /// Takes away every character of `other`. Both sets are normalized first.
    fn subtract(&mut self, mut other: Ranges) {
        self.normalize();
        other.normalize();
        let mut out = Vec::new();
        for &(lo, hi) in &self.0 {
            let mut start = lo;
            let mut done = false;
            for &(olo, ohi) in &other.0 {
                if ohi < start || olo > hi {
                    continue;
                }
                if olo > start {
                    out.push((start, olo - 1));
                }
                if ohi >= hi {
                    done = true;
                    break;
                }
                start = ohi + 1;
            }
            if !done {
                out.push((start, hi));
            }
        }
        self.0 = out;
    }

    /// Number of characters. Call on a normalized set.
    fn count(&self) -> u64 {
        self.0.iter().map(|&(lo, hi)| u64::from(hi - lo) + 1).sum()
    }
}

/// The set of one pattern.
fn pattern_set(pattern: &str) -> Parse<Ranges> {
    let chars: Vec<char> = pattern.chars().collect();
    let mut p = Parser {
        chars: &chars,
        pos: 0,
        set: Ranges::default(),
    };
    p.reg_exp(0)?;
    if p.pos != chars.len() {
        // Only a `)` with no `(` stops `reg_exp` before the end.
        return Err(Stop::Unreadable);
    }
    Ok(p.set)
}

/// Nesting depth of `(...)` allowed before a pattern is called unreadable. It
/// keeps a hostile schema from overflowing the stack.
const MAX_GROUP_DEPTH: usize = 64;

struct Parser<'a> {
    chars: &'a [char],
    pos: usize,
    /// Every character seen so far.
    set: Ranges,
}

impl Parser<'_> {
    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn peek_at(&self, n: usize) -> Option<char> {
        self.chars.get(self.pos + n).copied()
    }

    fn next(&mut self) -> Parse<char> {
        let c = self.peek().ok_or(Stop::Unreadable)?;
        self.pos += 1;
        Ok(c)
    }

    /// `regExp ::= branch ( '|' branch )*`. Stops at the end or at a `)` that
    /// closes the group this call is in (the `)` is left for the caller).
    fn reg_exp(&mut self, depth: usize) -> Parse<()> {
        if depth > MAX_GROUP_DEPTH {
            return Err(Stop::Unreadable);
        }
        // `true` once a piece has an atom, so a quantifier has something to follow.
        let mut have_atom = false;
        while let Some(c) = self.peek() {
            match c {
                '|' => {
                    self.pos += 1;
                    have_atom = false;
                }
                ')' => {
                    if depth == 0 {
                        return Err(Stop::Unreadable);
                    }
                    return Ok(());
                }
                '?' | '*' | '+' | '{' => {
                    if !have_atom {
                        return Err(Stop::Unreadable);
                    }
                    self.quantifier()?;
                    // `a**` is not allowed: one quantifier per atom.
                    have_atom = false;
                }
                _ => {
                    self.atom(depth)?;
                    have_atom = true;
                }
            }
        }
        if depth > 0 {
            // A `(` with no `)`.
            return Err(Stop::Unreadable);
        }
        Ok(())
    }

    /// Skips a quantifier: `?`, `*`, `+` or `{n}`, `{n,}`, `{n,m}`. Quantifiers
    /// do not change the set of characters.
    fn quantifier(&mut self) -> Parse<()> {
        if self.next()? != '{' {
            return Ok(());
        }
        let mut seen_digit = false;
        loop {
            match self.next()? {
                '}' if seen_digit => return Ok(()),
                d if d.is_ascii_digit() => seen_digit = true,
                ',' if seen_digit => {}
                _ => return Err(Stop::Unreadable),
            }
        }
    }

    /// `atom ::= Char | charClass | '(' regExp ')'`.
    fn atom(&mut self, depth: usize) -> Parse<()> {
        match self.next()? {
            '(' => {
                self.reg_exp(depth + 1)?;
                // `reg_exp` only returns early at a `)`.
                self.next()?;
                Ok(())
            }
            '[' => {
                let class = self.class_body()?;
                self.set.extend(class);
                Ok(())
            }
            // The wildcard matches almost any character.
            '.' => Err(Stop::NotRestricted),
            '\\' => {
                let mut tmp = Ranges::default();
                self.escape(&mut tmp)?;
                self.set.extend(tmp);
                Ok(())
            }
            // Characters that must be escaped outside a class.
            ']' | '}' => Err(Stop::Unreadable),
            c => {
                self.set.add_char(c);
                Ok(())
            }
        }
    }

    /// Reads an escape after its `\` and adds what it matches to `out`.
    ///
    /// - `\n \r \t` and an escaped symbol are one character.
    /// - `\s` is the four XML space characters.
    /// - `\S \d \D \w \W \i \I \c \C` and `\p{..} \P{..}` match large sets.
    ///
    /// Returns the character for a single-character escape, so a class can use it
    /// as the start or end of a range.
    fn escape(&mut self, out: &mut Ranges) -> Parse<Option<char>> {
        let c = match self.next()? {
            'n' => '\n',
            'r' => '\r',
            't' => '\t',
            c @ ('\\' | '|' | '.' | '-' | '^' | '?' | '*' | '+' | '{' | '}' | '(' | ')' | '['
            | ']') => c,
            's' => {
                for c in [' ', '\t', '\n', '\r'] {
                    out.add_char(c);
                }
                return Ok(None);
            }
            'S' | 'd' | 'D' | 'w' | 'W' | 'i' | 'I' | 'c' | 'C' | 'p' | 'P' => {
                return Err(Stop::NotRestricted);
            }
            _ => return Err(Stop::Unreadable),
        };
        out.add_char(c);
        Ok(Some(c))
    }

    /// Reads a character class after its `[`, up to and including the `]`.
    ///
    /// `charGroup ::= posCharGroup | negCharGroup | charClassSub`, where
    /// `charClassSub` is a group, then `-`, then another class to take away, as in
    /// `[a-z-[aeiou]]`. A negated group `[^...]` matches almost everything.
    fn class_body(&mut self) -> Parse<Ranges> {
        if self.peek() == Some('^') {
            return Err(Stop::NotRestricted);
        }
        let mut set = Ranges::default();
        let mut items = 0usize;
        loop {
            let c = self.next()?;
            match c {
                ']' if items > 0 => return Ok(set),
                // An empty class `[]` is not allowed.
                ']' => return Err(Stop::Unreadable),
                '-' if self.peek() == Some('[') && items > 0 => {
                    // Subtraction: `-[...]` must be the last thing in the class.
                    self.pos += 1;
                    let minus = self.class_body()?;
                    if self.next()? != ']' {
                        return Err(Stop::Unreadable);
                    }
                    set.subtract(minus);
                    return Ok(set);
                }
                // A bare `[` is not allowed inside a class.
                '[' => return Err(Stop::Unreadable),
                // A plain `-` is only allowed first or just before the `]`.
                '-' if items > 0 && self.peek() != Some(']') => return Err(Stop::Unreadable),
                _ => {
                    let start = if c == '\\' {
                        self.escape(&mut set)?
                    } else {
                        set.add_char(c);
                        Some(c)
                    };
                    items += 1;
                    // A range `a-z`. A `-` just before `]` is a plain dash, and
                    // `-[` starts a subtraction, so neither is a range.
                    if let Some(lo) = start
                        && self.peek() == Some('-')
                        && !matches!(self.peek_at(1), Some(']') | Some('[') | None)
                    {
                        self.pos += 1;
                        let hi = match self.next()? {
                            '\\' => {
                                let mut tmp = Ranges::default();
                                self.escape(&mut tmp)?.ok_or(Stop::Unreadable)?
                            }
                            '[' => return Err(Stop::Unreadable),
                            h => h,
                        };
                        if hi < lo {
                            return Err(Stop::Unreadable);
                        }
                        set.add(lo as u32, hi as u32);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one(p: &str) -> CharSet {
        analyze(&[p])
    }

    #[test]
    fn single_characters_and_classes() {
        assert_eq!(one("[abc\u{3095}\u{00A9}]"), CharSet::Restricted(5));
        assert_eq!(one("[A-X]"), CharSet::Restricted(24));
        assert_eq!(one("[\u{03D7}-\u{03DA}]"), CharSet::Restricted(4));
        assert_eq!(one("[a-z]{3}"), CharSet::Restricted(26));
        assert_eq!(one("abc"), CharSet::Restricted(3));
        assert_eq!(one("aaa"), CharSet::Restricted(1));
    }

    #[test]
    fn quantifiers_are_ignored() {
        assert_eq!(one("[0-9]{3}-[0-9]{2}-[0-9]{4}"), CharSet::Restricted(11));
        assert_eq!(one("a?b*c+d{2,}e{1,3}"), CharSet::Restricted(5));
    }

    #[test]
    fn branches_and_groups() {
        assert_eq!(one("ab|cd"), CharSet::Restricted(4));
        assert_eq!(one("(a|(b|c))+x"), CharSet::Restricted(4));
        assert_eq!(analyze(&["ab", "bc"]), CharSet::Restricted(3));
    }

    #[test]
    fn escapes() {
        assert_eq!(one(r"\n\r\t"), CharSet::Restricted(3));
        assert_eq!(one(r"\s"), CharSet::Restricted(4));
        assert_eq!(one(r"[\s]"), CharSet::Restricted(4));
        assert_eq!(
            one(r"\\\|\.\-\^\?\*\+\{\}\(\)\[\]"),
            CharSet::Restricted(14)
        );
        assert_eq!(one(r"[\--/]"), CharSet::Restricted(3));
        // `^` and `$` are plain characters in XSD patterns.
        assert_eq!(one("^a$"), CharSet::Restricted(3));
    }

    #[test]
    fn dashes_in_classes() {
        assert_eq!(one("[-a]"), CharSet::Restricted(2));
        assert_eq!(one("[a-]"), CharSet::Restricted(2));
        assert_eq!(one("[+-]"), CharSet::Restricted(2));
    }

    #[test]
    fn subtraction() {
        assert_eq!(one("[a-z-[aeiou]]"), CharSet::Restricted(21));
        assert_eq!(one("[a-z-[b-y]]"), CharSet::Restricted(2));
        assert_eq!(one("[a-c-[a-c]]"), CharSet::Restricted(0));
        assert_eq!(one("[a-z-[a-z-[m]]]"), CharSet::Restricted(1));
        // Taking a range away can make a big set small.
        assert_eq!(
            one("[\u{0}-\u{FFFF}-[\u{1}-\u{FFFF}]]"),
            CharSet::Restricted(1)
        );
    }

    #[test]
    fn empty_set_is_restricted() {
        assert_eq!(one(""), CharSet::Restricted(0));
        assert_eq!(one("()"), CharSet::Restricted(0));
        assert_eq!(analyze::<&str>(&[]), CharSet::Restricted(0));
    }

    #[test]
    fn large_sets_are_not_restricted() {
        for p in [
            ".*", "[^:]", r"\d+", r"\D", r"\w", r"\W", r"\i", r"\I", r"\c", r"\C", r"\S",
            r"\p{Lm}", r"\P{L}", r"[\d]", "a|.",
        ] {
            assert_eq!(one(p), CharSet::NotRestricted, "{p}");
        }
        // Inside a subtraction too: a large set anywhere is not restricted.
        assert_eq!(one(r"[a-z-[\d]]"), CharSet::NotRestricted);
        assert_eq!(one("[a-z-[^a]]"), CharSet::NotRestricted);
        // 256 characters is one too many.
        assert_eq!(one("[\u{0}-\u{FF}]"), CharSet::NotRestricted);
        assert_eq!(one("[\u{0}-\u{FE}]"), CharSet::Restricted(255));
        // One large pattern makes the whole type not restricted.
        assert_eq!(analyze(&["[a]", ".*"]), CharSet::NotRestricted);
    }

    #[test]
    fn outside_the_bmp_is_not_restricted() {
        assert_eq!(one("[ABC\u{10FFF}]"), CharSet::NotRestricted);
        assert_eq!(one("\u{1F600}"), CharSet::NotRestricted);
    }

    #[test]
    fn unreadable_patterns() {
        for p in [
            "(", ")", "a)", "[", "[]", "[a", "]", "}", "*a", "a**", "a{", "a{x}", "[z-a]", r"\q",
            r"\", "[a[b]]", "[a-[b]", r"[\s-z]", "[a-c-e]", r"[\/]", "[]a]",
        ] {
            assert_eq!(one(p), CharSet::Unreadable, "{p}");
        }
        // Deep groups stop before the stack does.
        let deep = format!("{}a{}", "(".repeat(100), ")".repeat(100));
        assert_eq!(one(&deep), CharSet::Unreadable);
        // Not restricted beats unreadable: the union is large either way.
        assert_eq!(analyze(&["(", ".*"]), CharSet::NotRestricted);
    }
}
