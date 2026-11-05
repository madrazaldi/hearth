//! Update checking and installing, for Settings › General and the quiet
//! checks at startup and every few hours.

use super::Hearth;
use crate::platform::updater::{Progress, Updater};
use gpui_kit::{Context, WeakEntity};
use std::time::Duration;

/// IRC clients stay open for weeks, so a check at launch is not enough.
pub const RECHECK: Duration = Duration::from_secs(6 * 60 * 60);

#[derive(Clone, Debug, PartialEq)]
pub enum UpdatePhase {
    Idle,
    Checking,
    Current,
    Available(String),
    Installing { version: String, progress: Option<f32> },
    Error(String),
}

pub struct Updates {
    updater: Updater,
    pub phase: UpdatePhase,
    /// A quietly found version nobody has been told about yet.
    pending: Option<String>,
    announced: Option<String>,
    /// Fakes the answers (`HEARTH_UPDATE=available|error`), for the mock backend.
    scripted: bool,
}

impl Updates {
    pub fn new(scripted: bool) -> Self {
        Updates { updater: Updater::new(), phase: UpdatePhase::Idle, pending: None, announced: None, scripted }
    }

    /// Why updates cannot work here, shown instead of a button that would fail.
    pub fn unavailable_reason(&self) -> Option<String> {
        if self.scripted {
            return None;
        }
        self.updater.unavailable_reason().map(String::from)
    }

    fn busy(&self) -> bool {
        matches!(self.phase, UpdatePhase::Checking | UpdatePhase::Installing { .. })
    }

    fn offered(&self) -> Option<String> {
        match &self.phase {
            UpdatePhase::Available(v) => Some(v.clone()),
            _ => None,
        }
    }
}

impl Hearth {
    /// Check now and again every few hours while "check for updates" is on.
    pub(super) fn start_update_checks(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this: WeakEntity<Hearth>, cx| {
            loop {
                let alive = this
                    .update(cx, |h, cx| {
                        if h.app.settings.check_updates {
                            h.check_for_update(true, cx);
                        }
                    })
                    .is_ok();
                if !alive {
                    break;
                }
                cx.background_executor().timer(RECHECK).await;
            }
        })
        .detach();
    }

    /// `quiet` swallows errors and announces a new version once with a toast.
    pub fn check_for_update(&mut self, quiet: bool, cx: &mut Context<Self>) {
        if self.updates.busy() {
            return;
        }
        self.updates.phase = UpdatePhase::Checking;
        cx.notify();
        let scripted = self.updates.scripted;
        let updater = self.updates.updater.clone();
        cx.spawn(async move |this: WeakEntity<Hearth>, cx| {
            let found: Result<Option<String>, String> = if scripted {
                cx.background_executor().timer(Duration::from_millis(1200)).await;
                match std::env::var("HEARTH_UPDATE").as_deref() {
                    Ok("available") => Ok(Some("0.2.0".into())),
                    Ok("error") => Err("signature check failed".into()),
                    _ => Ok(None),
                }
            } else {
                updater.check().await.map(|o| o.map(|o| o.version)).map_err(|e| format!("{e:#}"))
            };
            let _ = this.update(cx, |h, cx| {
                let u = &mut h.updates;
                match found {
                    Ok(None) => u.phase = UpdatePhase::Current,
                    Ok(Some(v)) => {
                        if quiet && u.announced.as_deref() != Some(v.as_str()) {
                            u.pending = Some(v.clone());
                        }
                        u.phase = UpdatePhase::Available(v);
                        h.announce_update();
                    }
                    // A failed background recheck should not hide an update that was already found.
                    Err(e) if quiet => {
                        u.phase = u.offered().map(UpdatePhase::Available).unwrap_or(UpdatePhase::Idle);
                        log::warn!("update check failed: {e}");
                    }
                    Err(e) => u.phase = UpdatePhase::Error(e),
                }
                h.pump(cx);
                cx.notify();
            });
        })
        .detach();
    }

    /// Toast a quietly found update, unless the first-run card is up (the toast would cover it).
    pub fn announce_update(&mut self) {
        if self.app.welcoming() {
            return;
        }
        if let Some(v) = self.updates.pending.take() {
            self.updates.announced = Some(v.clone());
            self.app.toast(format!("Hearth {v} is available. Install it from Settings \u{203a} General."), hearth_model::state::ToastKind::Info);
        }
    }

    pub fn install_update(&mut self, cx: &mut Context<Self>) {
        let Some(version) = self.updates.offered() else { return };
        self.updates.phase = UpdatePhase::Installing { version: version.clone(), progress: None };
        cx.notify();
        let scripted = self.updates.scripted;
        let updater = self.updates.updater.clone();
        let (tx, rx) = async_channel::unbounded::<Progress>();
        cx.spawn({
            let version = version.clone();
            async move |this: WeakEntity<Hearth>, cx| {
                while let Ok(p) = rx.recv().await {
                    let progress = p.total.filter(|t| *t > 0).map(|t| (p.downloaded as f32 / t as f32).min(1.));
                    if this.update(cx, |h, cx| h.set_install_progress(&version, progress, cx)).is_err() {
                        break;
                    }
                }
            }
        })
        .detach();
        cx.spawn(async move |this: WeakEntity<Hearth>, cx| {
            let done: Result<(), String> = if scripted {
                for step in 0..=10u64 {
                    let _ = tx.try_send(Progress { downloaded: step, total: Some(10) });
                    cx.background_executor().timer(Duration::from_millis(250)).await;
                }
                Ok(())
            } else {
                let sender = tx.clone();
                match updater
                    .download(move |p| {
                        let _ = sender.try_send(p);
                    })
                    .await
                {
                    // Applying restarts the app by itself.
                    Ok(()) => updater.apply_and_relaunch(cx).await.map_err(|e| format!("{e:#}")),
                    Err(e) => Err(format!("{e:#}")),
                }
            };
            drop(tx);
            let _ = this.update(cx, |h, cx| {
                // Where applying did not restart us, the version is now the running one.
                h.updates.phase = match done {
                    Ok(()) => UpdatePhase::Current,
                    Err(e) => UpdatePhase::Error(e),
                };
                cx.notify();
            });
        })
        .detach();
    }

    fn set_install_progress(&mut self, version: &str, progress: Option<f32>, cx: &mut Context<Self>) {
        if matches!(self.updates.phase, UpdatePhase::Installing { .. }) {
            self.updates.phase = UpdatePhase::Installing { version: version.to_string(), progress };
            cx.notify();
        }
    }
}
