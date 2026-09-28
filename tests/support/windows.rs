use super::*;
use std::os::windows::fs::OpenOptionsExt;
use std::process::Stdio;
use std::time::Instant;

#[test]
fn applying_local_windows_builds_replaces_and_reloads_only_after_success() {
    let herdr = FakeHerdr::new(tab_snapshot(vec![]));
    let project = herdr.directory.join("local plugin with spaces");
    fs::create_dir_all(project.join(".mise/tasks")).unwrap();
    fs::create_dir_all(project.join("tools")).unwrap();
    fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join(".mise/tasks/apply-local.ps1"),
        project.join(".mise/tasks/apply-local.ps1"),
    )
    .unwrap();
    fs::write(
        project.join("tools/cargo.cmd"),
        "@echo off\r\nif not \"%*\"==\"build --locked --release\" exit /b 90\r\nif \"%FAIL_AT%\"==\"build\" exit /b 42\r\nif not exist target\\release mkdir target\\release\r\nif \"%FAIL_AT%\"==\"copy\" exit /b 0\r\ncopy /y \"%TEST_BINARY%\" target\\release\\herdr-grid-slide.exe >nul\r\n",
    )
    .unwrap();
    fs::write(
        project.join("tools/herdr.cmd"),
        "@echo off\r\nif not exist bin\\herdr-grid-slide.exe exit /b 91\r\necho %*>>calls\r\nif \"%FAIL_AT%\"==\"%*\" exit /b 43\r\n",
    )
    .unwrap();
    let run = |failure: &str| {
        ProcessCommand::new("powershell.exe")
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-ExecutionPolicy",
                "Bypass",
                "-File",
            ])
            .arg(project.join(".mise/tasks/apply-local.ps1"))
            .current_dir(&herdr.directory)
            .env(
                "PATH",
                format!(
                    "{};{}",
                    project.join("tools").display(),
                    std::env::var("PATH").unwrap()
                ),
            )
            .env("TEST_BINARY", &herdr.program)
            .env("FAIL_AT", failure)
            .output()
            .unwrap()
    };
    let installed = project.join("bin/herdr-grid-slide.exe");
    for _ in 0..2 {
        let output = run("");
        assert!(output.status.success(), "{output:?}");
        assert_eq!(
            fs::read(&installed).unwrap(),
            fs::read(&herdr.program).unwrap()
        );
        fs::write(&installed, b"previous local build").unwrap();
    }
    assert_eq!(
        fs::read_to_string(project.join("calls"))
            .unwrap()
            .replace('\r', ""),
        "plugin link .\nserver reload-config\nplugin link .\nserver reload-config\n"
    );
    fs::remove_file(project.join("calls")).unwrap();
    fs::remove_file(project.join("target/release/herdr-grid-slide.exe")).unwrap();
    for failure in ["build", "copy"] {
        let output = run(failure);
        assert!(!output.status.success(), "{failure}: {output:?}");
        if failure == "build" {
            assert_eq!(output.status.code(), Some(42));
        }
        assert_eq!(fs::read(&installed).unwrap(), b"previous local build");
        assert!(!project.join("calls").exists());
    }
    // Windows denies replacement while a process holds the executable open.
    let locked = fs::OpenOptions::new()
        .read(true)
        .share_mode(1)
        .open(&installed)
        .unwrap();
    assert!(!run("").status.success());
    assert!(!project.join("calls").exists());
    assert_eq!(fs::read(&installed).unwrap(), b"previous local build");
    drop(locked);
    for (failure, calls) in [
        ("plugin link .", "plugin link .\n"),
        (
            "server reload-config",
            "plugin link .\nserver reload-config\n",
        ),
    ] {
        let output = run(failure);
        assert_eq!(output.status.code(), Some(43), "{output:?}");
        assert_eq!(
            fs::read_to_string(project.join("calls"))
                .unwrap()
                .replace('\r', ""),
            calls
        );
        fs::remove_file(project.join("calls")).unwrap();
    }
    assert_eq!(fs::read_dir(project.join("bin")).unwrap().count(), 1);
}

