//! How a connection looks in the lists: its name, its platform, the icon it shows when the user picked none, and
//! its blobatar.
//!
//! - **Name.** A computer is its own name ("M2A"), never "Reins desktop app on Mac.localdomain": see
//!   [`reins_proto::computer`]. Names the user typed are left as they are.
//! - **Automatic icon.** A computer shows its platform's logo (`macos`, `windows`, `linux`), an AI whose name says
//!   which one it is its provider's logo ("claude", "grok", ...), anything else its blobatar ([`BLOB`]).
//! - **Blobatar.** Each connection keeps a colour from [`COLOURS`], a shape and a seed for the figure's details
//!   ([`Blobatar`]). They are chosen when the connection first shows up, so that the user's connections differ: the
//!   colour least used among them, the farthest from those in use; the user can change them or shuffle. Kept in the
//!   synced account state, so every phone of the account draws the same one.

use reins_proto::computer::{self, Os};

/// The icon that means "draw the blobatar".
pub const BLOB: &str = "blob";

/// The blobatar's colours: OKLCH (lightness, chroma, hue), twelve hues around the wheel, each at the lightness where
/// it is most itself (a yellow is light, a blue deep) and every one inside sRGB. Two are at least 0.09 apart in OKLab
/// (`colours_are_far_apart`), and each reads on both the white and the dark plate.
pub const COLOURS: [(f64, f64, f64); 12] = [
    (0.63, 0.21, 27.0),   // red
    (0.73, 0.17, 58.0),   // orange
    (0.85, 0.165, 95.0),  // yellow
    (0.80, 0.19, 130.0),  // lime
    (0.64, 0.16, 150.0),  // green
    (0.71, 0.12, 186.0),  // teal
    (0.74, 0.13, 236.0),  // sky
    (0.57, 0.19, 262.0),  // blue
    (0.52, 0.22, 295.0),  // purple
    (0.70, 0.17, 318.0),  // violet
    (0.69, 0.20, 355.0),  // pink
    (0.56, 0.035, 255.0), // slate
];

/// The blobatar's shapes, in the library's order (its gen2 band table): `round`, `organic`, `boxy`, `capsule`, `nub`,
/// `cloud`, `droplet`, `hexagon`, `sun`, `triangle`. The apps draw shape `i` as the library's `i`th.
pub const SHAPES: [&str; 10] =
    ["round", "organic", "boxy", "capsule", "nub", "cloud", "droplet", "hexagon", "sun", "triangle"];

/// How many [`COLOURS`] and [`SHAPES`] there are, as the indexes kept count them.
const COLOUR_COUNT: u8 = 12;
const SHAPE_COUNT: u8 = 10;
const _: () = assert!(COLOURS.len() == COLOUR_COUNT as usize && SHAPES.len() == SHAPE_COUNT as usize);

/// `items[hash % len]`.
fn nth<T: Copy>(items: &[T], hash: u128) -> T {
    items[usize::try_from(hash % items.len() as u128).unwrap_or(0)]
}

/// Where the apps' copies of [`COLOURS`] and [`SHAPES`] are checked against (`the_shared_palette_is_this_one`):
/// Android's and iOS's own tests read the same file.
pub const PALETTE_FIXTURE: &str = "design/signal/tokens/blobatar-palette.json";

/// A connection's blobatar: the library's figure for `seed`, drawn in colour `colour` (an index of [`COLOURS`]) with
/// shape `shape` (the library's order, [`SHAPES`]).
#[derive(Clone, Debug, PartialEq, Eq, Hash, uniffi::Record)]
pub struct Blobatar {
    pub seed: String,
    pub shape: u8,
    pub colour: u8,
}

const VERSION: &str = "1";
const MAX_SEED_CHARS: usize = 40;

impl Blobatar {
    /// How it is kept: `1:<colour>:<shape>:<seed>`.
    #[must_use]
    pub fn encode(&self) -> String {
        format!("{VERSION}:{}:{}:{}", self.colour, self.shape, self.seed)
    }

    /// One kept by [`Self::encode`], if it is a valid one (a newer version's is not).
    #[must_use]
    pub fn decode(s: &str) -> Option<Self> {
        let mut parts = s.splitn(4, ':');
        if parts.next()? != VERSION {
            return None;
        }
        let colour: u8 = parts.next()?.parse().ok()?;
        let shape: u8 = parts.next()?.parse().ok()?;
        let seed = parts.next()?;
        Self::new(seed, shape, colour)
    }

