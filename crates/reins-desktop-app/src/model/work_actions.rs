//! Work sessions in the app: the overview card's start form, asking the phone (with the approval wait's countdown),
//! and ending a session (from the card, after a confirmation, or from the tray's "End work session"). The running
//! session itself comes with every snapshot (`work-session.json`). `--demo` pretends: the phone approves after a few
//! seconds and ending takes a moment; nothing is sent, and nothing is written.

use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::Context;
use gpui_tokio::Tokio;
use reins_desktop::work_session::Session;

use super::Model;
use crate::backend::Snapshot;
use crate::demo::Demo;
use crate::work::{self, Asking, DemoStart, Form, Length, Work};

/// `--demo`: how long the pretend phone takes to approve a session (`REINS_DEMO_SESSION=waiting`: to look at the
/// waiting card).
const DEMO_APPROVE: Duration = Duration::from_secs(3);
const DEMO_APPROVE_SLOWLY: Duration = Duration::from_secs(60);
/// `--demo`: how long ending takes.
const DEMO_END: Duration = Duration::from_millis(800);
/// The approval wait when the settings cannot be read (the config's default).
const APPROVAL_WAIT: u64 = 120;

/// An error from the library, as a sentence.
fn sentence(text: &str) -> String {
    let mut chars = text.trim().chars();
    chars.next().map_or_else(String::new, |first| first.to_uppercase().chain(chars).collect())
}

/// The card's state at start: in the demo, as `REINS_DEMO_SESSION` says.
pub(super) fn initial(demo: Option<Demo>) -> Work {
    let start = demo.and_then(|_| DemoStart::parse(&std::env::var("REINS_DEMO_SESSION").unwrap_or_default()));
    Work {
        demo: demo.filter(|_| start == Some(DemoStart::Running)).map(Demo::work_session),
        demo_start: start.filter(|s| *s != DemoStart::Running),
        ..Work::default()
    }
}

impl Model {
    /// The session running on this computer now.
    #[must_use]
    pub fn work_session(&self) -> Option<&Session> {
        let now = reins_desktop::now_unix();
        self.snapshot.as_ref().and_then(|s| work::running(s.work_session.as_ref(), now))
    }

    /// For each new snapshot: in the demo the pretend session stands in for the file; once no session runs, an open
    /// "End the session?" and an error from ending go with it.
    pub(super) fn fill_work_session(&mut self, snapshot: &mut Snapshot, now: i64, cx: &mut Context<'_, Self>) {
        if let Some(start) = self.work.demo_start.take() {
            let repos = snapshot
                .config
                .as_ref()
                .map(|c| work::recent_repos(&snapshot.overview.connections, &snapshot.activity, c))
                .unwrap_or_default();
            self.work.form = Some(work::demo_form(repos));
            if start == DemoStart::Waiting {
                self.start_work_session(cx);
            }
        }
        if self.args.demo {
            if self.work.demo.as_ref().is_some_and(|s| s.left(now) == 0) {
                self.work.demo = None;
            }
            snapshot.work_session.clone_from(&self.work.demo);
        }
        if work::running(snapshot.work_session.as_ref(), now).is_none() {
            self.work.confirm_end = false;
            self.work.end_error = None;
        }
    }

    /// Shows `session` as the running one now, before the next snapshot reads it.
    fn show_work_session(&mut self, session: Option<Session>) {
        if let Some(snapshot) = self.snapshot.as_mut() {
            Arc::make_mut(snapshot).work_session = session;
        }
        self.refresh_tray();
    }

    /// "Start" on the work session card: the form, with the repositories this computer used lately.
    pub fn open_work_form(&mut self, cx: &mut Context<'_, Self>) {
        let repos = self
            .snapshot
            .as_ref()
            .and_then(|s| {
                let config = s.config.as_ref().ok()?;
                Some(work::recent_repos(&s.overview.connections, &s.activity, config))
            })
            .unwrap_or_default();
        self.work.form = Some(Form::new(repos));
        self.work.failed = None;
        cx.notify();
    }

    pub fn close_work_form(&mut self, cx: &mut Context<'_, Self>) {
        if self.work.asking.is_none() {
            self.work.form = None;
            self.work.failed = None;
            cx.notify();
        }
    }

    pub fn set_work_length(&mut self, length: Length, cx: &mut Context<'_, Self>) {
        if let Some(form) = self.work.form.as_mut() {
            form.length = length;
            cx.notify();
        }
    }

    pub fn toggle_work_read(&mut self, id: &'static str, cx: &mut Context<'_, Self>) {
        if let Some(form) = self.work.form.as_mut() {
            form.toggle_read(id);
            cx.notify();
        }
    }

