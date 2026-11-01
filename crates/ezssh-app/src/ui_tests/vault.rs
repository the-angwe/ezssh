//! Headless tests of the change password, recovery code / unlock, reset vault, font picker, X server dialog.

use egui_kittest::kittest::{NodeT, Queryable};

use super::*;
use crate::style::Theme;
use crate::views::{font_picker::FontPicker, xserver::XServerDialog};

const NEW_PASSWORD: &str = "Brand-new-secret-42";

/// Lock from the status bar and wait for the unlock screen.
fn lock(h: &mut Harness<'static, App>) {
    h.get_by_label("Lock vault").click();
    wait(h, 5, "unlock screen", |a| matches!(&a.screen, Screen::Unlock(u) if !u.first_run));
    // A new card area sizes itself before it takes clicks.
    h.run_steps(5);
}

/// Type the password on the unlock card and press Unlock.
fn unlock_with(h: &mut Harness<'static, App>, pw: &str) {
    type_into(h, 0, pw);
    h.get_by_label("Unlock").click();
}

/// Replace the text of password field `index` (click, select all, type).
fn retype(h: &mut Harness<'static, App>, index: usize, text: &str) {
    h.query_all_by_role(Role::PasswordInput).nth(index).expect("password field").click();
    h.run_steps(1);
    h.key_press_modifiers(Modifiers::COMMAND, egui::Key::A);
    h.run_steps(1);
    // No click here: it would drop the selection.
    h.query_all_by_role(Role::PasswordInput).nth(index).expect("password field").type_text(text);
    h.run_steps(2);
}

/// Click a button by label (a hover tooltip may repeat the label).
fn press(h: &mut Harness<'static, App>, label: &str) {
    h.get_by_role_and_label(Role::Button, label).click();
    h.run_steps(2);
}

fn shot_both(h: &mut Harness<'static, App>, name: &str) {
    // Only the visuals: `App::set_theme` would also store the setting,
    // which fails (with a toast) while the vault is locked.
    shot(h, &format!("{name}-dark"));
    crate::style::apply(&h.ctx, Theme::Light);
    shot(h, &format!("{name}-light"));
    crate::style::apply(&h.ctx, Theme::Dark);
    h.run_steps(2);
}

#[test]
fn change_password_refuses_the_old_one_afterwards() {
    let mut env = start();
    unlock_fresh(&mut env);
    let h = &mut env.h;
    h.state_mut().dialogs.change_password = Some(Default::default());
    h.run_steps(3);
    assert!(h.query_by_label("Change master password").is_some());
    assert!(h.get_by_label("Change password").accesskit_node().is_disabled(), "disabled until every field is filled");

    // A wrong current password is refused by the core.
    type_into(h, 0, "not my password");
    type_into(h, 1, NEW_PASSWORD);
    type_into(h, 2, NEW_PASSWORD);
    assert!(h.query_by_label("Password strength: Very strong").is_some(), "strength meter under the new password");
    shot_both(h, "change-password");
    h.get_by_label("Change password").click();
    wait(h, 15, "refusal", |a| a.dialogs.change_password.as_ref().is_some_and(|d| d.error().is_some()));
    assert!(h.query_by_label("incorrect master password").is_some());
    shot(h, "change-password-error");

    // Mismatch is caught before the call.
    retype(h, 0, PASSWORD);
    retype(h, 2, "something else");
    h.get_by_label("Change password").click();
    h.run_steps(2);
    assert!(h.query_by_label("New passwords do not match").is_some());
    retype(h, 2, NEW_PASSWORD);

    h.get_by_label("Change password").click();
    wait(h, 20, "changed", |a| a.dialogs.change_password.as_ref().is_some_and(|d| d.is_done()));
    assert!(h.query_by_label("Master password changed").is_some());
    shot_both(h, "change-password-done");
    // Escape is gated in the success state.
    h.key_press(egui::Key::Escape);
    h.run_steps(2);
    assert!(h.state().dialogs.change_password.is_some(), "Escape does not leave the success card");
    h.get_by_label("Unlock").click();
    wait(h, 5, "unlock screen", |a| matches!(&a.screen, Screen::Unlock(u) if !u.first_run));
    assert!(h.state().dialogs.change_password.is_none());

    unlock_with(h, PASSWORD);
    wait(h, 15, "old password refused", |a| matches!(&a.screen, Screen::Unlock(u) if u.error().is_some()));
    assert!(h.query_by_label("Incorrect master password").is_some());
    retype(h, 0, NEW_PASSWORD);
    h.get_by_label("Unlock").click();
    wait(h, 15, "unlocked with the new password", |a| matches!(a.screen, Screen::Main));
}