    /// A blobatar, if its colour and shape exist and its seed is a short line.
    #[must_use]
    pub fn new(seed: &str, shape: u8, colour: u8) -> Option<Self> {
        let seed = crate::text::one_line(seed);
        (colour < COLOUR_COUNT && shape < SHAPE_COUNT && !seed.is_empty() && seed.chars().count() <= MAX_SEED_CHARS)
            .then_some(Self {
                seed,
                shape,
                colour,
            })
    }
}

/// The distance between two of [`COLOURS`] in OKLab (how different people see them).
#[must_use]
pub fn colour_distance(one: u8, other: u8) -> f64 {
    let lab = |index: u8| {
        let (lightness, chroma, hue) = COLOURS[usize::from(index) % COLOURS.len()];
        let angle = hue.to_radians();
        (lightness, chroma * angle.cos(), chroma * angle.sin())
    };
    let (p, q) = (lab(one), lab(other));
    ((p.0 - q.0).powi(2) + (p.1 - q.1).powi(2) + (p.2 - q.2).powi(2)).sqrt()
}

/// How many times `value` is in `used`.
fn uses(used: &[u8], value: u8) -> usize {
    used.iter().fold(0, |n, &u| n + usize::from(u == value))
}

/// A stable number for `key` (FNV-1a), to break ties the same way on every phone.
fn stable_hash(key: &str) -> u64 {
    key.bytes().fold(0xcbf2_9ce4_8422_2325, |h, b| (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3))
}

/// The colour for a new blobatar among `used` (the colours of the user's other connections): one used the fewest
/// times, of those the farthest from every colour in use, and among equals the one `key` (the connection's id)
/// hashes to.
#[must_use]
pub fn pick_colour(used: &[u8], key: &str) -> u8 {
    let count = |c: u8| uses(used, c);
    let fewest = (0..COLOUR_COUNT).map(count).min().unwrap_or(0);
    let candidates: Vec<u8> = (0..COLOUR_COUNT).filter(|&c| count(c) == fewest).collect();
    let nearest = |c: u8| used.iter().map(|&u| colour_distance(c, u)).fold(f64::INFINITY, f64::min);
    let best = candidates.iter().map(|&c| nearest(c)).fold(f64::NEG_INFINITY, f64::max);
    // Distances within a hair of the best count as equal, so that rounding never decides.
    let ties: Vec<u8> = candidates.into_iter().filter(|&c| nearest(c) >= best - 1e-9).collect();
    nth(&ties, u128::from(stable_hash(key)))
}

/// The shape for a new blobatar among `used`: one used the fewest times, among equals the one `key` hashes to.
#[must_use]
pub fn pick_shape(used: &[u8], key: &str) -> u8 {
    let count = |s: u8| uses(used, s);
    let fewest = (0..SHAPE_COUNT).map(count).min().unwrap_or(0);
    let ties: Vec<u8> = (0..SHAPE_COUNT).filter(|&s| count(s) == fewest).collect();
    nth(&ties, u128::from(stable_hash(&format!("shape:{key}"))))
}

/// The blobatar a new connection gets next to `others` (the user's other connections'): its id as the seed, a
/// colour and a shape picked to stand apart from theirs.
#[must_use]
pub fn first_blobatar(connection_id: &str, others: &[Blobatar]) -> Blobatar {
    let colours: Vec<u8> = others.iter().map(|b| b.colour).collect();
    let shapes: Vec<u8> = others.iter().map(|b| b.shape).collect();
    Blobatar {
        seed: crate::text::truncate_chars(connection_id, MAX_SEED_CHARS),
        shape: pick_shape(&shapes, connection_id),
        colour: pick_colour(&colours, connection_id),
    }
}

/// A new random blobatar for "Shuffle": a fresh seed, another shape, and another colour, one the user's other
/// connections (`others`) do not have when there is one. `random` is a fresh random number.
#[must_use]
pub fn shuffled(current: &Blobatar, others: &[Blobatar], random: u128) -> Blobatar {
    let seed = format!("{:08x}", random & 0xffff_ffff);
    let pick = |n: u8, avoid: &dyn Fn(u8) -> bool, salt: u32| -> u8 {
        let options: Vec<u8> = (0..n).filter(|&i| !avoid(i)).collect();
        if options.is_empty() {
            nth(&(0..n).collect::<Vec<u8>>(), random >> salt)
        } else {
            nth(&options, random >> salt)
        }
    };
    let taken: Vec<u8> = others.iter().map(|b| b.colour).collect();
    let free = pick(COLOUR_COUNT, &|c| c == current.colour || taken.contains(&c), 40);
    let colour = if free == current.colour {
        pick(COLOUR_COUNT, &|c| c == current.colour, 40)
    } else {
        free
    };
    let shape = pick(SHAPE_COUNT, &|s| s == current.shape, 80);
    Blobatar {
        seed,
        shape,
        colour,
    }
}

