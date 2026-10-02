//! Answering a refused push ourselves, the way receive-pack reports a rejected ref: git then prints
//! `! [remote rejected] main -> main (reason)` and, from side-band 2, `remote: <message>`.

use crate::git::ReceiveHead;
use crate::git::pktline::{FLUSH, encode, sideband};

pub const CONTENT_TYPE: &str = "application/x-git-receive-pack-result";

/// One printable line of at most `max` characters.
fn one_line(text: &str, max: usize) -> String {
    let line: String = text
        .chars()
        .map(|c| {
            if c.is_control() {
                ' '
            } else {
                c
            }
        })
        .collect();
    let line = line.split_whitespace().collect::<Vec<_>>().join(" ");
    if line.chars().count() <= max {
        line
    } else {
        let mut cut: String = line.chars().take(max.saturating_sub(3)).collect();
        cut.push_str("...");
        cut
    }
}

/// The receive-pack result rejecting every command in `head`: `reason` on each `ng` line, `message` on side-band 2
/// when the client asked for a side-band.
#[must_use]
pub fn rejection(head: &ReceiveHead, reason: &str, message: &str) -> Vec<u8> {
    let reason = one_line(reason, 120);
    let mut report = encode(b"unpack ok\n");
    for c in &head.commands {
        report.extend(encode(format!("ng {} {reason}\n", c.name).as_bytes()));
    }
    report.extend_from_slice(FLUSH);
    if !(head.has_capability("side-band-64k") || head.has_capability("side-band")) {
        return report;
    }
    let mut out = Vec::new();
    for line in message.lines().filter(|l| !l.trim().is_empty()) {
        out.extend(sideband(2, format!("{}\n", one_line(line, 900)).as_bytes()));
    }
    if head.has_capability("side-band-64k") {
        out.extend(sideband(1, &report));
    } else {
        // Plain side-band carries at most 1000 bytes a packet.
        for chunk in report.chunks(995) {
            out.extend(sideband(1, chunk));
        }
    }
    out.extend_from_slice(FLUSH);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::parse_receive_head;
    use crate::git::pktline::{Pkt, Reader};

    const A: &str = "1111111111111111111111111111111111111111";
    const B: &str = "2222222222222222222222222222222222222222";

    fn head(caps: &str) -> ReceiveHead {
        let mut body = encode(format!("{A} {B} refs/heads/main\0{caps}\n").as_bytes());
        body.extend(encode(format!("{A} {B} refs/heads/dev\n").as_bytes()));
        body.extend_from_slice(FLUSH);
        parse_receive_head(&body).unwrap()
    }

    fn data(buf: &[u8]) -> Vec<Vec<u8>> {
        let mut r = Reader::new(buf);
        let mut out = Vec::new();
        while let Some(p) = r.next_pkt().unwrap() {
            match p {
                Pkt::Data(d) => out.push(d.to_vec()),
                _ => out.push(b"<flush>".to_vec()),
            }
        }
        out
    }

    #[test]
    fn without_side_band_the_report_is_plain() {
        let out = rejection(&head("report-status"), "denied\non phone", "ignored");
        assert_eq!(
            data(&out),
            vec![
                b"unpack ok\n".to_vec(),
                b"ng refs/heads/main denied on phone\n".to_vec(),
                b"ng refs/heads/dev denied on phone\n".to_vec(),
                b"<flush>".to_vec()
            ]
        );
    }

    #[test]
    fn with_side_band_the_message_comes_first_and_the_report_on_band_one() {
        let out = rejection(&head("report-status side-band-64k"), "waiting", "Waiting for approval.\nRun git again.");
        let packets = data(&out);
        assert_eq!(packets[0], b"\x02Waiting for approval.\n");
        assert_eq!(packets[1], b"\x02Run git again.\n");
        assert_eq!(packets[2][0], 1);
        assert_eq!(packets[3], b"<flush>");
        let inner = data(&packets[2][1..]);
        assert_eq!(inner[1], b"ng refs/heads/main waiting\n");
        assert_eq!(inner.last().unwrap(), b"<flush>");
    }

    #[test]
    fn long_reasons_are_cut() {
        assert_eq!(one_line(&"x".repeat(500), 10), "xxxxxxx...");
        assert_eq!(one_line("a\tb\r\nc", 10), "a b c");
    }
}
