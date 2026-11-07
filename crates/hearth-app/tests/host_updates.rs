//! The updater state the General settings page shows, against the scripted
//! updater (`HEARTH_UPDATE` picks its answer). A test binary of its own, since
//! it sets an environment variable.

use gpui_kit::TestAppContext;
use hearth_app::boot::init_ui;
use hearth_app::host::updates::UpdatePhase;
use hearth_app::host::{Boot, Hearth, hearth, host};
use hearth_model::mock::MockOptions;
use std::time::Duration;

fn check(cx: &mut TestAppContext, answer: &str, quiet: bool) -> UpdatePhase {
    // SAFETY: this binary runs this one test, so nothing reads the environment concurrently.
    unsafe { std::env::set_var("HEARTH_UPDATE", answer) };
    cx.update(|app| hearth(app).update(app, |h, cx| h.check_for_update(quiet, cx)));
    cx.executor().advance_clock(Duration::from_secs(2));
    cx.run_until_parked();
    cx.update(|app| host(app).updates.phase.clone())
}

#[test]
fn a_failed_background_recheck_keeps_the_update_it_already_found() {
    let mut cx = TestAppContext::single();
    cx.update(|app| {
        init_ui(app);
        Hearth::install(Boot::Mock(MockOptions::default()), false, app);
    });
    // Let the check at startup finish first.
    cx.executor().advance_clock(Duration::from_secs(2));
    cx.run_until_parked();

    assert_eq!(check(&mut cx, "available", false), UpdatePhase::Available("0.2.0".into()));
    assert_eq!(check(&mut cx, "error", true), UpdatePhase::Available("0.2.0".into()), "the Install button stays");
    assert_eq!(check(&mut cx, "error", false), UpdatePhase::Error("signature check failed".into()), "a check the user asked for reports the error");
}