/// Gives every connection in `ids` (oldest first) a blobatar: the one it has in `kept`, else a new one standing apart
/// from those before it. Returns the new ones too, to be kept.
#[must_use]
pub fn assign<S: std::hash::BuildHasher>(
    ids: &[String],
    kept: &std::collections::HashMap<String, Blobatar, S>,
) -> Vec<(String, Blobatar, bool)> {
    let mut out: Vec<(String, Blobatar, bool)> = Vec::with_capacity(ids.len());
    // Those already kept count first: a newcomer stands apart from all of them, whatever their age.
    let mut taken: Vec<Blobatar> = ids.iter().filter_map(|id| kept.get(id).cloned()).collect();
    for id in ids {
        if let Some(b) = kept.get(id) {
            out.push((id.clone(), b.clone(), false));
        } else {
            let b = first_blobatar(id, &taken);
            taken.push(b.clone());
            out.push((id.clone(), b, true));
        }
    }
    out
}

/// A connection's name as the lists show it: one line, at most 64 characters, a computer's own name rather than
/// the older desktop app's "Reins desktop app on <hostname>" and without the platform the desktop app adds
/// ("M2A (macOS)" → "M2A"). Anything else (an AI app's name, a name the user typed) as it is.
#[must_use]
pub fn shown_label(label: &str) -> String {
    let line = crate::text::one_line(label);
    let (name, _) = computer::parse_client_name(&line);
    let name = if name.is_empty() {
        line
    } else {
        name
    };
    crate::text::truncate_chars(&name, 64)
}

/// A computer's platform, from the name it registered with (else its label): `macos`, `windows`, `linux`.
#[must_use]
pub fn platform(client_name: &str, label: &str) -> Option<Os> {
    computer::parse_client_name(&crate::text::one_line(client_name))
        .1
        .or_else(|| computer::parse_client_name(&crate::text::one_line(label)).1)
}

/// The AI providers whose logo the apps have, by the words in a connection's name that name them (the apps'
/// `Providers` table).
const PROVIDERS: &[(&str, &[&str])] = &[
    ("claude", &["claude", "anthropic"]),
    ("openai", &["chatgpt", "gpt", "openai", "codex"]),
    ("gemini", &["gemini", "bard"]),
    ("grok", &["grok", "xai"]),
    ("hermes", &["hermes"]),
    ("perplexity", &["perplexity"]),
    ("mistral", &["mistral", "le chat"]),
    ("deepseek", &["deepseek"]),
    ("copilot", &["copilot"]),
    ("cursor", &["cursor"]),
    ("qwen", &["qwen"]),
    ("kimi", &["kimi", "moonshot"]),
    ("meta", &["llama", "meta ai"]),
    ("ollama", &["ollama"]),
];

/// The provider a connection's name suggests ("My Claude" → `claude`).
#[must_use]
pub fn provider_of(label: &str) -> Option<&'static str> {
    let lower = label.to_lowercase();
    PROVIDERS.iter().find(|(_, words)| words.iter().any(|w| lower.contains(w))).map(|(key, _)| *key)
}

