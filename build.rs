use std::env;
use std::process::Command;

fn main() {
    println!("cargo::rerun-if-changed=build.rs");
    println!("cargo::rerun-if-env-changed=HERDR_GRID_SLIDE_RELEASE");

    let version = env::var("CARGO_PKG_VERSION").unwrap();
    let version = if env::var("HERDR_GRID_SLIDE_RELEASE").as_deref() == Ok("1") {
        version
    } else {
        for name in ["HEAD", "refs"] {
            let path = git(&["rev-parse", "--git-path", name]);
            println!("cargo::rerun-if-changed={path}");
        }
        let sha = git(&["rev-parse", "--short=12", "HEAD"]);
        format!("{version}-dev+{sha}")
    };
    println!("cargo::rustc-env=HERDR_GRID_SLIDE_VERSION={version}");
}

fn git(args: &[&str]) -> String {
    let output = Command::new("git")
        .args(args)
        .output()
        .expect("git is required to identify a development build");
    assert!(
        output.status.success(),
        "git failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}
