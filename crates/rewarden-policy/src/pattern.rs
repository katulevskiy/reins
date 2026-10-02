use std::fmt;

use regex::{Regex, RegexBuilder};
use rewarden_proto::normalize_address;
use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::PolicyError;

pub const MAX_PATTERN_LEN: usize = 512;
const COMPILED_SIZE_LIMIT: usize = 1 << 18;

/// How a [`Pattern`] was built; part of its identity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    /// Matches anywhere in the input.
    Find,
    /// The whole input must match.
    Full,
    /// Search for literal text (a `Find` over the escaped text).
    Literal,
}

/// A validated, case-insensitive regular expression.
///
/// Backed by the `regex` crate, whose matching time is linear in the input, so
/// neither a hostile pattern nor a hostile message body can cause catastrophic
/// backtracking.
///
/// The construction mode is part of the value: `Pattern::full("a")` and
/// `Pattern::find("a")` are different patterns and compare unequal.
///
/// Serde: a `Pattern` is a bare string that always deserializes with search
/// (`find`) semantics. `Find` and `Full` patterns serialize their source; a
/// `Literal` pattern serializes its *escaped* regex, so it round-trips as a
/// `Find` pattern that matches identically (though it no longer compares equal
/// to the original, and `source()` then returns the escaped text).
#[derive(Clone)]
pub struct Pattern {
    mode: Mode,
    source: String,
    regex: Regex,
}

impl Pattern {
    /// Search semantics: matches when the pattern occurs anywhere in the input.
    pub fn find(source: &str) -> Result<Self, PolicyError> {
        Self::build(Mode::Find, source, source)
    }

    /// Full-match semantics: the whole input must match.
    pub fn full(source: &str) -> Result<Self, PolicyError> {
        // Validate the source on its own first: only a balanced, standalone
        // pattern can be safely wrapped in a group without escaping it.
        Self::build(Mode::Full, source, source)?;
        Self::build(Mode::Full, source, &format!("^(?:{source})$"))
    }

    /// Search for `text` verbatim (case-insensitively), with no regex syntax.
    ///
    /// The length limit applies to `text`; its escaped form must also fit in
    /// [`MAX_PATTERN_LEN`] so that the serialized pattern can always be read back.
    pub fn literal(text: &str) -> Result<Self, PolicyError> {
        let escaped = regex::escape(text);
        if !text.is_empty() && escaped.len() > MAX_PATTERN_LEN {
            return Err(PolicyError::InvalidPattern {
                pattern: text.to_owned(),
                reason: format!("escaped length must be at most {MAX_PATTERN_LEN}"),
            });
        }
        Self::build(Mode::Literal, text, &escaped)
    }

    fn build(mode: Mode, source: &str, compiled: &str) -> Result<Self, PolicyError> {
        let error = |reason: String| PolicyError::InvalidPattern {
            pattern: source.to_owned(),
            reason,
        };
        if source.is_empty() || source.len() > MAX_PATTERN_LEN {
            return Err(error(format!("length must be 1..={MAX_PATTERN_LEN}")));
        }
        let regex = RegexBuilder::new(compiled)
            .case_insensitive(true)
            .size_limit(COMPILED_SIZE_LIMIT)
            .dfa_size_limit(COMPILED_SIZE_LIMIT)
            .build()
            .map_err(|e| error(e.to_string()))?;
        Ok(Self {
            mode,
            source: source.to_owned(),
            regex,
        })
    }

    /// The text the pattern was built from (for a literal, the original text).
    #[must_use]
    pub fn source(&self) -> &str {
        &self.source
    }

    #[must_use]
    pub fn is_match(&self, haystack: &str) -> bool {
        self.regex.is_match(haystack)
    }
}

impl fmt::Debug for Pattern {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("Pattern").field(&self.mode).field(&self.source).finish()
    }
}

impl PartialEq for Pattern {
    fn eq(&self, other: &Self) -> bool {
        self.mode == other.mode && self.source == other.source
    }
}

impl Eq for Pattern {}

impl Serialize for Pattern {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self.mode {
            Mode::Find | Mode::Full => serializer.serialize_str(&self.source),
            Mode::Literal => serializer.serialize_str(&regex::escape(&self.source)),
        }
    }
}

impl<'de> Deserialize<'de> for Pattern {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let source = String::deserialize(deserializer)?;
        Self::find(&source).map_err(D::Error::custom)
    }
}

/// Matches one normalized email address. Build with the constructors, which
/// normalize their input; a hand-built unnormalized variant simply never
/// matches (fails closed).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "AddrRuleRepr", into = "AddrRuleRepr")]
pub enum AddrRule {
    /// Exactly this address.
    Exact(String),
    /// Any address at exactly this domain (subdomains excluded).
    Domain(String),
    /// Whole-address regular expression. Build with [`AddrRule::regex`]; a
    /// hand-built rule around a non-`full` pattern never matches.
    Regex(Pattern),
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
enum AddrRuleRepr {
    Exact(String),
    Domain(String),
    Regex(String),
}

impl AddrRule {
    pub fn exact(addr: &str) -> Result<Self, PolicyError> {
        Ok(Self::Exact(normalize_address(addr)?))
    }

