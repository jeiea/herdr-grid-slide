use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::Value;

const HERDR_VERSION: &str = "herdr 0.8.2";
const PLUGIN_ID: &str = "jeiea.move-pane";
const POLL_INTERVAL: Duration = Duration::from_millis(25);
const READY_TIMEOUT: Duration = Duration::from_secs(10);
const ACTION_TIMEOUT: Duration = Duration::from_secs(10);

static SESSION_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[test]
#[ignore = "requires Herdr 0.8.2 and starts an isolated named session"]
fn move_right_swaps_panes_in_the_same_tab_and_keeps_focus() {
    let mut live = LiveHerdr::start().expect("start isolated Herdr session");
    let source = live
        .create_workspace("same-tab-source")
        .expect("create source workspace");
    let neighbor = live
        .split_pane(&source.root_pane_id, "right", false)
        .expect("split source tab");
    live.link_plugin().expect("link copied plugin");

    let before = live.snapshot().expect("capture pre-action snapshot");
    assert_eq!(
        pane_ids_in_tab(&before, &source.tab_id),
        BTreeSet::from([source.root_pane_id.clone(), neighbor.clone()])
    );
    assert!(
        pane_x(&before, &source.tab_id, &source.root_pane_id)
            < pane_x(&before, &source.tab_id, &neighbor)
    );

    live.invoke_action("move-right")
        .expect("invoke move-right and wait for its log");

    let after = live.snapshot().expect("capture post-action snapshot");
    assert_eq!(
        pane_ids_in_tab(&after, &source.tab_id),
        BTreeSet::from([source.root_pane_id.clone(), neighbor.clone()])
    );
    assert!(
        pane_x(&after, &source.tab_id, &source.root_pane_id)
            > pane_x(&after, &source.tab_id, &neighbor)
    );
    assert_eq!(focused_pane_id(&after), source.root_pane_id);
}

#[test]
#[ignore = "requires Herdr 0.8.2 and starts an isolated named session"]
fn move_right_crosses_into_the_next_tab_and_keeps_focus() {
    let mut live = LiveHerdr::start().expect("start isolated Herdr session");
    let source = live
        .create_workspace("tab-boundary-source")
        .expect("create source workspace");
    let moving = live
        .split_pane(&source.root_pane_id, "right", true)
        .expect("create focused moving pane");
    let destination = live
        .create_tab(&source.workspace_id, "tab-boundary-destination")
        .expect("create destination tab");
    live.focus_tab(&source.tab_id).expect("focus source tab");
    live.link_plugin().expect("link copied plugin");

    let before = live.snapshot().expect("capture pre-action snapshot");
    assert_eq!(focused_pane_id(&before), moving);

    live.invoke_action("move-right")
        .expect("invoke move-right and wait for its log");

    let after = live.snapshot().expect("capture post-action snapshot");
    assert_eq!(
        pane_ids_in_tab(&after, &source.tab_id),
        BTreeSet::from([source.root_pane_id.clone()])
    );
    assert_eq!(
        pane_ids_in_tab(&after, &destination.tab_id),
        BTreeSet::from([destination.root_pane_id.clone(), moving.clone()])
    );
    assert_eq!(focused_pane_id(&after), moving);
}

