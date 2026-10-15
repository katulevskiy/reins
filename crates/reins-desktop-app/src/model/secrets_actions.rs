//! Keys & secrets' scan and move in the app ([`crate::secrets`]): the scan runs on a background thread and reports
//! its progress to the window; the move runs the same import as `reins secrets import` (the phone approves the batch)
//! and shows its check digits while the phone decides; then what changed, removing the originals from the trash, or
//! undoing it. The window is a person's (an agent does not click it), so its scan asks nothing first.
//!
//! `--demo` never scans this computer: made-up findings stand in, the pretend phone approves after a few seconds, and
//! nothing is written. `REINS_DEMO_SECRETS=scanning|review|names|offline|checked|key|detail|moving|done` opens that
//! step, for screenshots (`key`: the review asking for the phone key's digits; `names`: the names step's folds open;
//! `offline`: the vault could not be listed; `checked`: the vault listed); `run` plays the whole flow by itself.
//! `REINS_DEMO_SCAN=large` makes it a big scan whose vault is not checked yet ([`crate::demo_scan`]).

use std::collections::BTreeSet;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

use futures::StreamExt as _;
use gpui::Context;
use gpui_tokio::Tokio;
use reins_desktop::secrets_import::{self, Context as ImportContext, Plan, real, receipt};
use reins_desktop::secrets_scan::{self, Caller, Progress, ScanOptions, Source};

use super::Model;
use crate::demo::Demo;
use crate::secrets::{self, Finished, Flow, MoveState, Moving, PhoneKey, Review, Scanning};

/// `--demo`: how long each stage of the pretend scan takes, and the pretend phone.
const DEMO_STAGE: Duration = Duration::from_millis(450);
const DEMO_APPROVE: Duration = Duration::from_secs(3);
/// How often the scan's progress reaches the window.
const PROGRESS_EVERY: Duration = Duration::from_millis(80);

/// `--demo` with `REINS_DEMO_SECRETS=run`: how long the review shows before the pretend move.
const DEMO_REVIEW: Duration = Duration::from_millis(2_500);

/// `--demo` with `REINS_DEMO_SECRETS=run`: the whole flow plays by itself (scan, review, move, done), as if clicked.
pub(super) fn autoplay(demo: bool) -> bool {
    demo && std::env::var("REINS_DEMO_SECRETS").is_ok_and(|v| v == "run")
}

/// The flow at start: in the demo, as `REINS_DEMO_SECRETS` says.
pub(super) fn initial(demo: bool, home: &std::path::Path) -> Flow {
    let step = std::env::var("REINS_DEMO_SECRETS").ok().filter(|_| demo).unwrap_or_default();
    let review = || {
        let mut r = Review::new(Arc::new(Demo::scan(home)), Demo::scan_vault());
        // As the mockup: the shell exports are left for later.
        if let Some(f) = r.report.findings.iter().find(|f| f.source == Source::Shell).map(|f| f.id.clone()) {
            r.toggle_finding(&f);
        }
        // `REINS_DEMO_SCAN=large`: as a first scan opens, the phone's key not checked here yet.
        if Demo::large_scan() {
            r.key = Some(PhoneKey::default());
        }
        r
    };
    match step.as_str() {
        "scanning" => Flow::Scanning(Scanning {
            progress: Demo::secrets_progress(3),
            stop: Arc::new(AtomicBool::new(false)),
        }),
        "review" => Flow::Review(review()),
        // The names step with its folds open.
        "names" => {
            let mut r = review();
            r.open_names = true;
            r.open_dups = true;
            Flow::Review(r)
        }
        // The phone did not answer the listing: said once, with Retry.
        "offline" => {
            let mut r = review();
            r.key = None;
            r.existing = secrets_import::Existing::Unknown("no answer from your phone in time".to_owned());
            Flow::Review(r)
        }
        // The phone's key checked and the vault listed: what needs a choice shows.
        "checked" => {
            let mut r = review();
            r.key = None;
            r.existing = Demo::listed_vault();
            Flow::Review(r)
        }
        "key" => {
            let mut r = review();
            r.key = Some(PhoneKey {
                typed: "4821-9930-12".to_owned(),
                checking: false,
            });
            Flow::Review(r)
        }
        "detail" => {
            let mut r = review();
            r.open = r.report.findings.first().map(|f| f.id.clone());
            Flow::Review(r)
        }
        "moving" => {
            let r = review();
            Flow::Moving(moving_of(&r.plan("Dana's MacBook", &Demo::machine()), Some("4819".to_owned())))
        }
        "done" => {
            let r = review();
            Flow::Done(Finished {
                receipt: Demo::secrets_receipt(&r.report, &r.ticked, false),
                path: Some(home.join(".local/state/reins/imports/demo.json")),
                after: None,
            })
        }
        _ => Flow::Idle,
    }
}

