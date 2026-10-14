//! Keys & secrets' scan and move in the app ([`crate::secrets`]): the scan runs on a background thread and reports
//! its progress to the window; the move runs the same import as `reins secrets import` (the phone approves the batch)
//! and shows its check digits while the phone decides; then what changed, removing the originals from the trash, or
//! undoing it. The window is a person's (an agent does not click it), so its scan asks nothing first.
//!
//! `--demo` never scans this computer: made-up findings stand in, the pretend phone approves after a few seconds, and
//! nothing is written. `REINS_DEMO_SECRETS=scanning|review|detail|moving|done` opens that step, for screenshots.

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
use crate::secrets::{Finished, Flow, Moving, Review, Scanning};

/// `--demo`: how long each stage of the pretend scan takes, and the pretend phone.
const DEMO_STAGE: Duration = Duration::from_millis(450);
const DEMO_APPROVE: Duration = Duration::from_secs(3);
/// How often the scan's progress reaches the window.
const PROGRESS_EVERY: Duration = Duration::from_millis(80);

/// The flow at start: in the demo, as `REINS_DEMO_SECRETS` says.
pub(super) fn initial(demo: bool, home: &std::path::Path) -> Flow {
    let step = std::env::var("REINS_DEMO_SECRETS").ok().filter(|_| demo).unwrap_or_default();
    let review = || {
        let mut r = Review::new(Arc::new(Demo::secrets_report(home)));
        // As the mockup: the shell exports are left for later.
        if let Some(f) = r.report.findings.iter().find(|f| f.source == Source::Shell).map(|f| f.id.clone()) {
            r.toggle_finding(&f);
        }
        r
    };
    match step.as_str() {
        "scanning" => Flow::Scanning(Scanning {
            progress: Demo::secrets_progress(3),
            stop: Arc::new(AtomicBool::new(false)),
        }),
        "review" => Flow::Review(review()),
        "detail" => {
            let mut r = review();
            r.open = r.report.findings.first().map(|f| f.id.clone());
            Flow::Review(r)
        }
        "moving" => {
            let r = review();
            Flow::Moving(moving_of(
                &secrets_import::plan(&r.report, &r.ticked, "Dana's MacBook"),
                Some("4819".to_owned()),
            ))
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
                                m.secrets = Flow::Review(Review::new(Arc::new(Demo::secrets_report(&home))));
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
        cx.spawn(async move |this, cx| {
            let result = scan.await;
            let _gone = this.update(cx, |m, cx| {
                m.secrets = match result {
                    Ok(report) if report.stopped => Flow::Idle,
                    Ok(report) => Flow::Review(Review::new(Arc::new(report))),
                    Err(e) => {
                        m.notice = Some(e);
                        Flow::Idle
                    }
                };
                cx.notify();
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

    /// "Move N · approve on phone": the ticked secrets go to the vault as one batch; the window shows the check
    /// digits while the phone decides.
    pub fn move_secrets(&mut self, cx: &mut Context<'_, Self>) {
        let Flow::Review(review) = &self.secrets else {
            return;
        };
        if review.count() == 0 {
            return;
        }
        let review = review.clone();
        let home = self.backend.home().to_path_buf();
        let computer = if self.args.demo {
            "Dana's MacBook".to_owned()
        } else {
            secrets_scan::computer_name()
        };
        let plan = secrets_import::plan(&review.report, &review.ticked, &computer);
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
        cx.spawn(async move |this, cx| {
            let moved = Tokio::spawn(cx, async move {
                let ctx = ImportContext {
                    home: home.clone(),
                    state_dir: paths.state_dir.clone(),
                    computer,
                    caller: Caller::Person,
                };
                let writer = real::PhoneWriter {
                    paths: paths.clone(),
                    config: config.clone(),
                };
                let system = real::System {
                    paths,
                    config,
                    home,
                };
                secrets_import::import(&ctx, &report, &ticked, keep_local, &writer, &system, &mut |_, check| {
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
                m.refresh_now(cx);
                cx.notify();
            });
        })
        .detach();
    }

    /// "Remove the originals from the Trash" (held to confirm): deletes for good exactly the files the import sent
    /// there.
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
                format!("Deleted the {n} originals from the Trash.")
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
