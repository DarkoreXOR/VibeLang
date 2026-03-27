use std::process::Command;

#[test]
fn example31_stdout_exact() {
    let exe = env!("CARGO_BIN_EXE_vibelang");
    let src_path = format!(
        "{}/examples/collections/example31.vc",
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
    let expected = "\
true
false
Option::Some(5.e+0)
Option::Some(1.23456e+2)
Option::None
no old value
0085070e-49a2-4b5e-b0d4-955dfe06579d
0085070e-49a2-4b5e-b0d4-955dfe06579d
no old value
";
    assert_eq!(stdout, expected);
}