/// What the waiting screen lists for a plan: each target, how many secrets it holds.
fn moving_of(plan: &Plan, check: Option<String>) -> Moving {
    let keys = |n: usize| {
        format!(
            "{n} key{} · sealed to your phone",
            if n == 1 {
                ""
            } else {
                "s"
            }
        )
    };
    // A project's environments first, then keys and tokens, then the computer's own credentials.
    let (project, machine): (Vec<&secrets_import::EnvTarget>, Vec<_>) =
        plan.envs.iter().partition(|e| e.project_root.is_some());
    let mut targets: Vec<(String, String, Source, usize)> =
        project.iter().map(|e| (e.item(), keys(e.vars.len()), Source::Dotenv, e.vars.len())).collect();
    targets.extend(
        plan.ssh.iter().map(|s| (s.name.clone(), "SSH key · sealed to your phone".to_owned(), Source::SshKey, 1)),
    );
    targets.extend(
        plan.tokens.iter().map(|t| (t.host.clone(), "git token · sealed to your phone".to_owned(), Source::GhCli, 1)),
    );
    targets.extend(machine.iter().map(|e| (e.item(), keys(e.vars.len()), Source::Aws, e.vars.len())));
    Moving {
        check,
        targets,
        count: plan.count(),
    }
}

impl Model {
    fn config_now(&self) -> Option<reins_desktop::config::Config> {
        self.snapshot.as_ref().and_then(|s| s.config.as_ref().ok()).map(|c| c.as_ref().clone())
    }

