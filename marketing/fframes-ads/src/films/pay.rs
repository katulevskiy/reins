//! Reins Pay, 20 seconds: the agent shops, the phone shows the exact cart, total, address and
//! card, Face ID approves it, the agent checks out with a one-time card for that cart only. When
//! the cart changes, the card stops working and the phone asks again.
//!
//! Kept apart from the other films so it can change with the feature: the status line on the end
//! card is `STATUS`, the cart and prices are in `ITEMS` (and in the phone's `pay_preview`).

use fframes::{AudioTrack, FFramesContext, Frame, Svgr};

use super::{music, sfx, vo};
use crate::kit::*;
use crate::ui::phone::{Kind, Phone, Req, phone};
use crate::ui::{Tone, Word, Words, backdrop, end_card, flash, words};

/// The line under the logo at the end. Change to "New in Reins." when Reins Pay ships.
pub const STATUS: &str = "Reins Pay. Coming soon.";

pub const BEATS: &[(&str, f32)] = &[("Shop", 2.2), ("Approve", 6.25), ("Result", 3.55), ("Change", 4.5), ("End", 3.5)];

const CHECKOUT: f32 = 2.2;
const CHANGE: f32 = 13.4;
const DECLINED: f32 = 14.0;
const OUT: f32 = 16.2;
const END: f32 = 16.5;
const TOKEN_ROW: f32 = 10.9;

const REQS: [Req; 2] = [
    Req {
        kind: Kind::Pay,
        buzz: 2.7,
        up: 3.05,
        tap: 7.3,
        approve: true,
    },
    Req {
        kind: Kind::PayAgain,
        buzz: 14.35,
        up: 99.0,
        tap: 99.0,
        approve: false,
    },
];

/// (name, quantity, price, appears at)
const ITEMS: [(&str, &str, &str, f32); 3] = [
    ("Single-origin beans, 1 kg", "×2", "$34.00", 0.35),
    ("USB-C cable, 2 m", "×1", "$9.99", 0.7),
    ("Espresso machine", "×1", "$465.22", CHANGE),
];

const OPEN: &[Word] = &[
    ("Your", -0.3, Tone::Text),
    ("agent", -0.2, Tone::Text),
    ("can", -0.1, Tone::Text),
    ("shop", 0.0, Tone::Accent),
    ("now.", 0.1, Tone::Accent),
];
const C1: &[Word] = &[
    ("Your", 3.2, Tone::Text),
    ("phone", 3.32, Tone::Text),
    ("shows", 3.44, Tone::Text),
    ("the", 3.56, Tone::Text),
    ("exact", 3.68, Tone::Accent),
    ("cart.", 3.8, Tone::Accent),
];
const C2: &[Word] = &[
    ("See", 4.77, Tone::Text),
    ("exactly", 4.93, Tone::Accent),
    ("what", 5.65, Tone::Text),
    ("will", 5.81, Tone::Text),
    ("happen.", 5.97, Tone::Text),
];
const C3: &[Word] = &[
    ("Approve", 7.1, Tone::Text),
    ("with", 7.22, Tone::Text),
    ("your", 7.34, Tone::Text),
    ("face.", 7.46, Tone::Success),
];
const C4: &[Word] = &[
    ("Agents", 9.53, Tone::Text),
    ("get", 9.93, Tone::Text),
    ("the", 10.05, Tone::Text),
    ("result.", 10.45, Tone::Success),
    ("/", 0.0, Tone::Text),
    ("Never", 10.97, Tone::Accent),
    ("the", 11.21, Tone::Accent),
    ("key.", 11.37, Tone::Accent),
];
const C5: &[Word] = &[
    ("The", 12.0, Tone::Text),
    ("card", 12.1, Tone::Text),
    ("only", 12.2, Tone::Text),
    ("works", 12.3, Tone::Text),
    ("for", 12.4, Tone::Text),
    ("/", 0.0, Tone::Text),
    ("the", 12.5, Tone::Text),
    ("exact", 12.6, Tone::Accent),
    ("cart", 12.7, Tone::Accent),
    ("you", 12.8, Tone::Text),
    ("approved.", 12.9, Tone::Text),
];
const C6: &[Word] = &[
    ("Cart", 14.0, Tone::Text),
    ("changed?", 14.12, Tone::Danger),
    ("/", 0.0, Tone::Text),
    ("It", 14.4, Tone::Text),
    ("asks", 14.5, Tone::Text),
    ("again.", 14.6, Tone::Text),
];

