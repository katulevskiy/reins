//! Reins Signal colour tokens for the GPUI desktop app. Generated from signal.tokens.json.
//! Each entry is (light_rgb, light_alpha, dark_rgb, dark_alpha).
//!
//! A copy of `design/signal/tokens/desktop/signal_tokens.rs` (kept unformatted, so the two stay comparable; a test
//! checks they agree). [`super::Palette`] turns them into colours.
#![allow(clippy::unreadable_literal, reason = "as generated")]

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Token { pub light: u32, pub light_a: f32, pub dark: u32, pub dark_a: f32 }

pub const PAPER: Token = Token { light: 0xF6F6F8, light_a: 1.0, dark: 0x121215, dark_a: 1.0 };
pub const PAPER_RAISED: Token = Token { light: 0xFFFFFF, light_a: 1.0, dark: 0x1C1C21, dark_a: 1.0 };
pub const PAPER_SUNKEN: Token = Token { light: 0xECECF0, light_a: 1.0, dark: 0x26262C, dark_a: 1.0 };
pub const INK: Token = Token { light: 0x16151B, light_a: 1.0, dark: 0xEDEDF0, dark_a: 1.0 };
pub const INK_2: Token = Token { light: 0x5F5E6B, light_a: 1.0, dark: 0xA6A6B0, dark_a: 1.0 };
pub const INK_META: Token = Token { light: 0x6B6A78, light_a: 1.0, dark: 0x8E8E99, dark_a: 1.0 };
pub const INK_FAINT: Token = Token { light: 0xA3A2AD, light_a: 1.0, dark: 0x63636E, dark_a: 1.0 };
pub const RULE: Token = Token { light: 0xE3E3EA, light_a: 1.0, dark: 0x2A2A30, dark_a: 1.0 };
pub const INVERSE: Token = Token { light: 0x16151B, light_a: 1.0, dark: 0xEDEDF0, dark_a: 1.0 };
pub const ON_INVERSE: Token = Token { light: 0xFFFFFF, light_a: 1.0, dark: 0x121215, dark_a: 1.0 };
pub const VIOLET: Token = Token { light: 0x5B43E8, light_a: 1.0, dark: 0xA196FF, dark_a: 1.0 };
pub const VIOLET_FILL: Token = Token { light: 0x5B43E8, light_a: 1.0, dark: 0x5B43E8, dark_a: 1.0 };
pub const SUCCESS: Token = Token { light: 0x15803D, light_a: 1.0, dark: 0x34D399, dark_a: 1.0 };
pub const DANGER: Token = Token { light: 0xB91C1C, light_a: 1.0, dark: 0xF87171, dark_a: 1.0 };
pub const DANGER_WASH: Token = Token { light: 0xFBE3E3, light_a: 1.0, dark: 0xF87171, dark_a: 0.14 };
pub const DANGER_FILL_2: Token = Token { light: 0xF6C9C9, light_a: 1.0, dark: 0xF87171, dark_a: 0.28 };
pub const WARNING: Token = Token { light: 0xA16207, light_a: 1.0, dark: 0xFACC15, dark_a: 1.0 };
pub const SCRIM: Token = Token { light: 0x16151B, light_a: 0.32, dark: 0x000000, dark_a: 0.55 };
pub const FIELD_PUSH: Token = Token { light: 0xC2410C, light_a: 1.0, dark: 0xC2410C, dark_a: 1.0 };
pub const FIELD_READ: Token = Token { light: 0x2563EB, light_a: 1.0, dark: 0x2563EB, dark_a: 1.0 };
pub const FIELD_DELETE: Token = Token { light: 0xB91C1C, light_a: 1.0, dark: 0xB91C1C, dark_a: 1.0 };
pub const FIELD_GRANT: Token = Token { light: 0x5B43E8, light_a: 1.0, dark: 0x5B43E8, dark_a: 1.0 };
pub const FIELD_PAIR: Token = Token { light: 0x0E7490, light_a: 1.0, dark: 0x0E7490, dark_a: 1.0 };
pub const FIELD_ACCOUNTS: Token = Token { light: 0xBE185D, light_a: 1.0, dark: 0xBE185D, dark_a: 1.0 };
pub const FIELD_WARN: Token = Token { light: 0xA16207, light_a: 1.0, dark: 0xA16207, dark_a: 1.0 };
pub const ON_FIELD: Token = Token { light: 0xFFFFFF, light_a: 1.0, dark: 0xFFFFFF, dark_a: 1.0 };
pub const ON_FIELD_2: Token = Token { light: 0xFFFFFF, light_a: 0.82, dark: 0xFFFFFF, dark_a: 0.82 };
pub const ON_FIELD_WASH: Token = Token { light: 0xFFFFFF, light_a: 0.18, dark: 0xFFFFFF, dark_a: 0.18 };
pub const RULE_2: Token = Token { light: 0xD3D3DC, light_a: 1.0, dark: 0x3A3A42, dark_a: 1.0 };
pub const KIND_PUSH: Token = Token { light: 0xC2410C, light_a: 1.0, dark: 0xFB923C, dark_a: 1.0 };
pub const KIND_READ: Token = Token { light: 0x2563EB, light_a: 1.0, dark: 0x60A5FA, dark_a: 1.0 };
pub const KIND_DELETE: Token = Token { light: 0xB91C1C, light_a: 1.0, dark: 0xF87171, dark_a: 1.0 };
pub const KIND_GRANT: Token = Token { light: 0x5B43E8, light_a: 1.0, dark: 0xA196FF, dark_a: 1.0 };
pub const KIND_PAIR: Token = Token { light: 0x0E7490, light_a: 1.0, dark: 0x22D3EE, dark_a: 1.0 };
pub const KIND_ACCOUNTS: Token = Token { light: 0xBE185D, light_a: 1.0, dark: 0xF472B6, dark_a: 1.0 };
pub const KIND_WARN: Token = Token { light: 0xA16207, light_a: 1.0, dark: 0xFACC15, dark_a: 1.0 };