#[test]
fn an_action_waits_for_a_busy_herdr_connection_then_completes() {
    let herdr = FakeHerdr::new(tab_snapshot(vec![pane("pane-only", 0, 0, 100, 100)]));
    let socket_path = herdr.directory.join("busy.sock");
    let socket_name = socket_path.to_string_lossy();
    let listener = ListenerOptions::new()
        .name(
            socket_name
                .as_ref()
                .to_ns_name::<GenericNamespaced>()
                .unwrap(),
        )
        .nonblocking(ListenerNonblockingMode::Accept)
        .create_sync()
        .unwrap();
    // Occupy the sole pending instance before the server accepts it. Another
    // CreateFile would fail with ERROR_PIPE_BUSY until accept creates a new one.
    let occupied = LocalStream::connect(
        socket_name
            .as_ref()
            .to_ns_name::<GenericNamespaced>()
            .unwrap(),
    )
    .unwrap();
    let mut child = ProcessCommand::new(&herdr.program)
        .args(["move-tab-workspace", "next"])
        .env("HERDR_SOCKET_PATH", &socket_path)
        .env("HERDR_WORKSPACE_ID", "workspace-1")
        .env("HERDR_TAB_ID", "tab-main")
        .env("HERDR_PANE_ID", "pane-only")
        .env("HERDR_PLUGIN_STATE_DIR", &herdr.state_path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    thread::sleep(Duration::from_millis(200));
    assert!(
        child.try_wait().unwrap().is_none(),
        "action failed while Herdr was busy"
    );
    drop(listener.accept().unwrap());
    drop(occupied);

    let deadline = Instant::now() + Duration::from_secs(10);
    let connection = loop {
        match listener.accept() {
            Ok(connection) => break connection,
            Err(error) if error.kind() == ErrorKind::WouldBlock => {
                if Instant::now() >= deadline {
                    child.kill().unwrap();
                    let output = child.wait_with_output().unwrap();
                    panic!(
                        "action did not reconnect: {}",
                        String::from_utf8_lossy(&output.stderr)
                    );
                }
                thread::sleep(Duration::from_millis(1));
            }
            Err(error) => panic!("accept failed: {error}"),
        }
    };
    let request = serve_request(connection, &mut HashMap::new(), &herdr.snapshot);
    let run = Run {
        output: child.wait_with_output().unwrap(),
        requests: vec![request],
    };
    run.assert_success();
    assert_eq!(run.requests, [snapshot_call()]);
}

#[test]
fn installing_a_windows_release_runs_registered_actions_and_failed_updates_preserve_it() {
    let mut herdr = FakeHerdr::new(tab_snapshot(vec![pane("pane-only", 0, 0, 100, 100)]));
    let project = herdr.directory.join("plugin with spaces");
    let release = herdr.directory.join("release");
    fs::create_dir_all(project.join("scripts")).unwrap();
    fs::create_dir_all(&release).unwrap();
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    for file in ["herdr-plugin.toml", "scripts/build-plugin.ps1"] {
        fs::copy(root.join(file), project.join(file)).unwrap();
    }
    let asset = format!(
        "herdr-grid-slide-v{}-x86_64-pc-windows-msvc.exe",
        env!("CARGO_PKG_VERSION")
    );
    fs::copy(&herdr.program, release.join(&asset)).unwrap();
    let hash_output = ProcessCommand::new("powershell.exe")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "(Get-FileHash -LiteralPath $env:TEST_ASSET -Algorithm SHA256).Hash",
        ])
        .env("TEST_ASSET", release.join(&asset))
        .env_remove("PSModulePath")
        .output()
        .unwrap();
    assert!(
        hash_output.status.success(),
        "PowerShell Get-FileHash failed: status={}\nstdout: {}\nstderr: {}",
        hash_output.status,
        String::from_utf8_lossy(&hash_output.stdout),
        String::from_utf8_lossy(&hash_output.stderr)
    );
    let hash = String::from_utf8(hash_output.stdout)
        .unwrap()
        .trim()
        .to_owned();
    let checksum = format!("{hash}  {asset}\n");
    fs::write(release.join("SHA256SUMS"), &checksum).unwrap();

    let manifest = fs::read_to_string(project.join("herdr-plugin.toml"))
        .unwrap()
        .replace("\r\n", "\n");
    let build = manifest
        .split("[[build]]")
        .skip(1)
        .find(|section| section.starts_with("\nplatforms = [\"windows\"]"))
        .unwrap();
    let argv: Vec<String> = serde_json::from_str(
        build
            .lines()
            .find_map(|line| line.strip_prefix("command = "))
            .unwrap(),
    )
    .unwrap();
    // Preserve inherited module paths so installation also covers launches through Herdr from pwsh.
    let install = |architecture: &str| {
        ProcessCommand::new(&argv[0])
            .args(&argv[1..])
            .current_dir(&project)
            .env(
                "HERDR_GRID_SLIDE_RELEASE_BASE_URL",
                format!(
                    "file:///{}",
                    release
                        .to_string_lossy()
                        .replace('\\', "/")
                        .replace(' ', "%20")
                ),
            )
            .env("PROCESSOR_ARCHITECTURE", architecture)
            .env_remove("PROCESSOR_ARCHITEW6432")
            .output()
            .unwrap()
    };
    let installed = project.join("bin/herdr-grid-slide.exe");
    let assert_clean = || {
        assert_eq!(fs::read_dir(project.join("bin")).unwrap().count(), 1);
    };

    // Both first installation and replacing an existing executable must work.
    for _ in 0..2 {
        let output = install("AMD64");
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            fs::read(&installed).unwrap(),
            fs::read(&herdr.program).unwrap()
        );
        assert_clean();
    }
    // Herdr joins this relative program to the plugin directory; Rust appends .exe.
    let action = manifest
        .split("[[actions]]")
        .skip(1)
        .find(|section| section.starts_with("\nid = \"tab-to-next-workspace\""))
        .unwrap();
    let action: Vec<String> = serde_json::from_str(
        action
            .lines()
            .find_map(|line| line.strip_prefix("command = "))
            .unwrap(),
    )
    .unwrap();
    herdr.program = project.join(action[0].strip_prefix("./").unwrap());
    let args: Vec<_> = action[1..].iter().map(String::as_str).collect();
    let run = herdr.run("workspace-1", "tab-main", "pane-only", &args);
    run.assert_success();
    assert_eq!(run.requests, [snapshot_call()]);

    let original = fs::read(&installed).unwrap();
    for (checksums, contents, architecture) in [
        (checksum.clone(), b"corrupted asset".as_slice(), "AMD64"),
        (checksum.repeat(2), original.as_slice(), "AMD64"),
        (format!("invalid  {asset}\n"), original.as_slice(), "AMD64"),
        (String::new(), original.as_slice(), "AMD64"),
        (checksum.clone(), original.as_slice(), "ARM64"),
    ] {
        fs::write(release.join("SHA256SUMS"), checksums).unwrap();
        fs::write(release.join(&asset), contents).unwrap();
        let output = install(architecture);
        assert!(!output.status.success());
        assert_eq!(fs::read(&installed).unwrap(), original);
        assert_clean();
    }
    fs::remove_file(release.join(&asset)).unwrap();
    let output = install("AMD64");
    assert_eq!(
        output.status.code(),
        Some(37),
        "curl's missing-file status must propagate: {output:?}"
    );
    assert_eq!(fs::read(&installed).unwrap(), original);
    assert_clean();
}
