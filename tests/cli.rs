//! CLI integration tests: exit codes and report files.

use std::path::Path;
use std::process::Command;

fn write(dir: &Path, rel: &str, body: &str) {
    let path = dir.join(rel);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(path, body).unwrap();
}

fn fixture(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    // A two-file import cycle produces at least one `cycle` warning.
    write(
        &dir,
        "src/a.ts",
        "import { b } from './b';\nexport function a() { return b(); }\n",
    );
    write(
        &dir,
        "src/b.ts",
        "import { a } from './a';\nexport function b() { return a(); }\n",
    );
    dir
}

#[test]
fn cli_writes_reports_and_exits_zero() {
    let dir = fixture("cg_cli_ok");
    let json = dir.join("report.json");
    let md = dir.join("report.md");
    let status = Command::new(env!("CARGO_BIN_EXE_cg_index"))
        .arg(&dir)
        .arg("--json")
        .arg(&json)
        .arg("--markdown")
        .arg(&md)
        .arg("--no-git")
        .status()
        .expect("run cg_index");
    assert!(status.success(), "exit: {status:?}");
    assert!(json.exists(), "json report should exist");
    let text = std::fs::read_to_string(&md).unwrap();
    assert!(text.contains("## Modules"));
    assert!(text.contains("cycle"), "cycle should be reported: {text}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn cli_exits_two_on_new_warnings_versus_baseline() {
    let dir = fixture("cg_cli_baseline");
    let baseline = dir.join("baseline.json");
    // An empty baseline: every current warning counts as new.
    std::fs::write(
        &baseline,
        r#"{"created":"0","warnings":[],"module_metrics":{}}"#,
    )
    .unwrap();

    let status = Command::new(env!("CARGO_BIN_EXE_cg_index"))
        .arg(&dir)
        .arg("--no-git")
        .arg("--baseline")
        .arg(&baseline)
        .arg("--fail-on-new")
        .arg("warn")
        .status()
        .expect("run cg_index");
    assert_eq!(status.code(), Some(2), "expected exit 2");

    // Without --fail-on-new the same comparison exits 0.
    let status = Command::new(env!("CARGO_BIN_EXE_cg_index"))
        .arg(&dir)
        .arg("--no-git")
        .arg("--baseline")
        .arg(&baseline)
        .status()
        .expect("run cg_index");
    assert!(status.success(), "exit: {status:?}");

    let _ = std::fs::remove_dir_all(&dir);
}