/// The tokens by name, as the design names them (`paper-raised`), for the gallery and the tests.
pub const ALL: [(&str, Token); 36] = [
    ("paper", PAPER), ("paper-raised", PAPER_RAISED), ("paper-sunken", PAPER_SUNKEN), ("ink", INK), ("ink-2", INK_2),
    ("ink-meta", INK_META), ("ink-faint", INK_FAINT), ("rule", RULE), ("inverse", INVERSE), ("on-inverse", ON_INVERSE),
    ("violet", VIOLET), ("violet-fill", VIOLET_FILL), ("success", SUCCESS), ("danger", DANGER),
    ("danger-wash", DANGER_WASH), ("danger-fill-2", DANGER_FILL_2), ("warning", WARNING), ("scrim", SCRIM),
    ("field-push", FIELD_PUSH), ("field-read", FIELD_READ), ("field-delete", FIELD_DELETE),
    ("field-grant", FIELD_GRANT), ("field-pair", FIELD_PAIR), ("field-accounts", FIELD_ACCOUNTS),
    ("field-warn", FIELD_WARN), ("on-field", ON_FIELD), ("on-field-2", ON_FIELD_2), ("on-field-wash", ON_FIELD_WASH),
    ("rule-2", RULE_2), ("kind-push", KIND_PUSH), ("kind-read", KIND_READ), ("kind-delete", KIND_DELETE),
    ("kind-grant", KIND_GRANT), ("kind-pair", KIND_PAIR), ("kind-accounts", KIND_ACCOUNTS), ("kind-warn", KIND_WARN),
];

/// The generated file itself, to check this copy against.
#[cfg(test)]
#[allow(dead_code, unreachable_pub, reason = "the generated file, as it is")]
#[path = "../../../../design/signal/tokens/desktop/signal_tokens.rs"]
mod design;

#[cfg(test)]
mod tests {
    use super::*;

    /// Every token here is the generated one, in value and in name.
    #[test]
    fn the_tokens_equal_the_generated_file() {
        macro_rules! same {
            ($($name:ident),+ $(,)?) => {{
                $(
                    let (a, b) = ($name, design::$name);
                    assert_eq!(
                        (a.light, a.light_a.to_bits(), a.dark, a.dark_a.to_bits()),
                        (b.light, b.light_a.to_bits(), b.dark, b.dark_a.to_bits()),
                        stringify!($name)
                    );
                )+
                [$(stringify!($name)),+].len()
            }};
        }
        let checked = same!(
            PAPER, PAPER_RAISED, PAPER_SUNKEN, INK, INK_2, INK_META, INK_FAINT, RULE, INVERSE, ON_INVERSE, VIOLET,
            VIOLET_FILL, SUCCESS, DANGER, DANGER_WASH, DANGER_FILL_2, WARNING, SCRIM, FIELD_PUSH, FIELD_READ,
            FIELD_DELETE, FIELD_GRANT, FIELD_PAIR, FIELD_ACCOUNTS, FIELD_WARN, ON_FIELD, ON_FIELD_2, ON_FIELD_WASH,
            RULE_2, KIND_PUSH, KIND_READ, KIND_DELETE, KIND_GRANT, KIND_PAIR, KIND_ACCOUNTS, KIND_WARN,
        );
        let generated = include_str!("../../../../design/signal/tokens/desktop/signal_tokens.rs");
        assert_eq!(checked, generated.matches("pub const ").count(), "a generated token is missing here");
        assert_eq!(ALL.len(), checked);
        // ALL names each constant in the design's spelling.
        for (name, token) in ALL {
            let constant = name.to_ascii_uppercase().replace('-', "_");
            let line = generated.lines().find(|l| l.starts_with(&format!("pub const {constant}: "))).unwrap();
            assert!(line.contains(&format!("light: 0x{:06X},", token.light)), "{name}");
            assert!(line.contains(&format!("dark: 0x{:06X},", token.dark)), "{name}");
        }
    }
}