#[test]
fn recovery_code_generate_then_unlock_with_it() {
    let mut env = start();
    unlock_fresh(&mut env);
    let h = &mut env.h;

    // No code yet: the unlock screen has no recovery link.
    lock(h);
    wait(h, 5, "recovery probe", |a| matches!(&a.screen, Screen::Unlock(u) if !u.recovery_available()));
    h.run_steps(5);
    assert!(h.query_by_label("Use recovery code").is_none());
    assert!(h.query_by_label("Forgot password?").is_some());
    unlock_with(h, PASSWORD);
    wait(h, 15, "main", |a| matches!(a.screen, Screen::Main));

    h.state_mut().dialogs.recovery_code = Some(Default::default());
    wait(h, 10, "probed", |a| a.dialogs.recovery_code.as_ref().is_some_and(|d| !d.probing()));
    assert!(h.query_by_label("Generate code").is_some());
    assert!(h.query_by_label("I understand the existing recovery code will stop working as soon as the new one is generated.").is_none(), "fresh: no checkbox");
    shot_both(h, "recovery-code-fresh");
    type_into(h, 0, PASSWORD);
    h.get_by_label("Generate code").click();
    wait(h, 20, "code shown", |a| a.dialogs.recovery_code.as_ref().is_some_and(|d| d.code().is_some()));
    let code = h.state().dialogs.recovery_code.as_ref().and_then(|d| d.code()).unwrap().to_string();
    assert!(code.len() >= 24, "a 24-character code (with hyphens)");
    press(h, "Copy");
    assert!(h.query_by_label("Copied").is_some());
    shot_both(h, "recovery-code-shown");
    // Escape is gated while the code is on screen.
    h.key_press(egui::Key::Escape);
    h.run_steps(2);
    assert!(h.state().dialogs.recovery_code.is_some());

    // Regenerate asks for the "I understand" box.
    press(h, "Regenerate");
    let box_label = "I understand the existing recovery code will stop working as soon as the new one is generated.";
    assert!(h.query_by_label(box_label).is_some());
    assert!(h.query_by_label("A recovery code already exists. Generating a new one will invalidate the old one. A snapshot of the current vault will be saved before the change.").is_some());
    type_into(h, 0, PASSWORD);
    assert!(h.get_by_label("Generate code").accesskit_node().is_disabled(), "gated by the checkbox");
    shot(h, "recovery-code-replace");
    h.key_press(egui::Key::Escape);
    h.run_steps(2);
    assert!(h.state().dialogs.recovery_code.is_none(), "Escape closes outside the shown phase");

    // Lock: the link appears; unlock with the code.
    lock(h);
    wait(h, 5, "recovery probe", |a| matches!(&a.screen, Screen::Unlock(u) if u.recovery_available()));
    shot(h, "unlock-links");
    h.get_by_label("Use recovery code").click();
    h.run_steps(6);
    assert!(h.query_by_label("Enter the 24-character recovery code you saved. Hyphens, spaces, and case are ignored.").is_some());
    h.get_by_label("Recovery code").type_text("AAAA-BBBB");
    h.run_steps(2);
    shot_both(h, "recovery-unlock");
    let dialog_unlock = |h: &Harness<'static, App>| {
        h.query_all_by_label("Unlock").find(|n| n.rect().width() < 200.0).expect("the dialog's Unlock button").click();
    };
    dialog_unlock(h);
    let until = Instant::now() + Duration::from_secs(15);
    while h.query_by_label("Incorrect recovery code").is_none() {
        assert!(Instant::now() < until, "no refusal for a wrong code");
        h.step();
        std::thread::sleep(Duration::from_millis(15));
    }
    shot(h, "recovery-unlock-error");
    // Let the refusal settle (the dialog re-focuses the field next frame).
    h.run_steps(5);
    h.get_by_label("Recovery code").click();
    h.run_steps(1);
    h.key_press_modifiers(Modifiers::COMMAND, egui::Key::A);
    h.run_steps(1);
    h.get_by_label("Recovery code").type_text(&code.to_lowercase());
    h.run_steps(2);
    dialog_unlock(h);
    wait(h, 20, "unlocked with the recovery code", |a| matches!(a.screen, Screen::Main));
}