pub fn audio() -> Vec<AudioTrack<'static>> {
    vec![
        music("music_pay.wav", -11.0),
        vo("vo09.mp3", 4.4),
        vo("vo15.mp3", 9.4),
        vo("vo21.mp3", END + 0.3),
        sfx("sfx_pop.wav", ITEMS[0].3, -10.0),
        sfx("sfx_pop.wav", ITEMS[1].3, -10.0),
        sfx("sfx_tap.wav", CHECKOUT, -8.0),
        sfx("sfx_buzz.wav", REQS[0].buzz, -9.0),
        sfx("sfx_notify.wav", REQS[0].buzz, -12.0),
        sfx("sfx_swish.wav", REQS[0].up - 0.1, -15.0),
        sfx("sfx_tap.wav", REQS[0].tap, -7.0),
        sfx("sfx_scan.wav", REQS[0].tap + 0.15, -11.0),
        sfx("sfx_chime.wav", REQS[0].done(), -8.0),
        sfx("sfx_whoosh.wav", 9.0, -15.0),
        sfx("sfx_lock.wav", TOKEN_ROW, -10.0),
        sfx("sfx_pop.wav", CHANGE, -9.0),
        sfx("sfx_stamp.wav", DECLINED, -8.0),
        sfx("sfx_deny.wav", DECLINED + 0.05, -11.0),
        sfx("sfx_buzz.wav", REQS[1].buzz, -9.0),
        sfx("sfx_notify.wav", REQS[1].buzz, -12.0),
        sfx("sfx_whooshLow.wav", OUT, -11.0),
        sfx("sfx_boom.wav", END, -8.0),
        sfx("sfx_shimmer.wav", END + 0.05, -12.0),
    ]
}

pub fn draw<F: Format>(frame: &mut Frame, ctx: &FFramesContext, t: f32) -> Svgr<'static> {
    let (w, h) = (F::W as f32, F::H as f32);
    let mut l = vec![backdrop(t, w, h, 0.25 * life(t, DECLINED, DECLINED + 0.8, M::Soft), 0.3)];
    let leave = ease(t, OUT, 0.35, E::In);

    if leave < 1.0 {
        if F::TALL {
            // The phone comes up for the payment, leaves for the result, comes back when asked again.
            let on = sp(t, 2.35, M::Heavy) - ease(t, 9.0, 0.4, E::In) + sp(t, 14.05, M::Heavy);
            let compact = clamp01(sp(t, 2.25, M::Snappy) - sp(t, 9.05, M::Snappy) + sp(t, 13.95, M::Snappy));
            let y = lerp(-20.0, -500.0, compact);
            l.push(group(tr(0.0, -leave * 80.0), 1.0 - leave, vec![cart(frame, ctx, t, 0.0, y, 960.0, compact)]));
            if on > 0.001 {
                l.push(phone(
                    frame,
                    ctx,
                    t,
                    &Phone {
                        x: 0.0,
                        y: lerp(1420.0, 170.0, on) + leave * 900.0,
                        height: 1040.0,
                        reqs: &REQS,
                        keys_at: None,
                        wake: 1.0,
                    },
                ));
            }
        } else {
            l.push(group(tr(-leave * 120.0, 0.0), 1.0 - leave, vec![cart(frame, ctx, t, -420.0, 150.0, 860.0, 0.0)]));
            let p = sp(t, -0.4, M::Heavy);
            l.push(phone(
                frame,
                ctx,
                t,
                &Phone {
                    x: lerp(1500.0, 480.0, p) + leave * 700.0,
                    y: 0.0,
                    height: 940.0,
                    reqs: &REQS,
                    keys_at: None,
                    wake: 1.0,
                },
            ));
        }
    }

    let (cx, cy, cs, cw) = if F::TALL {
        (0.0, -730.0, 76.0, 960.0)
    } else {
        (-420.0, -370.0, 64.0, 900.0)
    };
    let (ox, oy, os) = if F::TALL {
        (0.0, -560.0, 104.0)
    } else {
        (-420.0, -340.0, 84.0)
    };
    l.push(
        words(
            frame,
            ctx,
            t,
            &Words {
                words: OPEN,
                size: os,
                x: ox,
                y: oy,
                max_width: cw,
                exit: 3.0,
                ..Default::default()
            },
        )
        .0,
    );
    for (ws, exit) in [(C1, 4.45), (C2, 6.95), (C3, 9.0), (C4, 11.9), (C5, 13.95), (C6, OUT)] {
        l.push(
            words(
                frame,
                ctx,
                t,
                &Words {
                    words: ws,
                    size: cs,
                    x: cx,
                    y: cy,
                    max_width: cw,
                    exit,
                    ..Default::default()
                },
            )
            .0,
        );
    }
    l.push(flash(t, REQS[0].done(), 0.4, c::SUCCESS, 0.12, w, h));
    if t >= END - 0.1 {
        l.push(end_card(
            frame,
            ctx,
            t,
            END,
            if F::TALL {
                -120.0
            } else {
                -90.0
            },
            F::TALL,
            STATUS,
        ));
    }
    fframes::svgr!(<g>{l}</g>)
}