#[test]
#[ignore = "requires Herdr 0.8.2 and starts an isolated named session"]
fn move_down_crosses_into_the_next_workspaces_active_tab_and_keeps_focus() {
    let mut live = LiveHerdr::start().expect("start isolated Herdr session");
    let source = live
        .create_workspace("workspace-boundary-source")
        .expect("create source workspace");
    let moving = live
        .split_pane(&source.root_pane_id, "right", true)
        .expect("create focused moving pane");
    let destination = live
        .create_workspace("workspace-boundary-destination")
        .expect("create destination workspace");
    let active_destination = live
        .create_tab(&destination.workspace_id, "active-destination")
        .expect("create active destination tab");
    live.focus_workspace(&source.workspace_id)
        .expect("focus source workspace");
    live.link_plugin().expect("link copied plugin");

    let before = live.snapshot().expect("capture pre-action snapshot");
    let moving_terminal_id = pane(&before, &moving)["terminal_id"]
        .as_str()
        .expect("moving pane terminal ID")
        .to_string();
    let inactive_destination_panes = pane_ids_in_tab(&before, &destination.tab_id);
    let active_destination_panes = pane_ids_in_tab(&before, &active_destination.tab_id);
    assert_eq!(focused_pane_id(&before), moving);

    live.invoke_action("move-down")
        .expect("invoke move-down and wait for its log");

    let after = live.snapshot().expect("capture post-action snapshot");
    let moved = pane_by_terminal_id(&after, &moving_terminal_id);
    assert_eq!(moved["workspace_id"], destination.workspace_id);
    assert_eq!(moved["tab_id"], active_destination.tab_id);
    assert_eq!(moved["focused"], true);
    assert_eq!(focused_pane_id(&after), moved["pane_id"]);
    assert_eq!(
        pane_ids_in_tab(&after, &source.tab_id),
        BTreeSet::from([source.root_pane_id.clone()])
    );
    assert_eq!(
        pane_ids_in_tab(&after, &destination.tab_id),
        inactive_destination_panes
    );
    assert_eq!(
        pane_ids_in_tab(&after, &active_destination.tab_id).len(),
        active_destination_panes.len() + 1
    );
}

struct LiveHerdr {
    herdr: PathBuf,
    session: String,
    root: PathBuf,
    config_home: PathBuf,
    state_home: PathBuf,
    runtime_dir: PathBuf,
    config_path: PathBuf,
    plugin_root: PathBuf,
    plugin_binary: PathBuf,
    server: Option<Child>,
    last_snapshot: Option<Value>,
}

struct Fixture {
    workspace_id: String,
    tab_id: String,
    root_pane_id: String,
}

impl LiveHerdr {
    fn start() -> Result<Self, String> {
        let unique = unique_suffix();
        // Herdr repeats both components in its socket path, so keep their prefixes short.
        let name = format!("h-{unique}");
        let root = PathBuf::from("/tmp").join(&name);
        let config_home = root.join("config");
        let state_home = root.join("state");
        let runtime_dir = root.join("runtime");
        let config_path = root.join("herdr-config.toml");
        let plugin_root = root.join("plugin");
        let plugin_binary = plugin_root.join("bin/herdr-move-pane");
        let session = name;
        let herdr = PathBuf::from("herdr");

        fs::create_dir(&root)
            .map_err(|error| format!("create isolated root {}: {error}", root.display()))?;
        for directory in [
            &config_home,
            &state_home,
            &runtime_dir,
            &plugin_root.join("bin"),
        ] {
            fs::create_dir_all(directory).map_err(|error| {
                format!("create isolated directory {}: {error}", directory.display())
            })?;
        }
        fs::write(&config_path, "onboarding = false\n")
            .map_err(|error| format!("write isolated Herdr config: {error}"))?;

        fs::copy(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("herdr-plugin.toml"),
            plugin_root.join("herdr-plugin.toml"),
        )
        .map_err(|error| format!("copy plugin manifest: {error}"))?;
        fs::copy(
            Path::new(env!("CARGO_BIN_EXE_herdr-move-pane")),
            &plugin_binary,
        )
        .map_err(|error| format!("copy test plugin binary: {error}"))?;

        let mut live = Self {
            herdr,
            session,
            root,
            config_home,
            state_home,
            runtime_dir,
            config_path,
            plugin_root,
            plugin_binary,
            server: None,
            last_snapshot: None,
        };
        live.require_version()?;
        live.spawn_server()?;
        live.wait_until_ready()?;
        Ok(live)
    }

    fn require_version(&self) -> Result<(), String> {
        let output = self.run_raw(["--version"])?;
        let version = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if version != HERDR_VERSION {
            return Err(format!(
                "live smoke requires {HERDR_VERSION}, found {version:?}"
            ));
        }
        Ok(())
    }

    fn spawn_server(&mut self) -> Result<(), String> {
        let mut command = Command::new(&self.herdr);
        command
            .arg("--session")
            .arg(&self.session)
            .arg("server")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        self.apply_environment(&mut command);
        self.server = Some(
            command
                .spawn()
                .map_err(|error| format!("start Herdr server: {error}"))?,
        );
        Ok(())
    }

