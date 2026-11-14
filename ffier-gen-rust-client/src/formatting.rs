//! Format generated client source with rustfmt.

/// Run `rustfmt` on the generated source. Panics if `rustfmt` is not
/// available or if it fails (the generated code has a syntax error).
pub(crate) fn rustfmt(src: &str) -> String {
    use std::io::Write;
    use std::process::{Command, Stdio};

    let mut child = Command::new("rustfmt")
        .arg("--edition")
        .arg("2024")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to spawn rustfmt — is it installed?");

    child
        .stdin
        .take()
        .unwrap()
        .write_all(src.as_bytes())
        .expect("failed to write to rustfmt stdin");

    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "rustfmt failed on generated code:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("rustfmt produced non-UTF-8 output")
}
