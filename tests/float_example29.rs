use std::process::Command;

use astro_float::{BigFloat, Consts, Radix, RoundingMode};

#[test]
fn float_example29_stdout_exact() {
    let exe = env!("CARGO_BIN_EXE_vibelang");
    let src_path = format!(
        "{}/examples/example29.vc",
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

    // Must match VM's parsing + arithmetic precision/rounding and print_gen formatting.
    let p = 1024;
    let mut cc = Consts::new().expect("astro-float constants cache");
    let f1 = BigFloat::parse("12345.7890", Radix::Dec, p, RoundingMode::ToEven, &mut cc);
    let f2 = BigFloat::parse("456.789", Radix::Dec, p, RoundingMode::ToEven, &mut cc);

    let sum = f1.add(&f2, p, RoundingMode::ToEven);
    let sub = f1.sub(&f2, p, RoundingMode::ToEven);
    let mul = f1.mul(&f2, p, RoundingMode::ToEven);
    let div = f1.div(&f2, p, RoundingMode::ToEven);

    let expected = format!(
        "{}\n{}\n{}\n{}\n",
        sum.to_string(),
        sub.to_string(),
        mul.to_string(),
        div.to_string()
    );

    assert_eq!(stdout, expected);
}