/// What a connection shows when the user picked no icon ("Auto"): a computer its platform's logo (its blobatar when
/// the platform is not known), an AI its provider's logo when its name says which, anything else its blobatar.
#[must_use]
pub fn auto_icon(label: &str, computer: bool, platform: Option<Os>) -> String {
    if computer {
        return platform.map_or(BLOB, Os::key).to_owned();
    }
    provider_of(label).unwrap_or(BLOB).to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn colours_are_far_apart_and_drawable() {
        for a in 0..COLOUR_COUNT {
            for b in (a + 1)..COLOUR_COUNT {
                assert!(colour_distance(a, b) >= 0.09, "{a} and {b} are {:.3} apart", colour_distance(a, b));
            }
        }
        // In sRGB (the apps would otherwise pull them in, and two could meet).
        for &(l, c, h) in &COLOURS {
            let (a, b) = (c * h.to_radians().cos(), c * h.to_radians().sin());
            let l_ = (l + 0.396_337_777_4 * a + 0.215_803_757_3 * b).powi(3);
            let m_ = (l - 0.105_561_345_8 * a - 0.063_854_172_8 * b).powi(3);
            let s_ = (l - 0.089_484_177_5 * a - 1.291_485_548 * b).powi(3);
            let rgb = [
                4.076_741_662_1 * l_ - 3.307_711_591_3 * m_ + 0.230_969_929_2 * s_,
                -1.268_438_004_6 * l_ + 2.609_757_401_1 * m_ - 0.341_319_396_5 * s_,
                -0.004_196_086_3 * l_ - 0.703_418_614_7 * m_ + 1.707_614_701 * s_,
            ];
            assert!(rgb.iter().all(|v| (-1e-3..=1.001).contains(v)), "{l} {c} {h}: {rgb:?}");
        }
    }

    /// The apps draw a kept colour and shape by index from their own copies; all three check them against one file.
    #[test]
    fn the_shared_palette_is_this_one() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").join(PALETTE_FIXTURE);
        let fixture: serde_json::Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        let colours: Vec<(f64, f64, f64)> = fixture["colours"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| (c["l"].as_f64().unwrap(), c["c"].as_f64().unwrap(), c["h"].as_f64().unwrap()))
            .collect();
        assert_eq!(colours, COLOURS);
        let shapes: Vec<&str> = fixture["shapes"].as_array().unwrap().iter().map(|s| s.as_str().unwrap()).collect();
        assert_eq!(shapes, SHAPES);
    }

    /// The property the user asked for: however many connections, and whatever their ids, they get different
    /// colours while there are colours left, the ones picked first far apart, and no colour twice before every one is
    /// used once.
    #[test]
    fn new_connections_get_distinct_well_separated_colours() {
        for run in 0..200u32 {
            let ids: Vec<String> =
                (0..30).map(|i| format!("conn-{run}-{}", stable_hash(&format!("{run}/{i}")))).collect();
            let all = assign(&ids, &HashMap::new());
            for n in 1..=ids.len() {
                let colours: Vec<u8> = all[..n].iter().map(|(_, b, _)| b.colour).collect();
                let mut counts = [0usize; COLOURS.len()];
                for &c in &colours {
                    counts[usize::from(c)] += 1;
                }
                let (lo, hi) = (counts.iter().min().unwrap(), counts.iter().max().unwrap());
                assert!(hi - lo <= 1, "run {run}, {n}: {counts:?}");
                if n <= COLOURS.len() {
                    assert!(*hi == 1, "run {run}: {n} connections share a colour: {colours:?}");
                }
                // The first few are spread widely: farthest-first, so the smallest gap shrinks slowly.
                let min_gap = colours
                    .iter()
                    .enumerate()
                    .flat_map(|(i, &a)| colours[i + 1..].iter().map(move |&b| colour_distance(a, b)))
                    .fold(f64::INFINITY, f64::min);
                let floor = match n {
                    2 => 0.25,
                    3 | 4 => 0.18,
                    5..=8 => 0.12,
                    9..=12 => 0.09,
                    _ => 0.0,
                };
                assert!(min_gap >= floor, "run {run}, {n} connections: closest {min_gap:.3} < {floor}");
            }
            let shapes: Vec<u8> = all[..SHAPES.len()].iter().map(|(_, b, _)| b.shape).collect();
            let mut unique = shapes.clone();
            unique.sort_unstable();
            unique.dedup();
            assert_eq!(unique.len(), SHAPES.len(), "the first ten have ten shapes");
        }
    }

    #[test]
    fn a_new_connection_stands_apart_from_the_ones_kept() {
        // The user's two computers came out nearly the same green; a third must not be green too.
        let mut kept = HashMap::new();
        kept.insert("mac".to_owned(), Blobatar::new("mac", 0, 4).unwrap());
        kept.insert("pc".to_owned(), Blobatar::new("pc", 1, 3).unwrap());
        let ids = ["mac", "pc", "grok"].map(str::to_owned);
        let all = assign(&ids, &kept);
        assert_eq!((all[0].1.colour, all[0].2), (4, false), "kept as it is");
        assert_eq!((all[1].1.colour, all[1].2), (3, false));
        let grok = &all[2].1;
        assert!(all[2].2, "new, to be kept");
        assert!(colour_distance(grok.colour, 4) >= 0.2 && colour_distance(grok.colour, 3) >= 0.2, "{grok:?}");
        assert_eq!(grok.seed, "grok");
        // Every phone works it out the same way.
        assert_eq!(assign(&ids, &kept), all);
    }

    #[test]
    fn a_blobatar_is_kept_as_a_short_string() {
        let b = Blobatar::new("c-1", 7, 11).unwrap();
        assert_eq!(b.encode(), "1:11:7:c-1");
        assert_eq!(Blobatar::decode(&b.encode()), Some(b));
        assert_eq!(Blobatar::decode("1:3:2:seed:with:colons").unwrap().seed, "seed:with:colons");
        for bad in ["", "2:1:1:x", "1:12:0:x", "1:0:10:x", "1:0:0:", "1:a:0:x", &format!("1:0:0:{}", "x".repeat(41))] {
            assert_eq!(Blobatar::decode(bad), None, "{bad}");
        }
    }

    #[test]
    fn shuffle_changes_the_seed_the_shape_and_the_colour() {
        let current = Blobatar::new("c-1", 2, 5).unwrap();
        let others = [Blobatar::new("x", 0, 0).unwrap(), Blobatar::new("y", 1, 1).unwrap()];
        for r in 0..500u128 {
            let random = r.wrapping_mul(0x9e37_79b9_7f4a_7c15_f39c_c060_5ced_c834) ^ (r << 64);
            let s = shuffled(&current, &others, random);
            assert!(Blobatar::new(&s.seed, s.shape, s.colour).is_some());
            assert_ne!(s.shape, current.shape);
            assert_ne!(s.colour, current.colour);
            assert!(s.colour > 1, "not another connection's colour while one is free: {s:?}");
        }
        // With every colour taken, it still changes.
        let all: Vec<Blobatar> = (0..12).map(|c| Blobatar::new("o", 0, c).unwrap()).collect();
        assert_ne!(shuffled(&current, &all, 12345).colour, current.colour);
    }

    #[test]
    fn a_computer_shows_its_own_name() {
        assert_eq!(shown_label("Reins desktop app on Mac.localdomain"), "Mac");
        assert_eq!(shown_label("Reins desktop app on honeypot"), "honeypot");
        assert_eq!(shown_label("M2A (macOS)"), "M2A");
        assert_eq!(shown_label("honeypot (Linux)"), "honeypot");
        assert_eq!(shown_label("Claude"), "Claude");
        assert_eq!(shown_label("Grok"), "Grok");
        assert_eq!(shown_label("  My\nwork laptop "), "My work laptop", "a name the user typed stays");
        assert_eq!(shown_label(&"y".repeat(100)).chars().count(), 64);
    }

    #[test]
    fn the_platform_comes_from_the_registered_name() {
        assert_eq!(platform("M2A (macOS)", "M2A"), Some(Os::MacOs));
        assert_eq!(platform("honeypot (Linux)", "Work box"), Some(Os::Linux), "renamed by hand, still Linux");
        assert_eq!(platform("DESKTOP-1 (Windows)", "DESKTOP-1"), Some(Os::Windows));
        assert_eq!(
            platform("Reins desktop app on Mac.localdomain", "Reins desktop app on Mac.localdomain"),
            Some(Os::MacOs)
        );
        assert_eq!(platform("Reins desktop app on honeypot", "Reins desktop app on honeypot"), None);
        assert_eq!(platform("Claude", "Claude"), None);
    }

    #[test]
    fn the_automatic_icon() {
        assert_eq!(auto_icon("M2A", true, Some(Os::MacOs)), "macos");
        assert_eq!(auto_icon("DESKTOP-1", true, Some(Os::Windows)), "windows");
        assert_eq!(auto_icon("honeypot", true, Some(Os::Linux)), "linux");
        assert_eq!(auto_icon("honeypot", true, None), BLOB, "a computer of unknown platform: its blobatar");
        assert_eq!(auto_icon("Claude Code on my claude box", true, None), BLOB, "a computer is never an AI's logo");
        assert_eq!(auto_icon("Claude", false, None), "claude");
        assert_eq!(auto_icon("ChatGPT", false, None), "openai");
        assert_eq!(auto_icon("Grok", false, None), "grok");
        assert_eq!(auto_icon("Cursor", false, None), "cursor");
        assert_eq!(auto_icon("Le Chat", false, None), "mistral");
        assert_eq!(auto_icon("Raycast", false, None), BLOB);
    }
}