#[test]
fn reset_vault_from_the_unlock_screen() {
    let mut env = start();
    unlock_fresh(&mut env);
    let h = &mut env.h;
    let tab = open_local(h);
    lock(h);
    h.get_by_label("Forgot password?").click();
    h.run_steps(6);
    let reset_button = |h: &Harness<'static, App>| h.get_by_role_and_label(Role::Button, "Reset vault").accesskit_node().is_disabled();
    assert!(reset_button(h), "disabled until DELETE is typed");
    h.get_by_label("Confirmation").type_text("delete");
    h.run_steps(2);
    assert!(reset_button(h), "case matters");
    shot_both(h, "reset-vault");
    // Escape cancels before the reset.
    h.key_press(egui::Key::Escape);
    h.run_steps(2);
    assert!(matches!(&h.state().screen, Screen::Unlock(u) if u.reset_dialog().is_none()));

    h.get_by_label("Forgot password?").click();
    h.run_steps(6);
    h.get_by_label("Confirmation").type_text("DELETE");
    h.run_steps(2);
    assert!(!reset_button(h));
    press(h, "Reset vault");
    wait(h, 20, "reset done", |a| matches!(&a.screen, Screen::Unlock(u) if u.reset_dialog().is_some_and(|d| d.is_done())));
    assert!(h.query_by_label("Vault reset").is_some());
    shot_both(h, "reset-vault-done");
    h.key_press(egui::Key::Escape);
    h.run_steps(2);
    assert!(h.query_by_label("Vault reset").is_some(), "Escape is inert after the reset");
    h.get_by_label("Set a new master password").click();
    h.run_steps(3);
    assert!(matches!(&h.state().screen, Screen::Unlock(u) if u.first_run), "first-run screen");
    assert!(h.query_by_label("Set master password").is_some());
    assert!(h.state().tabs.get(tab).is_none(), "the dead tabs are gone");
    let status = h.state().core.block_on(|st| async move { ezssh_core::api::vault::vault_status(&st).await }).unwrap();
    assert_eq!(status, "uninitialized");
}