    /// "Scan this computer": the scan, off the UI thread, its progress shown as it goes.
    pub fn start_scan(&mut self, cx: &mut Context<'_, Self>) {
        if matches!(self.secrets, Flow::Scanning(_) | Flow::Moving(_)) {
            return;
        }
        let stop = Arc::new(AtomicBool::new(false));
        self.secrets = Flow::Scanning(Scanning {
            progress: Demo::secrets_progress(0),
            stop: Arc::clone(&stop),
        });
        if !self.args.demo
            && let Flow::Scanning(s) = &mut self.secrets
        {
            s.progress = Progress::default();
        }
        self.notice = None;
        self.note = None;
        cx.notify();
        let home = self.backend.home().to_path_buf();
        if self.args.demo {
            cx.spawn(async move |this, cx| {
                for step in 1..=6 {
                    cx.background_executor().timer(DEMO_STAGE).await;
                    let going = this
                        .update(cx, |m, cx| {
                            let Flow::Scanning(s) = &mut m.secrets else {
                                return false;
                            };
                            if s.stop.load(std::sync::atomic::Ordering::Relaxed) {
                                m.secrets = Flow::Idle;
                                cx.notify();
                                return false;
                            }
                            if step < 6 {
                                s.progress = Demo::secrets_progress(step);
                            } else {
                                let mut r = Review::new(Arc::new(Demo::scan(&home)), Demo::scan_vault());
                                if Demo::large_scan() {
                                    r.key = Some(PhoneKey::default());
                                }
                                m.secrets = Flow::Review(r);
                                // `REINS_DEMO_SECRETS=run`: the move follows by itself, as if clicked.
                                if autoplay(true) {
                                    cx.spawn(async move |this, cx| {
                                        cx.background_executor().timer(DEMO_REVIEW).await;
                                        let _gone = this.update(cx, Model::move_secrets);
                                    })
                                    .detach();
                                }
                            }
                            cx.notify();
                            true
                        })
                        .unwrap_or(false);
                    if !going {
                        break;
                    }
                }
            })
            .detach();
            return;
        }
        let paths = self.backend.paths().clone();
        let config = self.config_now().unwrap_or_default();
        let (tx, mut rx) = futures::channel::mpsc::unbounded::<Progress>();
        let scan = cx.background_executor().spawn(async move {
            let mut opts = ScanOptions::new(home.clone());
            // The window is a person's: whatever its environment says, a click started this.
            opts.caller = Caller::Person;
            opts.exposure = secrets_scan::Exposure::detect(&paths, &config, &home);
            opts.stop = Some(stop);
            let mut last = Instant::now();
            let mut last_stages = std::collections::BTreeMap::default();
            secrets_scan::scan(&opts, &mut |p| {
                if p.stages != last_stages || last.elapsed() >= PROGRESS_EVERY {
                    last = Instant::now();
                    last_stages.clone_from(&p.stages);
                    tx.unbounded_send(p.clone()).ok();
                }
            })
        });
        cx.spawn(async move |this, cx| {
            while let Some(p) = rx.next().await {
                let gone = this
                    .update(cx, |m, cx| {
                        if let Flow::Scanning(s) = &mut m.secrets {
                            s.progress = p;
                            cx.notify();
                        }
                    })
                    .is_err();
                if gone {
                    break;
                }
            }
        })
        .detach();
        let writer = self.phone_writer(None);
        // Never checked here: the review asks for the phone key's digits first (a window cannot ask at a terminal),
        // and the vault's projects are listed once it is checked (the phone seals the list to that key).
        let unchecked = writer.phone_key_unchecked();
        cx.spawn(async move |this, cx| {
            let result = scan.await;
            let _gone = this.update(cx, |m, cx| {
                m.secrets = match result {
                    Ok(report) if report.stopped => Flow::Idle,
                    Ok(report) => {
                        // Not known until listed: never merged meanwhile, and the names say so once.
                        let why = if unchecked {
                            secrets::KEY_UNCHECKED
                        } else {
                            "asking your phone"
                        };
                        let mut r = Review::new(Arc::new(report), secrets_import::Existing::Unknown(why.to_owned()));
                        r.key = unchecked.then(PhoneKey::default);
                        Flow::Review(r)
                    }
                    Err(e) => {
                        m.notice = Some(e);
                        Flow::Idle
                    }
                };
                // The review shows now; what the vault has follows (one approval on the phone).
                if !unchecked {
                    m.list_vault(cx);
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn phone_writer(&self, digits: Option<String>) -> real::PhoneWriter {
        real::PhoneWriter {
            paths: self.backend.paths().clone(),
            config: self.config_now().unwrap_or_default(),
            phone_key_digits: digits,
        }
    }

    /// Lists the vault's projects (names and origins only, one approval on the phone), so the review never merges
    /// into a project from elsewhere unasked: after a scan, and "Retry" when it could not.
    pub fn list_vault(&mut self, cx: &mut Context<'_, Self>) {
        let Flow::Review(r) = &mut self.secrets else {
            return;
        };
        if r.listing || r.key.is_some() {
            return;
        }
        r.listing = true;
        r.existing = secrets_import::Existing::Unknown("asking your phone".to_owned());
        cx.notify();
        if self.args.demo {
            cx.spawn(async move |this, cx| {
                cx.background_executor().timer(DEMO_APPROVE).await;
                let _gone = this.update(cx, |m, cx| {
                    m.review(cx, |r| {
                        r.listing = false;
                        r.existing = Demo::listed_vault();
                    });
                });
            })
            .detach();
            return;
        }
        let writer = self.phone_writer(None);
        cx.spawn(async move |this, cx| {
            let existing = Tokio::spawn(cx, async move { secrets_import::existing(&writer).await })
                .await
                .unwrap_or_else(|e| secrets_import::Existing::Unknown(e.to_string()));
            let _gone = this.update(cx, |m, cx| {
                m.review(cx, |r| {
                    r.listing = false;
                    r.existing = existing;
                });
            });
        })
        .detach();
    }

    /// "Stop": the scan ends where it is.
    pub fn stop_scan(&mut self, cx: &mut Context<'_, Self>) {
        if let Flow::Scanning(s) = &self.secrets {
            s.stop();
            if self.args.demo {
                self.secrets = Flow::Idle;
            }
            cx.notify();
        }
    }

    /// Back to the Keys & secrets page.
    pub fn secrets_home(&mut self, cx: &mut Context<'_, Self>) {
        if !matches!(self.secrets, Flow::Moving(_)) {
            self.secrets = Flow::Idle;
            cx.notify();
        }
    }

    /// Changes the review (ticks, the open file, what happens after).
    pub fn review(&mut self, cx: &mut Context<'_, Self>, f: impl FnOnce(&mut Review)) {
        if let Flow::Review(r) = &mut self.secrets {
            f(r);
            cx.notify();
        }
    }

    /// "Check" (or Enter) under the phone's key: the phone hands this computer its key (one approval), it is kept
    /// when it matches the twelve digits typed, and the vault's projects are listed again with it.
    pub fn check_phone_key(&mut self, cx: &mut Context<'_, Self>) {
        let Flow::Review(review) = &mut self.secrets else {
            return;
        };
        let Some(key) = review.key.as_mut() else {
            return;
        };
        if key.checking {
            return;
        }
        let Some(digits) = key.digits() else {
            review.error = Some("Type the twelve digits your phone shows for its key.".to_owned());
            cx.notify();
            return;
        };
        key.checking = true;
        review.error = None;
        cx.notify();
        if self.args.demo {
            cx.spawn(async move |this, cx| {
                cx.background_executor().timer(DEMO_APPROVE).await;
                let _gone = this.update(cx, |m, cx| {
                    m.review(cx, |r| {
                        r.key = None;
                        // The pretend phone lists the vault's projects once its key is checked.
                        r.existing = Demo::listed_vault();
                    });
                });
            })
            .detach();
            return;
        }
        let writer = self.phone_writer(Some(digits));
        cx.spawn(async move |this, cx| {
            let checked = Tokio::spawn(cx, async move {
                writer.check_phone_key().await?;
                Ok::<_, String>(secrets_import::existing(&writer).await)
            })
            .await
            .unwrap_or_else(|e| Err(e.to_string()));
            let _gone = this.update(cx, |m, cx| {
                m.review(cx, |r| match checked {
                    Ok(existing) => {
                        r.key = None;
                        r.existing = existing;
                    }
                    Err(e) => {
                        if let Some(k) = r.key.as_mut() {
                            k.checking = false;
                        }
                        r.error = Some(e);
                    }
                });
            });
        })
        .detach();
    }

    /// "Move N · approve on phone": the ticked secrets go to the vault as one batch; the window shows the check
    /// digits while the phone decides.
    pub fn move_secrets(&mut self, cx: &mut Context<'_, Self>) {
        let Flow::Review(review) = &self.secrets else {
            return;
        };
        // The phone's key is checked first (its digits typed in the review), and the names are settled.
        match review.move_state() {
            MoveState::Ready(_) => {}
            MoveState::CheckKey => return self.check_phone_key(cx),
            _ => return,
        }
        let review = review.clone();
        let home = self.backend.home().to_path_buf();
        let (computer, machine) = (self.computer.clone(), self.machine.clone());
        let plan = review.plan(&computer, &machine);
        self.secrets = Flow::Moving(moving_of(
            &plan,
            if self.args.demo {
                Some("4819".to_owned())
            } else {
                None
            },
        ));
        cx.notify();
        if self.args.demo {
            cx.spawn(async move |this, cx| {
                cx.background_executor().timer(DEMO_APPROVE).await;
                let _gone = this.update(cx, |m, cx| {
                    m.secrets = Flow::Done(Finished {
                        receipt: Demo::secrets_receipt(&review.report, &review.ticked, review.keep_local),
                        path: Some(home.join(".local/state/reins/imports/demo.json")),
                        after: None,
                    });
                    cx.notify();
                });
            })
            .detach();
            return;
        }
        let paths = self.backend.paths().clone();
        let config = self.config_now().unwrap_or_default();
        let (tx, mut rx) = futures::channel::mpsc::unbounded::<String>();
        cx.spawn(async move |this, cx| {
            while let Some(check) = rx.next().await {
                let _gone = this.update(cx, |m, cx| {
                    if let Flow::Moving(moving) = &mut m.secrets {
                        moving.check = Some(check);
                        cx.notify();
                    }
                });
            }
        })
        .detach();
        let report = Arc::clone(&review.report);
        let ticked: BTreeSet<String> = review.ticked.clone();
        let keep_local = review.keep_local;
        let merge = review.merge.clone();
        // The import names the projects against the vault as it is now; if that is not what the review showed (a
        // project of the same name appeared, or the list could not be read this time), nothing is sent.
        let reviewed = plan;
        let changed = Arc::new(AtomicBool::new(false));
        let changed_seen = Arc::clone(&changed);
        cx.spawn(async move |this, cx| {
            let moved = Tokio::spawn(cx, async move {
                let ctx = ImportContext {
                    home: home.clone(),
                    state_dir: paths.state_dir.clone(),
                    computer,
                    machine,
                    caller: Caller::Person,
                    merge,
                };
                let writer = real::PhoneWriter {
                    paths: paths.clone(),
                    config: config.clone(),
                    phone_key_digits: None,
                };
                let system = real::System {
                    paths,
                    config,
                    home,
                };
                secrets_import::import(&ctx, &report, &ticked, keep_local, &writer, &system, &mut |plan, check| {
                    if !secrets::same_places(plan, &reviewed) {
                        changed.store(true, std::sync::atomic::Ordering::Relaxed);
                        return Err("Your vault's projects changed since the review: check the names, then move \
                                    again. Nothing was sent."
                            .to_owned());
                    }
                    tx.unbounded_send(check.to_owned()).ok();
                    Ok(())
                })
                .await
            })
            .await
            .unwrap_or_else(|e| Err(e.to_string()));
            let _gone = this.update(cx, |m, cx| {
                m.secrets = match moved {
                    Ok((receipt, path)) => Flow::Done(Finished {
                        receipt,
                        path,
                        after: None,
                    }),
                    Err(e) => {
                        let mut r = review;
                        r.error = Some(e);
                        Flow::Review(r)
                    }
                };
                // The names changed under the review: list the vault again, so it shows what a move would do.
                if changed_seen.load(std::sync::atomic::Ordering::Relaxed) {
                    m.list_vault(cx);
                }
                m.refresh_now(cx);
                cx.notify();
            });
        })
        .detach();
    }

    /// "Shred" (held to confirm): overwrites and deletes exactly the files the import sent to the Trash.
    pub fn purge_originals(&mut self, cx: &mut Context<'_, Self>) {
        self.after_done(cx, true);
    }

    /// "Undo": the files come back from the trash.
    pub fn undo_import(&mut self, cx: &mut Context<'_, Self>) {
        self.after_done(cx, false);
    }

    fn after_done(&mut self, cx: &mut Context<'_, Self>, purge: bool) {
        let Flow::Done(done) = &mut self.secrets else {
            return;
        };
        if self.args.demo {
            let n = done.receipt.trashed().len();
            done.after = Some(if purge {
                done.receipt.purged_at = Some(reins_desktop::now_unix());
                for m in done.receipt.moved.iter_mut().filter(|m| m.in_trash) {
                    m.in_trash = false;
                    m.on_disk = false;
                }
                format!("Shredded the {n} originals.")
            } else {
                done.receipt.undone_at = Some(reins_desktop::now_unix());
                "Undone: the files are back. The secrets stay in your vault.".to_owned()
            });
            cx.notify();
            return;
        }
        let Some(path) = done.path.clone() else {
            return;
        };
        let paths = self.backend.paths().clone();
        let config = self.config_now().unwrap_or_default();
        let home = self.backend.home().to_path_buf();
        let task = cx.background_executor().spawn(async move {
            let system = real::System {
                paths: paths.clone(),
                config,
                home: home.clone(),
            };
            let name = path.to_string_lossy().into_owned();
            let (path, mut r) = receipt::load(&paths.state_dir, &name)?;
            let lines = if purge {
                receipt::purge(&paths.state_dir, &path, &mut r, &system)?
            } else {
                receipt::undo(&home, &paths.state_dir, &path, &mut r, &system)?
            };
            Ok::<_, String>((r, lines))
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _gone = this.update(cx, |m, cx| {
                if let Flow::Done(done) = &mut m.secrets {
                    match result {
                        Ok((receipt, lines)) => {
                            done.receipt = receipt;
                            done.after = Some(lines.join("\n"));
                        }
                        Err(e) => done.after = Some(e),
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }
}