    fn wait_until_ready(&mut self) -> Result<(), String> {
        let deadline = Instant::now() + READY_TIMEOUT;
        let mut last_error = String::new();
        while Instant::now() < deadline {
            if let Some(status) = self
                .server
                .as_mut()
                .expect("server child")
                .try_wait()
                .map_err(|error| format!("inspect Herdr server: {error}"))?
            {
                let output = self
                    .server
                    .take()
                    .expect("server child")
                    .wait_with_output()
                    .map_err(|error| format!("read exited Herdr server output: {error}"))?;
                return Err(format!(
                    "Herdr server exited before ready: {status}\nstdout: {}\nstderr: {}",
                    String::from_utf8_lossy(&output.stdout),
                    String::from_utf8_lossy(&output.stderr)
                ));
            }
            match self.run_json(["api", "snapshot"]) {
                Ok(response) => {
                    self.last_snapshot = Some(response["result"]["snapshot"].clone());
                    return Ok(());
                }
                Err(error) => last_error = error,
            }
            std::thread::sleep(POLL_INTERVAL);
        }
        Err(format!("Herdr server did not become ready: {last_error}"))
    }

    fn link_plugin(&self) -> Result<(), String> {
        let path = self.plugin_root.to_string_lossy().into_owned();
        let response = self.run_json(["plugin", "link", path.as_str()])?;
        let plugin = &response["result"]["plugin"];
        if response["result"]["type"] != "plugin_linked" || plugin["plugin_id"] != PLUGIN_ID {
            return Err(format!("unexpected plugin link response: {response}"));
        }

        assert_same_path(
            plugin["manifest_path"]
                .as_str()
                .ok_or_else(|| format!("link response omitted manifest path: {response}"))?,
            self.plugin_root.join("herdr-plugin.toml"),
        )?;
        assert_same_path(
            plugin["plugin_root"]
                .as_str()
                .ok_or_else(|| format!("link response omitted plugin root: {response}"))?,
            &self.plugin_root,
        )?;

        let entrypoint = plugin["actions"]
            .as_array()
            .and_then(|actions| actions.iter().find(|action| action["id"] == "move-right"))
            .and_then(|action| action["command"].as_array())
            .and_then(|command| command.first())
            .and_then(Value::as_str)
            .ok_or_else(|| format!("link response omitted move-right entrypoint: {response}"))?;
        assert_same_path(self.plugin_root.join(entrypoint), &self.plugin_binary)
    }

    fn create_workspace(&self, label: &str) -> Result<Fixture, String> {
        let cwd = self.root.to_string_lossy().into_owned();
        let response = self.run_json([
            "workspace",
            "create",
            "--cwd",
            cwd.as_str(),
            "--label",
            label,
            "--focus",
        ])?;
        fixture_from_create_response(&response)
    }

    fn create_tab(&self, workspace_id: &str, label: &str) -> Result<Fixture, String> {
        let cwd = self.root.to_string_lossy().into_owned();
        let response = self.run_json([
            "tab",
            "create",
            "--workspace",
            workspace_id,
            "--cwd",
            cwd.as_str(),
            "--label",
            label,
            "--focus",
        ])?;
        Ok(Fixture {
            workspace_id: response_string(&response, &["result", "tab", "workspace_id"])?,
            tab_id: response_string(&response, &["result", "tab", "tab_id"])?,
            root_pane_id: response_string(&response, &["result", "root_pane", "pane_id"])?,
        })
    }

    fn split_pane(&self, pane_id: &str, direction: &str, focus: bool) -> Result<String, String> {
        let focus_flag = if focus { "--focus" } else { "--no-focus" };
        let response = self.run_json([
            "pane",
            "split",
            pane_id,
            "--direction",
            direction,
            focus_flag,
        ])?;
        response_string(&response, &["result", "pane", "pane_id"])
    }

    fn focus_tab(&self, tab_id: &str) -> Result<(), String> {
        self.run_json(["tab", "focus", tab_id]).map(|_| ())
    }

    fn focus_workspace(&self, workspace_id: &str) -> Result<(), String> {
        self.run_json(["workspace", "focus", workspace_id])
            .map(|_| ())
    }