#[test]
fn font_picker_applies_live_and_persists() {
    let mut env = start();
    unlock_fresh(&mut env);
    let h = &mut env.h;
    let tab = open_local(h);
    let session_id = h.state().tabs.get(tab).unwrap().session.id;
    let start_size = h.state().terms[&tab].term.font_size();
    h.state_mut().dialogs.font_picker = Some(FontPicker::new(tab, Some(Pos2::new(600.0, 300.0))));
    h.run_steps(3);
    assert!(h.query_by_label("Font").is_some());
    shot_both(h, "font-picker");

    press(h, "Increase font size");
    press(h, "Increase font size");
    assert_eq!(h.state().terms[&tab].term.font_size(), start_size + 2.0, "applied live");

    // Pick another family from the list.
    h.get_by_role(Role::ComboBox).click();
    h.run_steps(3);
    let other = crate::font_presets::choices()[1].1;
    h.get_by_label(other).click();
    h.run_steps(3);
    assert_eq!(h.state().dialogs.font_picker.as_ref().map(|p| p.tab), Some(tab), "picking a family keeps the picker open");
    shot(h, "font-picker-changed");

    // Type a size into the box.
    h.get_by_label("Font size (pt)").click();
    h.run_steps(1);
    h.key_press_modifiers(Modifiers::COMMAND, egui::Key::A);
    h.get_by_label("Font size (pt)").type_text("17");
    h.key_press(egui::Key::Enter);
    h.run_steps(2);
    assert_eq!(h.state().terms[&tab].term.font_size(), 17.0);

    h.get_by_label("Save as session default").click();
    h.get_by_label("Done").click();
    h.run_steps(2);
    assert!(h.state().dialogs.font_picker.is_none());
    let want = other.to_string();
    wait(h, 10, "font saved", move |a| {
        a.core
            .block_on(|st| async move { ezssh_core::api::sessions::session_get(&st, session_id).await })
            .is_ok_and(|s| s.font_size == 17 && s.font_family == want)
    });
    wait(h, 10, "the tab's copy follows", move |a| a.tabs.get(tab).is_some_and(|t| t.session.font_size == 17));

    // Reopened: shows the live values; Escape closes without saving.
    h.state_mut().dialogs.font_picker = Some(FontPicker::new(tab, None));
    h.run_steps(3);
    press(h, "Decrease font size");
    assert_eq!(h.state().terms[&tab].term.font_size(), 16.0);
    h.key_press(egui::Key::Escape);
    h.run_steps(2);
    assert!(h.state().dialogs.font_picker.is_none());
    let saved = h.state().core.block_on(|st| async move { ezssh_core::api::sessions::session_get(&st, session_id).await }).unwrap();
    assert_eq!(saved.font_size, 17, "not saved without asking");
}

#[test]
fn xserver_dialog_connect_without_x11_and_cancel() {
    let mut env = start();
    unlock_fresh(&mut env);
    let h = &mut env.h;
    let tab = open_local(h);
    h.state_mut().dialogs.xserver = Some(XServerDialog::new(tab, "VcXsrv is not installed".into()));
    h.run_steps(3);
    assert!(h.query_by_label("VcXsrv is required for X11 forwarding").is_some());
    assert_eq!(h.query_by_label("Install VcXsrv").is_some(), cfg!(windows), "install only where the core can");
    shot_both(h, "xserver-missing");

    h.get_by_label("Continue without X11").click();
    h.run_steps(2);
    assert!(h.state().dialogs.xserver.is_none());
    wait(h, 15, "reconnected", move |a| a.tabs.get(tab).is_some_and(|t| t.status == TabStatus::Connected));

    // Cancel closes the tab, as in the webview.
    h.state_mut().dialogs.xserver = Some(XServerDialog::new(tab, "VcXsrv is not installed".into()));
    h.run_steps(3);
    h.get_by_label("Cancel").click();
    h.run_steps(3);
    assert!(h.state().dialogs.xserver.is_none());
    assert!(h.state().tabs.get(tab).is_none(), "tab closed");
}

/// Two tabs missing the X server: the second waits behind the first
/// instead of replacing it, and is shown once the first is answered.
#[test]
fn xserver_dialog_queues_a_second_tab() {
    let mut env = start();
    unlock_fresh(&mut env);
    let h = &mut env.h;
    let a = open_local(h);
    // A second tab of the same session (the sidebar collapsed on the first).
    let session = h.state().tabs.get(a).unwrap().session.clone();
    h.state_mut().open_session(session);
    let b = h.state().tabs.active.unwrap();
    h.run_steps(3);
    h.state_mut().dialogs.xserver = Some(XServerDialog::new(a, "VcXsrv is not installed".into()));
    h.state_mut().dialogs.xserver.as_mut().unwrap().add(b);
    h.state_mut().dialogs.xserver.as_mut().unwrap().add(a); // the shown tab is not queued twice
    h.run_steps(3);
    h.get_by_label("Continue without X11").click();
    h.run_steps(3);
    assert_eq!(h.state().dialogs.xserver.as_ref().map(|d| d.tab), Some(b), "the waiting tab is next");
    h.get_by_label("Cancel").click();
    h.run_steps(3);
    assert!(h.state().dialogs.xserver.is_none());
    assert!(h.state().tabs.get(b).is_none() && h.state().tabs.get(a).is_some());
}