    /// Back to the form after the phone did not start a session.
    pub fn edit_work_request(&mut self, cx: &mut Context<'_, Self>) {
        self.work.failed = None;
        cx.notify();
    }

    /// "Ask my phone" (and "Try again"): asks for the session the form describes and waits for the answer.
    pub fn start_work_session(&mut self, cx: &mut Context<'_, Self>) {
        if self.work.asking.is_some() || self.work_session().is_some() {
            return;
        }
        let Some(request) = self.work.form.as_ref().and_then(Form::request) else {
            return;
        };
        let timeout_secs = self
            .snapshot
            .as_ref()
            .and_then(|s| s.config.as_ref().ok())
            .map_or(APPROVAL_WAIT, |c| c.approval_timeout_secs);
        let since = Instant::now();
        self.work.asking = Some(Asking {
            since,
            timeout_secs,
            request: request.clone(),
        });
        self.work.failed = None;
        self.notice = None;
        self.note = None;
        cx.notify();
        // The countdown.
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_secs(1)).await;
                let asking = this.update(cx, |m, cx| {
                    let asking = m.work.asking.as_ref().is_some_and(|a| a.since == since);
                    if asking {
                        cx.notify();
                    }
                    asking
                });
                if !asking.unwrap_or(false) {
                    break;
                }
            }
        })
        .detach();
        let demo = self.args.demo;
        let backend = Arc::clone(&self.backend);
        cx.spawn(async move |this, cx| {
            let started = if demo {
                let slowly =
                    std::env::var("REINS_DEMO_SESSION").is_ok_and(|v| DemoStart::parse(&v) == Some(DemoStart::Waiting));
                cx.background_executor()
                    .timer(if slowly {
                        DEMO_APPROVE_SLOWLY
                    } else {
                        DEMO_APPROVE
                    })
                    .await;
                Ok(Demo::approve(&request, reins_desktop::now_unix()))
            } else {
                Tokio::spawn(cx, async move { backend.start_work_session(request).await })
                    .await
                    .unwrap_or_else(|e| Err(e.to_string()))
            };
            let _gone = this.update(cx, |m, cx| {
                m.work.asking = None;
                match started {
                    Ok(session) => {
                        log::info!("work session started: {} ({} grants)", session.reason, session.grants.len());
                        m.work.form = None;
                        if m.args.demo {
                            m.work.demo = Some(session.clone());
                        }
                        m.show_work_session(Some(session));
                        m.refresh_now(cx);
                    }
                    Err(e) => m.work.failed = Some(sentence(&e)),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// "End now": asks first.
    pub fn confirm_end_work_session(&mut self, cx: &mut Context<'_, Self>) {
        self.work.confirm_end = true;
        self.work.end_error = None;
        cx.notify();
    }

    /// "Keep": the session goes on.
    pub fn keep_work_session(&mut self, cx: &mut Context<'_, Self>) {
        self.work.confirm_end = false;
        cx.notify();
    }

    /// Ends the running session now (the card's "End", the tray's "End work session"): its permissions end on the
    /// phone.
    pub fn end_work_session(&mut self, cx: &mut Context<'_, Self>) {
        if self.work.ending || self.work_session().is_none() {
            return;
        }
        self.work.ending = true;
        self.work.confirm_end = false;
        self.work.end_error = None;
        self.notice = None;
        self.note = None;
        self.refresh_tray();
        cx.notify();
        let demo = self.args.demo;
        let grants = self.work_session().map_or(0, |s| s.grants.len());
        let backend = Arc::clone(&self.backend);
        cx.spawn(async move |this, cx| {
            let ended = if demo {
                cx.background_executor().timer(DEMO_END).await;
                Ok(grants)
            } else {
                Tokio::spawn(cx, async move { backend.end_work_session().await })
                    .await
                    .unwrap_or_else(|e| Err(e.to_string()))
            };
            let _gone = this.update(cx, |m, cx| {
                m.work.ending = false;
                match ended {
                    Ok(n) => {
                        if m.args.demo {
                            m.work.demo = None;
                        }
                        m.show_work_session(None);
                        m.note = Some(match n {
                            0 => "The work session had already ended.".to_owned(),
                            n => format!(
                                "Work session ended ({} ended on your phone).",
                                crate::format::count(u64::try_from(n).unwrap_or(u64::MAX), "permission", "permissions")
                            ),
                        });
                        m.refresh_now(cx);
                    }
                    Err(e) => m.work.end_error = Some(sentence(&e)),
                }
                m.refresh_tray();
                cx.notify();
            });
        })
        .detach();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errors_read_as_sentences() {
        assert_eq!(sentence("denied on your phone: not now"), "Denied on your phone: not now");
        assert_eq!(sentence("  "), "");
    }
}