    fn invoke_action(&self, action_id: &str) -> Result<(), String> {
        let response = self.run_json([
            "plugin", "action", "invoke", action_id, "--plugin", PLUGIN_ID,
        ])?;
        let log_id = response_string(&response, &["result", "log", "log_id"])?;
        let deadline = Instant::now() + ACTION_TIMEOUT;

        while Instant::now() < deadline {
            let response = self.run_json([
                "plugin", "log", "list", "--plugin", PLUGIN_ID, "--limit", "50",
            ])?;
            if let Some(log) = response["result"]["logs"]
                .as_array()
                .and_then(|logs| logs.iter().find(|log| log["log_id"] == log_id))
            {
                match log["status"].as_str() {
                    Some("running") => {}
                    Some("succeeded") => {
                        if log["finished_unix_ms"].as_u64().is_none()
                            || log["exit_code"].as_i64() != Some(0)
                        {
                            return Err(format!("successful action log is incomplete: {log}"));
                        }
                        return Ok(());
                    }
                    Some("failed") => return Err(format!("plugin action failed: {log}")),
                    status => {
                        return Err(format!("unknown plugin action status {status:?}: {log}"));
                    }
                }
            }
            std::thread::sleep(POLL_INTERVAL);
        }
        Err(format!(
            "plugin action {action_id} log {log_id} did not finish within {ACTION_TIMEOUT:?}"
        ))
    }

    fn snapshot(&mut self) -> Result<Value, String> {
        let response = self.run_json(["api", "snapshot"])?;
        let snapshot = response["result"]["snapshot"].clone();
        if !snapshot.is_object() {
            return Err(format!("snapshot response omitted snapshot: {response}"));
        }
        self.last_snapshot = Some(snapshot.clone());
        Ok(snapshot)
    }

    fn run_json<I, S>(&self, args: I) -> Result<Value, String>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<std::ffi::OsStr>,
    {
        let output = self.run_raw(args)?;
        serde_json::from_slice(&output.stdout).map_err(|error| {
            format!(
                "parse Herdr JSON: {error}\nstdout: {}\nstderr: {}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            )
        })
    }

    fn run_raw<I, S>(&self, args: I) -> Result<Output, String>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<std::ffi::OsStr>,
    {
        let mut command = Command::new(&self.herdr);
        command.arg("--session").arg(&self.session).args(args);
        self.apply_environment(&mut command);
        let output = command
            .output()
            .map_err(|error| format!("run Herdr CLI: {error}"))?;
        if !output.status.success() {
            return Err(format!(
                "Herdr CLI exited {}\nstdout: {}\nstderr: {}",
                output.status,
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            ));
        }
        Ok(output)
    }

    fn apply_environment(&self, command: &mut Command) {
        command
            .env("XDG_CONFIG_HOME", &self.config_home)
            .env("XDG_STATE_HOME", &self.state_home)
            .env("XDG_RUNTIME_DIR", &self.runtime_dir)
            .env("HERDR_CONFIG_PATH", &self.config_path)
            .env("HERDR_SESSION", &self.session);
    }

    fn cleanup(&mut self) {
        let mut stop = Command::new(&self.herdr);
        stop.args(["session", "stop", &self.session]);
        self.apply_environment(&mut stop);
        let stop_output = stop.output();

        let server_output = self.server.take().and_then(|mut server| {
            let deadline = Instant::now() + READY_TIMEOUT;
            while Instant::now() < deadline {
                match server.try_wait() {
                    Ok(Some(_)) => return server.wait_with_output().ok(),
                    Ok(None) => std::thread::sleep(POLL_INTERVAL),
                    Err(_) => break,
                }
            }
            let _ = server.kill();
            server.wait_with_output().ok()
        });

        let mut delete = Command::new(&self.herdr);
        delete.args(["session", "delete", &self.session]);
        self.apply_environment(&mut delete);
        let delete_output = delete.output();

        if std::thread::panicking() {
            eprintln!("live Herdr session: {}", self.session);
            if let Some(snapshot) = &self.last_snapshot {
                eprintln!("last live Herdr snapshot: {snapshot}");
            }
            print_command_output("session stop", stop_output);
            if let Some(output) = server_output {
                eprintln!(
                    "Herdr server stdout:\n{}\nHerdr server stderr:\n{}",
                    String::from_utf8_lossy(&output.stdout),
                    String::from_utf8_lossy(&output.stderr)
                );
            }
            print_command_output("session delete", delete_output);
        }

        if let Err(error) = fs::remove_dir_all(&self.root)
            && error.kind() != std::io::ErrorKind::NotFound
        {
            eprintln!(
                "failed to remove live Herdr root {}: {error}",
                self.root.display()
            );
        }
    }
}

impl Drop for LiveHerdr {
    fn drop(&mut self) {
        self.cleanup();
    }
}

fn unique_suffix() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock after Unix epoch")
        .as_nanos();
    let sequence = SESSION_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    format!("{}-{nanos}-{sequence}", std::process::id())
}

