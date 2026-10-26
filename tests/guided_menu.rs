//! Real terminal coverage without an application-only testing mode.
#[cfg(unix)]
#[test]
fn guided_menu_in_a_real_terminal() {
    let status = std::process::Command::new("python3")
        .arg("tests/menu_pty.py")
        .env("SUBREPO_TEST_BINARY", env!("CARGO_BIN_EXE_git-subrepo"))
        .status()
        .expect("Python 3 is required for the Unix terminal integration test");
    assert!(status.success(), "guided terminal integration failed");
}
