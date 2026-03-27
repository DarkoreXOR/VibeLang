use std::process::Command;

#[test]
fn example30_stdout_exact() {
    let exe = env!("CARGO_BIN_EXE_vibelang");
    let src_path = format!(
        "{}/examples/async/example30.vc",
        env!("CARGO_MANIFEST_DIR").replace('\\', "/")
    );

    let output = Command::new(exe)
        .arg(src_path)
        .output()
        .expect("failed to run vibelang binary");

    assert!(
        output.status.success(),
        "process failed: status={:?}, stderr={}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout).replace("\r\n", "\n");
    assert_eq!(stdout, "hello\n()\n");
}

