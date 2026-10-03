//! The proxy with the real git client against a `git http-backend` upstream, decided by a scripted authorizer.

mod proxy_support;

use std::io::Write as _;

use proxy_support::{Answer, Home, Proxy, Scripted, TOKEN, Upstream, logs, raw_http, token_basic};
use reins_desktop::control::Client;
use reins_desktop::git::pktline::{FLUSH, Pkt, Reader, encode};
use reins_proto::desktop::push_digest;
use sha2::{Digest as _, Sha256};

fn assert_no_secret(text: &str) {
    assert!(!text.contains(TOKEN), "the token leaked:\n{text}");
    assert!(!text.contains(&token_basic()), "the Authorization header leaked:\n{text}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_public_clone_needs_no_approval() {
    let up = Upstream::start().await;
    up.create("me/pub", false);
    let auth = Scripted::new(Answer::Deny("no reads"), Answer::Deny("no pushes"));
    let proxy = Proxy::scripted(&up, &auth).await;
    let home = Home::new(proxy.addr());
    home.git_ok("", &["clone", "-q", "https://github.com/me/pub.git", "pub"]).await;
    assert!(home.path("pub/README.md").exists());
    assert!(auth.reads.lock().unwrap().is_empty());
    assert!(up.seen().iter().all(|s| s.authorization.is_none()));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_private_clone_asks_and_the_token_reaches_only_upstream() {
    let up = Upstream::start().await;
    up.create("me/priv", true);
    let auth = Scripted::new(Answer::Allow, Answer::Deny("no pushes"));
    let proxy = Proxy::scripted(&up, &auth).await;
    let home = Home::new(proxy.addr());
    let run = home.git_ok("", &["clone", "https://github.com/me/priv", "priv"]).await;
    assert!(home.path("priv/README.md").exists());
    assert!(auth.reads.lock().unwrap().iter().all(|r| r == "me/priv"));
    assert!(!auth.reads.lock().unwrap().is_empty());
    let seen = up.seen();
    // One anonymous try, then always with the credential (the repository is remembered as private).
    assert_eq!(seen.iter().filter(|s| s.status == 401).count(), 1, "{seen:#?}");
    assert!(
        seen.iter()
            .filter(|s| s.status == 200)
            .all(|s| s.authorization.as_deref() == Some(&format!("Basic {}", token_basic())))
    );
    assert_no_secret(&run.all());
    let fetch = home.git_ok("priv", &["fetch", "origin"]).await;
    assert_no_secret(&fetch.all());
    assert_eq!(up.seen().iter().filter(|s| s.status == 401).count(), 1);
    assert_no_secret(&logs());
    assert_no_secret(&std::fs::read_to_string(home.path("priv/.git/config")).unwrap());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn refused_and_waiting_reads_are_shown_by_git() {
    let up = Upstream::start().await;
    up.create("me/priv", true);
    let auth = Scripted::new(
        Answer::Wait("Waiting for approval on your phone. Approve it, then run git again."),
        Answer::Allow,
    );
    let proxy = Proxy::scripted(&up, &auth).await;
    let home = Home::new(proxy.addr());
    let run = home.git("", &["clone", "https://github.com/me/priv", "priv"]).await;
    assert!(!run.ok);
    assert!(
        run.stderr.contains("remote: Waiting for approval on your phone. Approve it, then run git again."),
        "{}",
        run.stderr
    );
    *auth.read.lock().unwrap() = Answer::Deny("Denied on your phone.");
    let run = home.git("", &["clone", "https://github.com/me/priv", "priv"]).await;
    assert!(run.stderr.contains("remote: Denied on your phone."), "{}", run.stderr);
    assert!(!home.path("priv/README.md").exists());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_approved_push_lands_upstream_with_the_push_digest() {
    let up = Upstream::start().await;
    up.create("me/priv", true);
    let auth = Scripted::new(Answer::Allow, Answer::Allow);
    let proxy = Proxy::scripted(&up, &auth).await;
    let home = Home::new(proxy.addr());
    home.git_ok("", &["clone", "-q", "https://github.com/me/priv", "priv"]).await;
    let old = up.head("me/priv", "main").unwrap();
    let new = home.commit("priv", "a.txt", b"hello\n", "Add a").await;
    let run = home.git_ok("priv", &["push", "origin", "main"]).await;
    assert_eq!(up.head("me/priv", "main").as_deref(), Some(new.as_str()));
    assert_no_secret(&run.all());
    let pushes = auth.pushes.lock().unwrap().clone();
    assert_eq!(pushes.len(), 1);
    let (repo, summary, digest) = &pushes[0];
    assert_eq!(repo, "me/priv");
    assert_eq!(summary.updates[0].name, "refs/heads/main");
    assert_eq!((summary.updates[0].old.as_str(), summary.updates[0].new.as_str()), (old.as_str(), new.as_str()));
    assert!(summary.pack_bytes > 0);
    assert_eq!(digest.len(), 64);
    assert_no_secret(&logs());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_denied_push_is_rejected_by_git_and_nothing_reaches_upstream() {
    let up = Upstream::start().await;
    up.create("me/priv", true);
    let auth = Scripted::new(Answer::Allow, Answer::Deny("Denied on your phone."));
    let proxy = Proxy::scripted(&up, &auth).await;
    let home = Home::new(proxy.addr());
    home.git_ok("", &["clone", "-q", "https://github.com/me/priv", "priv"]).await;
    let before = up.head("me/priv", "main");
    home.commit("priv", "a.txt", b"hello\n", "Add a").await;
    home.git_ok("priv", &["branch", "feature"]).await;
    let run = home.git("priv", &["push", "origin", "main", "feature"]).await;
    assert!(!run.ok);
    assert!(run.stderr.contains("! [remote rejected] main -> main (Denied on your phone.)"), "{}", run.stderr);
    assert!(run.stderr.contains("! [remote rejected] feature -> feature (Denied on your phone.)"), "{}", run.stderr);
    assert!(run.stderr.contains("remote: Denied on your phone."), "{}", run.stderr);
    assert_eq!(up.head("me/priv", "main"), before);
    assert_eq!(up.head("me/priv", "feature"), None);
    assert!(up.seen().iter().all(|s| !(s.method == "POST" && s.path.ends_with("git-receive-pack"))));

    *auth.push.lock().unwrap() =
        Answer::Wait("Waiting for approval on your phone. Approve it, then run git push again.");
    let run = home.git("priv", &["push", "origin", "main"]).await;
    assert!(
        run.stderr.contains("! [remote rejected] main -> main (Waiting for approval on your phone."),
        "{}",
        run.stderr
    );
    assert!(
        run.stderr.contains("remote: Waiting for approval on your phone. Approve it, then run git push again."),
        "{}",
        run.stderr
    );
    assert_eq!(up.head("me/priv", "main"), before);
}

fn random_bytes(n: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(n + 32);
    let mut block = Sha256::digest(b"seed").to_vec();
    while out.len() < n {
        block = Sha256::digest(&block).to_vec();
        out.extend_from_slice(&block);
    }
    out.truncate(n);
    out
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_large_push_is_chunked_after_a_probe_and_still_approved_once() {
    let up = Upstream::start().await;
    up.create("me/priv", true);
    let auth = Scripted::new(Answer::Allow, Answer::Allow);
    let proxy = Proxy::scripted(&up, &auth).await;
    let home = Home::new(proxy.addr());
    home.git_ok("", &["clone", "-q", "https://github.com/me/priv", "priv"]).await;
    let new = home.commit("priv", "big.bin", &random_bytes(3 << 20), "Big file").await;
    home.git_ok("priv", &["push", "origin", "main"]).await;
    assert_eq!(up.head("me/priv", "main").as_deref(), Some(new.as_str()));
    let posts: Vec<_> =
        up.seen().into_iter().filter(|s| s.method == "POST" && s.path.ends_with("git-receive-pack")).collect();
    assert_eq!(posts.len(), 2, "{posts:#?}");
    assert_eq!(posts[0].body_len, 4, "the probe is a flush");
    assert!(posts[1].body_len > 3 << 20);
    assert_eq!(auth.pushes.lock().unwrap().len(), 1);
}

fn pkt_data(buf: &[u8]) -> Vec<Vec<u8>> {
    let mut r = Reader::new(buf);
    let mut out = Vec::new();
    while let Ok(Some(p)) = r.next_pkt() {
        if let Pkt::Data(d) = p {
            out.push(d.to_vec());
        }
    }
    out
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_gzip_push_body_is_read_for_approval_and_forwarded_as_sent() {
    let up = Upstream::start().await;
    up.create("me/priv", true);
    let auth = Scripted::new(Answer::Allow, Answer::Allow);
    let proxy = Proxy::scripted(&up, &auth).await;
    let home = Home::new(proxy.addr());
    home.git_ok("", &["clone", "-q", "https://github.com/me/priv", "priv"]).await;
    let old = up.head("me/priv", "main").unwrap();
    let new = home.commit("priv", "z.txt", b"zipped\n", "Zipped").await;
    let pack = std::process::Command::new("git")
        .current_dir(home.path("priv"))
        .args(["pack-objects", "--stdout", "--revs", "-q"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .and_then(|mut c| {
            c.stdin.take().unwrap().write_all(format!("{new}\n^{old}\n").as_bytes())?;
            c.wait_with_output()
        })
        .unwrap()
        .stdout;
    let mut body = encode(format!("{old} {new} refs/heads/main\0report-status agent=test\n").as_bytes());
    body.extend_from_slice(FLUSH);
    body.extend_from_slice(&pack);
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    gz.write_all(&body).unwrap();
    let zipped = gz.finish().unwrap();
    let resp = reins_desktop::http::client(None)
        .unwrap()
        .post(format!("http://{}/github.com/me/priv.git/git-receive-pack", proxy.addr()))
        .header("content-type", "application/x-git-receive-pack-request")
        .header("content-encoding", "gzip")
        .body(zipped.clone())
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let answer = resp.bytes().await.unwrap();
    let lines = pkt_data(&answer);
    assert_eq!(lines, vec![b"unpack ok\n".to_vec(), b"ok refs/heads/main\n".to_vec()]);
    assert_eq!(up.head("me/priv", "main").as_deref(), Some(new.as_str()));
    let forwarded = up.seen().into_iter().rfind(|s| s.path.ends_with("git-receive-pack")).unwrap();
    assert_eq!((forwarded.content_encoding.as_deref(), forwarded.body_len), (Some("gzip"), zipped.len()));
    let pack_sha = data_encoding::HEXLOWER.encode(&Sha256::digest(&pack));
    let expected = push_digest("me/priv", &[(old, new, "refs/heads/main".to_owned())], &pack_sha, &[]);
    assert_eq!(auth.pushes.lock().unwrap()[0].2, expected);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn protocol_v2_and_v0_fetches_work() {
    let up = Upstream::start().await;
    up.create("me/pub", false);
    let auth = Scripted::new(Answer::Allow, Answer::Allow);
    let proxy = Proxy::scripted(&up, &auth).await;
    let home = Home::new(proxy.addr());
    home.git_ok("", &["-c", "protocol.version=2", "clone", "-q", "https://github.com/me/pub", "v2"]).await;
    assert!(up.seen().iter().any(|s| s.git_protocol.as_deref() == Some("version=2") && s.method == "POST"));
    home.git_ok("", &["-c", "protocol.version=0", "clone", "-q", "https://github.com/me/pub", "v0"]).await;
    // A new upstream commit arrives by fetch.
    let seed = up.dir.path().join("seed/me/pub");
    std::fs::write(seed.join("b.txt"), "b\n").unwrap();
    proxy_support::run_git(&seed, &[], &["add", "b.txt"]);
    proxy_support::run_git(&seed, &[], &["-c", "user.name=S", "-c", "user.email=s@e", "commit", "-q", "-m", "B"]);
    proxy_support::run_git(&seed, &[], &["push", "-q", up.bare("me/pub").to_str().unwrap(), "main"]);
    home.git_ok("v2", &["-c", "protocol.version=2", "pull", "-q", "--ff-only"]).await;
    assert!(home.path("v2/b.txt").exists());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_renamed_repository_is_followed_inside_the_proxy() {
    let up = Upstream::start().await;
    up.create("me/new-name", true);
    up.move_repo("me/old-name", "me/new-name");
    let auth = Scripted::new(Answer::Allow, Answer::Allow);
    let proxy = Proxy::scripted(&up, &auth).await;
    let home = Home::new(proxy.addr());
    let run = home.git_ok("", &["clone", "https://github.com/me/old-name", "renamed"]).await;
    assert!(home.path("renamed/README.md").exists());
    assert!(!run.all().contains("new-name"), "git must not see the new location:\n{}", run.all());
    // Once learnt, requests go to the new place directly.
    assert_eq!(up.seen().iter().filter(|s| s.status == 301).count(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn lfs_requests_carry_the_read_credential() {
    let up = Upstream::start().await;
    up.create("me/priv", true);
    let auth = Scripted::new(Answer::Allow, Answer::Allow);
    let proxy = Proxy::scripted(&up, &auth).await;
    let resp = reins_desktop::http::client(None)
        .unwrap()
        .post(format!("http://{}/github.com/me/priv.git/info/lfs/objects/batch", proxy.addr()))
        .header("accept", "application/vnd.git-lfs+json")
        .header("content-type", "application/vnd.git-lfs+json")
        .body("{\"operation\":\"download\",\"objects\":[]}")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    assert_eq!(resp.text().await.unwrap(), "{\"authed\":true}");
    assert_eq!(auth.reads.lock().unwrap().as_slice(), ["me/priv"]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn foreign_hosts_browsers_and_unknown_paths_are_refused() {
    let up = Upstream::start().await;
    up.create("me/pub", false);
    let auth = Scripted::new(Answer::Allow, Answer::Allow);
    let proxy = Proxy::scripted(&up, &auth).await;
    let addr = proxy.addr();
    let path = "/github.com/me/pub.git/info/refs?service=git-upload-pack";
    let ok =
        raw_http(addr, &format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nConnection: close\r\n\r\n", addr.port()))
            .await;
    assert!(ok.starts_with("HTTP/1.1 200"), "{ok}");
    let ok =
        raw_http(addr, &format!("GET {path} HTTP/1.1\r\nHost: localhost:{}\r\nConnection: close\r\n\r\n", addr.port()))
            .await;
    assert!(ok.starts_with("HTTP/1.1 200"), "{ok}");
    for request in [
        format!("GET {path} HTTP/1.1\r\nHost: evil.example:{}\r\nConnection: close\r\n\r\n", addr.port()),
        format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1:1\r\nConnection: close\r\n\r\n"),
        format!("GET {path} HTTP/1.1\r\nConnection: close\r\n\r\n"),
        format!(
            "GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nOrigin: http://evil.example\r\nConnection: close\r\n\r\n",
            addr.port()
        ),
        format!("OPTIONS {path} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nConnection: close\r\n\r\n", addr.port()),
        format!(
            "GET /_reins/status HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nOrigin: null\r\nConnection: close\r\n\r\n",
            addr.port()
        ),
    ] {
        let answer = raw_http(addr, &request).await;
        assert!(answer.starts_with("HTTP/1.1 403"), "{request}\n{answer}");
    }
    for path in [
        "/gitlab.com/me/pub.git/info/refs?service=git-upload-pack",
        "/github.com/me/../x/info/refs?service=git-upload-pack",
        "/",
    ] {
        let answer = raw_http(
            addr,
            &format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nConnection: close\r\n\r\n", addr.port()),
        )
        .await;
        assert!(answer.starts_with("HTTP/1.1 404"), "{path}\n{answer}");
    }
    assert!(up.seen().len() <= 2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_control_api_needs_the_token() {
    let up = Upstream::start().await;
    let auth = Scripted::new(Answer::Allow, Answer::Allow);
    let proxy = Proxy::scripted(&up, &auth).await;
    let addr = proxy.addr();
    let without = raw_http(
        addr,
        &format!("GET /_reins/status HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nConnection: close\r\n\r\n", addr.port()),
    )
    .await;
    assert!(without.starts_with("HTTP/1.1 401"), "{without}");
    let wrong = raw_http(
        addr,
        &format!(
            "GET /_reins/pending HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nX-Reins-Token: nope\r\nConnection: close\r\n\r\n",
            addr.port()
        ),
    )
    .await;
    assert!(wrong.starts_with("HTTP/1.1 401"), "{wrong}");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = std::fs::metadata(proxy.paths.control_token_file()).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }
    let client = Client::new(&proxy.paths, addr).unwrap();
    let status = client.status().await.unwrap();
    assert_eq!(status.decides, "a script");
    assert_eq!(status.listen, addr.to_string());
    assert_eq!(status.proxy_base, format!("http://{addr}/github.com/"));
    assert_eq!(status.fingerprint.len(), 9);
    assert!(client.pending().await.unwrap().is_empty());
    assert!(client.answer("abcd1234", true).await.unwrap_err().to_string().contains("No pending approval"));
    assert!(proxy.control_token().len() >= 64);
}

#[cfg(target_os = "linux")]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn git_is_told_while_an_approval_is_awaited() {
    let up = Upstream::start().await;
    up.create("me/priv", true);
    let auth = Scripted::new(Answer::Allow, Answer::Allow);
    *auth.delay.lock().unwrap() = std::time::Duration::from_secs(2);
    let proxy = Proxy::scripted(&up, &auth).await;
    let home = Home::new(proxy.addr());
    let clone = home.git_ok("", &["clone", "-q", "https://github.com/me/priv", "priv"]).await;
    // The daemon tells the client through /proc (Linux only; elsewhere it says nothing, see notice.rs).
    if cfg!(target_os = "linux") {
        assert!(clone.stderr.contains("reins: waiting for approval: read github.com/me/priv…"), "{}", clone.all());
        assert!(clone.stderr.contains("reins: approved."), "{}", clone.all());
    }
    home.commit("priv", "a.txt", b"hello\n", "Add a").await;
    let push = home.git_ok("priv", &["push", "origin", "main"]).await;
    assert!(push.stderr.contains("push to github.com/me/priv: main (1 commit)…"), "{}", push.all());
    // A quick answer (the read is cached by nobody here, but no delay) says nothing.
    *auth.delay.lock().unwrap() = std::time::Duration::ZERO;
    let fetch = home.git_ok("priv", &["fetch", "origin"]).await;
    assert!(!fetch.stderr.contains("reins:"), "{}", fetch.all());
}