fn fixture_from_create_response(response: &Value) -> Result<Fixture, String> {
    Ok(Fixture {
        workspace_id: response_string(response, &["result", "workspace", "workspace_id"])?,
        tab_id: response_string(response, &["result", "tab", "tab_id"])?,
        root_pane_id: response_string(response, &["result", "root_pane", "pane_id"])?,
    })
}

fn response_string(response: &Value, path: &[&str]) -> Result<String, String> {
    let mut value = response;
    for key in path {
        value = value
            .get(key)
            .ok_or_else(|| format!("response omitted {}: {response}", path.join(".")))?;
    }
    value.as_str().map(str::to_string).ok_or_else(|| {
        format!(
            "response field {} is not a string: {response}",
            path.join(".")
        )
    })
}

fn assert_same_path(actual: impl AsRef<Path>, expected: impl AsRef<Path>) -> Result<(), String> {
    let actual = fs::canonicalize(actual.as_ref())
        .map_err(|error| format!("resolve path {}: {error}", actual.as_ref().display()))?;
    let expected = fs::canonicalize(expected.as_ref())
        .map_err(|error| format!("resolve path {}: {error}", expected.as_ref().display()))?;
    if actual != expected {
        return Err(format!(
            "linked path mismatch: expected {}, found {}",
            expected.display(),
            actual.display()
        ));
    }
    Ok(())
}

fn focused_pane_id(snapshot: &Value) -> String {
    snapshot["focused_pane_id"]
        .as_str()
        .expect("snapshot focused pane ID")
        .to_string()
}

fn pane<'a>(snapshot: &'a Value, pane_id: &str) -> &'a Value {
    snapshot["panes"]
        .as_array()
        .expect("snapshot pane list")
        .iter()
        .find(|pane| pane["pane_id"] == pane_id)
        .unwrap_or_else(|| panic!("pane {pane_id} missing from snapshot: {snapshot}"))
}

fn pane_by_terminal_id<'a>(snapshot: &'a Value, terminal_id: &str) -> &'a Value {
    snapshot["panes"]
        .as_array()
        .expect("snapshot pane list")
        .iter()
        .find(|pane| pane["terminal_id"] == terminal_id)
        .unwrap_or_else(|| panic!("terminal {terminal_id} missing from snapshot: {snapshot}"))
}

fn pane_ids_in_tab(snapshot: &Value, tab_id: &str) -> BTreeSet<String> {
    snapshot["panes"]
        .as_array()
        .expect("snapshot pane list")
        .iter()
        .filter(|pane| pane["tab_id"] == tab_id)
        .map(|pane| {
            pane["pane_id"]
                .as_str()
                .expect("snapshot pane ID")
                .to_string()
        })
        .collect()
}

fn pane_x(snapshot: &Value, tab_id: &str, pane_id: &str) -> u64 {
    snapshot["layouts"]
        .as_array()
        .expect("snapshot layout list")
        .iter()
        .find(|layout| layout["tab_id"] == tab_id)
        .and_then(|layout| layout["panes"].as_array())
        .and_then(|panes| panes.iter().find(|pane| pane["pane_id"] == pane_id))
        .and_then(|pane| pane["rect"]["x"].as_u64())
        .unwrap_or_else(|| panic!("pane {pane_id} layout missing from snapshot: {snapshot}"))
}

fn print_command_output(label: &str, output: std::io::Result<Output>) {
    match output {
        Ok(output) => eprintln!(
            "{label} status: {}\nstdout: {}\nstderr: {}",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ),
        Err(error) => eprintln!("{label} failed to run: {error}"),
    }
}
