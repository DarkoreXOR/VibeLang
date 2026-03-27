use std::process::Command;

#[test]
fn fetch_example_stdout_relaxed() {
    let exe = env!("CARGO_BIN_EXE_vibelang");
    let src_path = format!(
        "{}/examples/network/fetch.vc",
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
    assert!(
        stdout.contains("status: 200"),
        "expected HTTP 200 in output, got: {}",
        stdout
    );
    assert!(
        stdout.contains("postman-echo.com/get"),
        "expected endpoint in output, got: {}",
        stdout
    );
    assert!(
        stdout.contains("\"url\"") && stdout.contains("\"args\""),
        "expected JSON markers in output, got: {}",
        stdout
    );
}