    pub fn domain(domain: &str) -> Result<Self, PolicyError> {
        let normalized = normalize_address(&format!("x@{}", domain.trim().trim_start_matches('@')))?;
        let (_, domain) = normalized.rsplit_once('@').expect("normalize_address output always contains '@'");
        Ok(Self::Domain(domain.to_owned()))
    }

    pub fn regex(source: &str) -> Result<Self, PolicyError> {
        Ok(Self::Regex(Pattern::full(source)?))
    }

    /// `addr` must already be normalized (see `rewarden_proto::normalize_address`).
    /// Anything else (display names, several addresses, whitespace, control
    /// characters, upper case) never matches, whatever the rule.
    #[must_use]
    pub fn matches(&self, addr: &str) -> bool {
        if normalize_address(addr).as_deref() != Ok(addr) {
            return false;
        }
        match self {
            Self::Exact(a) => a == addr,
            Self::Domain(d) => addr.rsplit_once('@').is_some_and(|(_, domain)| domain == d),
            Self::Regex(p) => p.mode == Mode::Full && p.is_match(addr),
        }
    }
}

impl TryFrom<AddrRuleRepr> for AddrRule {
    type Error = PolicyError;

    fn try_from(repr: AddrRuleRepr) -> Result<Self, Self::Error> {
        match repr {
            AddrRuleRepr::Exact(a) => Self::exact(&a),
            AddrRuleRepr::Domain(d) => Self::domain(&d),
            AddrRuleRepr::Regex(r) => Self::regex(&r),
        }
    }
}

impl From<AddrRule> for AddrRuleRepr {
    fn from(rule: AddrRule) -> Self {
        match rule {
            AddrRule::Exact(a) => Self::Exact(a),
            AddrRule::Domain(d) => Self::Domain(d),
            AddrRule::Regex(p) => Self::Regex(p.source),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use serde_json::json;

    use super::*;

    #[test]
    fn find_searches_case_insensitively() {
        let p = Pattern::find("invoice").unwrap();
        assert!(p.is_match("Your INVOICE #42"));
        assert!(!p.is_match("receipt"));
    }

    #[test]
    fn full_requires_whole_input() {
        let p = Pattern::full("bank").unwrap();
        assert!(p.is_match("BANK"));
        assert!(!p.is_match("notbank"));
        let alt = Pattern::full("a|ab").unwrap();
        assert!(alt.is_match("ab"), "alternation must be anchored as a group");
    }

    #[test]
    fn full_rejects_wrapper_escape() {
        // Would compile to ^(?:a)|(.*)$ and match everything if not validated alone.
        assert!(Pattern::full("a)|(.*").is_err());
        assert!(Pattern::full("a\\").is_err());
    }

    #[test]
    fn rejects_empty_long_and_invalid() {
        assert!(Pattern::find("").is_err());
        assert!(Pattern::find(&"a".repeat(MAX_PATTERN_LEN + 1)).is_err());
        assert!(Pattern::find("(").is_err());
        assert!(Pattern::find("a{100000}").is_err(), "compiled size limit");
    }

    #[test]
    fn pathological_pattern_is_linear() {
        let p = Pattern::full("(a+)+$").unwrap();
        let input = format!("{}b", "a".repeat(100_000));
        let start = Instant::now();
        assert!(!p.is_match(&input));
        assert!(start.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn pattern_serde_round_trip() {
        let p = Pattern::find("inv.*").unwrap();
        let v = serde_json::to_value(&p).unwrap();
        assert_eq!(v, json!("inv.*"));
        assert_eq!(serde_json::from_value::<Pattern>(v).unwrap(), p);
        assert!(serde_json::from_value::<Pattern>(json!("(")).is_err());
    }

    #[test]
    fn find_mode_addr_rule_never_matches() {
        // A hand-built rule around a search-mode pattern would match substrings.
        let r = AddrRule::Regex(Pattern::find("bank").unwrap());
        assert!(!r.matches("x@bank.com"));
        assert!(!r.matches("bank"));
        let r = AddrRule::Regex(Pattern::literal("x@bank.com").unwrap());
        assert!(!r.matches("x@bank.com"));
        // The validated constructor still works.
        assert!(AddrRule::regex(".*@bank[.]com").unwrap().matches("x@bank.com"));
    }

    #[test]
    fn pattern_equality_includes_mode() {
        assert_ne!(Pattern::full("a").unwrap(), Pattern::find("a").unwrap());
        assert_ne!(Pattern::literal("a").unwrap(), Pattern::find("a").unwrap());
        assert_eq!(Pattern::full("a").unwrap(), Pattern::full("a").unwrap());
        assert_eq!(Pattern::literal("a.b").unwrap(), Pattern::literal("a.b").unwrap());
    }

    #[test]
    fn literal_matches_text_not_regex() {
        let p = Pattern::literal("a.b").unwrap();
        assert!(p.is_match("xx A.B yy"), "search semantics, case-insensitive");
        assert!(!p.is_match("aXb"));
        assert_eq!(p.source(), "a.b");
        assert!(Pattern::literal("(").unwrap().is_match("f(x)"));
        assert!(Pattern::literal("").is_err());
        assert!(Pattern::literal(&"a".repeat(MAX_PATTERN_LEN + 1)).is_err());
        assert!(Pattern::literal(&"a".repeat(MAX_PATTERN_LEN)).is_ok());
        // Must survive a serde round trip, so the escaped form is bounded too.
        assert!(Pattern::literal(&".".repeat(MAX_PATTERN_LEN)).is_err());
    }

    #[test]
    fn literal_serde_round_trip_preserves_matching() {
        for text in ["a.b", "(", "$5 [urgent]", r"back\slash", "a|b", &"x.".repeat(120)] {
            let p = Pattern::literal(text).unwrap();
            let v = serde_json::to_value(&p).unwrap();
            assert_eq!(v, json!(regex::escape(text)), "serialized as the escaped regex");
            let back: Pattern = serde_json::from_value(v).unwrap();
            for probe in [text, "aXb", "a.b", "xx a.B yy", "a", "b"] {
                assert_eq!(p.is_match(probe), back.is_match(probe), "{text:?} vs {probe:?}");
            }
        }
    }

    #[test]
    fn addr_rule_serde_keeps_full_mode() {
        let r = AddrRule::regex(r".*@bank\.com").unwrap();
        let back: AddrRule = serde_json::from_value(serde_json::to_value(&r).unwrap()).unwrap();
        assert_eq!(back, r);
        assert!(back.matches("x@bank.com"));
        assert!(!back.matches("x@bank.com.evil.com"));
    }

    #[test]
    fn exact_rule_normalizes() {
        let r = AddrRule::exact(" Alice@Bank.COM ").unwrap();
        assert!(r.matches("alice@bank.com"));
        assert!(AddrRule::exact("Bob <bob@x.com>").is_err());
    }

    #[test]
    fn domain_rule_is_exact_domain() {
        let r = AddrRule::domain("@Bank.com").unwrap();
        assert!(r.matches("alice@bank.com"));
        assert!(!r.matches("alice@sub.bank.com"));
        assert!(!r.matches("alice@bank.com.evil.com"));
        assert!(!r.matches("alice@notbank.com"));
        assert!(AddrRule::domain("bank").is_err());
        assert!(AddrRule::domain("a@bank.com").is_err());
    }

    #[test]
    fn regex_rule_is_full_match() {
        let r = AddrRule::regex(r".*@bank\.com").unwrap();
        assert!(r.matches("alice@bank.com"));
        assert!(!r.matches("alice@bank.com.evil.com"));
        assert!(!AddrRule::regex("bank").unwrap().matches("notbank@x.com"));
    }

    #[test]
    fn unnormalized_input_never_matches() {
        let rules = [
            AddrRule::exact("x@bank.com").unwrap(),
            AddrRule::domain("bank.com").unwrap(),
            AddrRule::regex(r".*@bank\.com").unwrap(),
        ];
        let attacks = [
            "evil@evil.com, x@bank.com",
            "evil@evil.com\r\nBcc: x@bank.com",
            "\"Bank\" <evil@evil.com>, x@bank.com",
            "X@Bank.com",
            " x@bank.com",
            "x@bank.com ",
            "Bob <x@bank.com>",
        ];
        for rule in &rules {
            for attack in attacks {
                assert!(!rule.matches(attack), "{rule:?} matched unnormalized {attack:?}");
            }
            assert!(rule.matches("x@bank.com"), "{rule:?} must still match the normalized form");
        }
    }

    #[test]
    fn non_ascii_rules_rejected() {
        assert!(AddrRule::domain("\u{212A}ing.com").is_err());
        assert!(AddrRule::exact("\u{212A}@bank.com").is_err());
        assert!(serde_json::from_value::<AddrRule>(json!({"kind": "domain", "value": "\u{212A}ing.com"})).is_err());
        assert_eq!(AddrRule::domain("@Bank.COM").unwrap(), AddrRule::Domain("bank.com".to_owned()));
    }

    #[test]
    fn addr_rule_serde() {
        let r = AddrRule::domain("bank.com").unwrap();
        let v = serde_json::to_value(&r).unwrap();
        assert_eq!(v, json!({"kind": "domain", "value": "bank.com"}));
        assert_eq!(serde_json::from_value::<AddrRule>(v).unwrap(), r);
        assert!(serde_json::from_value::<AddrRule>(json!({"kind": "exact", "value": "not an address"})).is_err());
        assert!(serde_json::from_value::<AddrRule>(json!({"kind": "regex", "value": "a)|(.*"})).is_err());
    }
}
