use std::fs;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn constants_example_stdout_exact() {
    let exe = env!("CARGO_BIN_EXE_vibelang");
    let src_path = format!(
        "{}/examples/misc/constants.vc",
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
    assert_eq!(stdout, "1\n3.1415e+0\n1.0.1-alpha\nfalse\n");
}

#[test]
fn const_reassignment_is_rejected() {
    let exe = env!("CARGO_BIN_EXE_vibelang");
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let tmp_dir = std::env::temp_dir().join(format!("vibelang_const_reassign_{unique}"));
    fs::create_dir_all(&tmp_dir).expect("mkdir");

    let src = r#"
const PI: Float = 3.1415;
func main() {
    PI = 2.71;
}
"#;
    let src_path = tmp_dir.join("main.vc");
    fs::write(&src_path, src).expect("write source");

    let output = Command::new(exe)
        .arg(src_path.to_string_lossy().to_string())
        .output()
        .expect("run vibelang");

    let _ = fs::remove_dir_all(&tmp_dir);

    assert!(
        !output.status.success(),
        "expected failure, got success with stdout={}",
        String::from_utf8_lossy(&output.stdout)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("cannot assign to constant `PI`"),
        "stderr missing const reassignment message: {stderr}"
    );
}

#[test]
fn exported_const_can_be_imported() {
    let exe = env!("CARGO_BIN_EXE_vibelang");
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let tmp_dir = std::env::temp_dir().join(format!("vibelang_export_const_{unique}"));
    fs::create_dir_all(&tmp_dir).expect("mkdir");

    fs::write(
        tmp_dir.join("m.vc"),
        "const A: Int = 41;\nexport A;\n",
    )
    .expect("write module");
    fs::write(
        tmp_dir.join("main.vc"),
        "import { A } from \"m\";\nfunc main() { let v = A; }\n",
    )
    .expect("write main");

    let output = Command::new(exe)
        .arg(tmp_dir.join("main.vc").to_string_lossy().to_string())
        .output()
        .expect("run vibelang");

    let _ = fs::remove_dir_all(&tmp_dir);

    assert!(
        output.status.success(),
        "process failed: status={:?}, stderr={}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
}