/// The agent's side: a checkout page. `compact` (0..1) folds it to the header, total and button.
fn cart(frame: &mut Frame, ctx: &FFramesContext, t: f32, x: f32, y: f32, w: f32, compact: f32) -> Svgr<'static> {
    let open = 1.0 - compact;
    let fs = 34.0;
    let row_h = 74.0;
    let head_h = 92.0;
    let rows_h: f32 = ITEMS.iter().map(|i| clamp01(sp(t, i.3, M::Snappy)) * row_h).sum::<f32>() * open;
    let token_h = clamp01(sp(t, TOKEN_ROW, M::Snappy)) * 96.0 * open;
    let h = head_h + rows_h + 96.0 + 110.0 + token_h + 20.0;
    let (left, top) = (x - w / 2.0, y - h / 2.0);
    let pad = 40.0;
    let mut parts = vec![
        rect_o(left, top + 24.0, w, h, 26.0, "#000000", 0.5),
        rect(left, top, w, h, 26.0, "#0E0E11"),
        stroke_rect(left, top, w, h, 26.0, "#FFFFFF", 1.5, 0.09),
        icon_tile(Icon::Bot, left + pad + 26.0, top + head_h / 2.0, 52.0, c::LILAC),
        text(left + pad + 70.0, top + head_h / 2.0 + 11.0, "Shopping agent", SANS, 30.0, 600, c::TEXT, "start"),
        text(left + w - pad, top + head_h / 2.0 + 10.0, "lumen.supply/checkout", MONO, 24.0, 400, c::TERTIARY, "end"),
        rect(left, top + head_h, w, 1.5, 0.0, c::HAIRLINE),
    ];
    // Items.
    let mut ry = top + head_h;
    for item in ITEMS {
        let k = clamp01(sp(t, item.3, M::Snappy)) * open;
        if k <= 0.001 {
            continue;
        }
        let pop = sp(t, item.3, M::Pop);
        let new = item.3 >= CHANGE;
        let col = if new {
            c::DANGER
        } else {
            c::TEXT
        };
        parts.push(group(
            trs(x, ry + row_h * 0.5 * k, lerp(0.9, 1.0, pop)),
            clamp01(pop * 1.5) * k,
            vec![
                icon_tile(
                    Icon::Cart,
                    -w / 2.0 + pad + 24.0,
                    0.0,
                    48.0,
                    if new {
                        c::DANGER
                    } else {
                        c::ACCENT
                    },
                ),
                text(-w / 2.0 + pad + 70.0, fs * 0.35, item.0, SANS, fs, 500, col, "start"),
                text(w / 2.0 - pad - 170.0, fs * 0.35, item.1, SANS, fs * 0.9, 500, c::TERTIARY, "end"),
                text(w / 2.0 - pad, fs * 0.35, item.2, MONO, fs * 0.9, 500, col, "end"),
            ],
        ));
        ry += row_h * k;
    }
    // Total, counting up as items land.
    let total = lerp(0.0, 47.18, ease(t, 0.3, 0.9, E::OutExpo)) + 465.22 * ease(t, CHANGE, 0.6, E::OutExpo);
    let ty = ry + 62.0;
    parts.push(rect(left + pad, ry + 8.0, w - pad * 2.0, 1.5, 0.0, c::HAIRLINE));
    parts.push(text(left + pad, ty, "Total", SANS, 40.0, 700, c::TEXT, "start"));
    parts.push(text(
        left + w - pad,
        ty,
        format!("${total:.2}"),
        MONO,
        40.0,
        700,
        if t >= CHANGE {
            c::DANGER
        } else {
            c::TEXT
        },
        "end",
    ));
    // The button: checkout, waiting, placed, declined, asking again.
    let by = ty + 40.0;
    let bh = 84.0;
    let press = sp(t, CHECKOUT, M::Pop) - sp(t, CHECKOUT + 0.12, M::Snappy);
    let done = REQS[0].done();
    let (bg, fg, label, ic, spin) = if t < CHECKOUT + 0.1 {
        (c::ACCENT, "#FFFFFF", "Checkout · $47.18".to_owned(), Some(Icon::Card), false)
    } else if t < done {
        ("#2A2410", c::WARNING, "Waiting for your phone".to_owned(), None, true)
    } else if t < DECLINED {
        ("#0F2A20", c::SUCCESS, "Order placed · #A-4821".to_owned(), Some(Icon::Check), false)
    } else if t < REQS[1].buzz + 0.2 {
        ("#2A1010", c::DANGER, "Card declined: cart changed".to_owned(), Some(Icon::X), false)
    } else {
        ("#2A2410", c::WARNING, "Asking your phone again".to_owned(), None, true)
    };
    let bx = left + pad;
    let bw = w - pad * 2.0;
    let lw = measure(frame, ctx, SANS, 600, 32.0, &label);
    let ix = x - lw / 2.0 - 24.0;
    let icon_svg = match (ic, spin) {
        (Some(i), _) => icon(i, ix, by + bh / 2.0, 30.0, fg, 2.6),
        (None, true) => {
            let r = 13.0;
            let a = t * 540.0;
            let cy = by + bh / 2.0;
            fframes::svgr!(<g transform={format!("rotate({a:.1} {ix:.2} {cy:.2})")}>
                <circle cx={ix} cy={cy} r={r} fill="none" stroke={fg.to_owned()} stroke-opacity="0.25" stroke-width="4" />
                <path d={format!("M{:.2} {cy:.2} a{r} {r} 0 0 1 {:.2} {:.2}", ix + r, -r, -r)} fill="none" stroke={fg.to_owned()} stroke-width="4" stroke-linecap="round" />
            </g>)
        }
        _ => Svgr::empty(),
    };
    parts.push(group(
        format!(
            "translate({x:.2} {:.2}) scale({:.4}) translate({:.2} {:.2})",
            by + bh / 2.0,
            1.0 - press * 0.05,
            -x,
            -(by + bh / 2.0)
        ),
        1.0,
        vec![
            rect(bx, by, bw, bh, bh / 2.0, bg),
            icon_svg,
            text(x + 18.0, by + bh / 2.0 + 11.0, label, SANS, 32.0, 600, fg, "middle"),
        ],
    ));
    // What the agent holds instead of a card number.
    if token_h > 1.0 {
        let k = clamp01(sp(t, TOKEN_ROW, M::Snappy));
        let yy = by + bh + 58.0;
        parts.push(group(
            tr(0.0, (1.0 - k) * 20.0),
            k * open,
            vec![
                icon(Icon::Card, left + pad + 18.0, yy - 10.0, 30.0, c::SECONDARY, 2.2),
                text(left + pad + 50.0, yy, "Card the agent sees:", SANS, 28.0, 500, c::SECONDARY, "start"),
                text(left + w - pad, yy, "•••• •••• •••• ••••", MONO, 28.0, 600, c::LILAC, "end"),
            ],
        ));
    }
    fframes::svgr!(<g>{parts}</g>)
}
