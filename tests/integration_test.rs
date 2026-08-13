#![cfg(unix)]

use std::fs;
use std::io::{BufRead, BufReader, ErrorKind, Write};
use std::os::unix::net::UnixListener;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[test]
fn moves_wrap_by_visible_workspace_and_tab_number() {
    let integration = standard_integration();

    for (scope, direction, workspace_id, tab_id, target_tab_id) in [
        ("workspace", "next", "workspace-3", "tab-3-1", "tab-1-1"),
        ("workspace", "previous", "workspace-1", "tab-1-1", "tab-3-1"),
        ("tab", "next", "workspace-2", "tab-2-3", "tab-2-1"),
        ("tab", "previous", "workspace-2", "tab-2-1", "tab-2-3"),
    ] {
        let output = integration.run(workspace_id, tab_id, "pane-current", &[scope, direction]);
        assert_success(&output);
        assert_eq!(
            integration.call(),
            [
                "pane.move",
                "pane-current",
                target_tab_id,
                "right",
                "0.5",
                "true"
            ]
        );
    }
}

#[test]
fn focuses_each_geometric_neighbor_inside_a_tab() {
    for (direction, target_id, target_rect) in [
        ("left", "pane-left", (0, 40)),
        ("right", "pane-right", (80, 40)),
        ("up", "pane-up", (40, 0)),
        ("down", "pane-down", (40, 80)),
    ] {
        let integration = Integration::new(
            json!([layout(
                "tab-current",
                vec![
                    pane("pane-current", 40, 40, 40, 40),
                    pane(target_id, target_rect.0, target_rect.1, 40, 40),
                ],
            )]),
            json!([tab("workspace-1", "tab-current", 1)]),
            json!([workspace("workspace-1", "tab-current", 1)]),
        );

        let output = integration.run(
            "workspace-1",
            "tab-current",
            "pane-current",
            &["focus", direction],
        );
        assert_success(&output);
        assert_eq!(integration.call(), ["pane.focus", target_id]);
    }
}

#[test]
fn focus_wraps_across_tabs_and_workspaces_in_visual_order() {
    for (direction, workspace_id, tab_id, expected) in [
        ("left", "workspace-2", "tab-2-1", "pane-2-3-bottom-right"),
        ("right", "workspace-2", "tab-2-3", "pane-2-1-top-left"),
        ("up", "workspace-1", "tab-1-1", "pane-3-bottom-right"),
        ("down", "workspace-3", "tab-3-1", "pane-1-top-left"),
    ] {
        let mut layouts = standard_layouts();
        let current = layouts
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|layout| layout["tab_id"] == tab_id)
            .unwrap();
        *current = layout(tab_id, vec![pane("pane-current", 0, 0, 100, 80)]);
        let integration = Integration::new(layouts, standard_tabs(), standard_workspaces());

        let output = integration.run(workspace_id, tab_id, "pane-current", &["focus", direction]);
        assert_success(&output);
        assert_eq!(integration.call(), ["pane.focus", expected]);
    }
}

#[test]
fn focus_remembers_the_cross_axis_anchor_across_panes() {
    let integration = anchor_integration();

    for (pane_id, direction, expected) in [
        ("pane-a", "right", "pane-b"),
        ("pane-b", "down", "pane-c"),
        ("pane-c", "left", "pane-a"),
        ("pane-a", "right", "pane-c"),
    ] {
        let output = integration.run("workspace-1", "tab-anchor", pane_id, &["focus", direction]);
        assert_success(&output);
        assert_eq!(integration.call(), ["pane.focus", expected]);
    }

    let state_files: Vec<_> = fs::read_dir(&integration.state_path)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(state_files, ["focus-anchor.json"]);
}

#[test]
fn focus_uses_live_snapshot_context_instead_of_stale_environment() {
    let integration =
        anchor_integration().with_focused_context("workspace-1", "tab-anchor", "pane-b");

    let output = integration.run(
        "workspace-stale",
        "tab-stale",
        "pane-stale",
        &["focus", "down"],
    );

    assert_success(&output);
    assert_eq!(integration.call(), ["pane.focus", "pane-c"]);
}

#[test]
fn move_uses_live_snapshot_context_instead_of_stale_environment() {
    let integration =
        standard_integration().with_focused_context("workspace-2", "tab-2-1", "pane-live");

    let output = integration.run(
        "workspace-stale",
        "tab-stale",
        "pane-stale",
        &["tab", "next"],
    );

    assert_success(&output);
    assert_eq!(
        integration.call(),
        ["pane.move", "pane-live", "tab-2-2", "right", "0.5", "true"]
    );
}

#[test]
fn focus_resets_stale_or_malformed_anchor_state() {
    for state in [
        json!({
            "paneId": "pane-stale",
            "tabId": "tab-anchor",
            "workspaceId": "workspace-1",
            "x": 0.3,
            "y": 0.9,
        })
        .to_string(),
        "not json".to_owned(),
    ] {
        let integration = anchor_integration();
        integration.write_state(&state);

        let output = integration.run("workspace-1", "tab-anchor", "pane-a", &["focus", "right"]);
        assert_success(&output);
        assert_eq!(integration.call(), ["pane.focus", "pane-b"]);
    }
}

#[test]
fn focus_preserves_anchor_while_wrapping_across_tabs() {
    let integration = Integration::new(
        json!([
            layout("tab-source", vec![pane("pane-source", 0, 0, 100, 200)]),
            layout(
                "tab-target",
                vec![
                    pane("pane-top", 0, 0, 100, 50),
                    pane("pane-bottom", 0, 50, 100, 50),
                ],
            ),
        ]),
        json!([
            tab("workspace-1", "tab-source", 1),
            tab("workspace-1", "tab-target", 2),
        ]),
        json!([workspace("workspace-1", "tab-source", 1)]),
    );
    integration.write_state(
        &json!({
            "paneId": "pane-source",
            "tabId": "tab-source",
            "workspaceId": "workspace-1",
            "x": 0.5,
            "y": 0.75,
        })
        .to_string(),
    );

    let output = integration.run(
        "workspace-1",
        "tab-source",
        "pane-source",
        &["focus", "right"],
    );
    assert_success(&output);
    assert_eq!(integration.call(), ["pane.focus", "pane-bottom"]);
}

#[test]
fn focus_preserves_anchor_while_wrapping_across_workspaces() {
    let integration = Integration::new(
        json!([
            layout("tab-source", vec![pane("pane-source", 0, 0, 200, 100)]),
            layout(
                "tab-target",
                vec![
                    pane("pane-left", 0, 0, 50, 100),
                    pane("pane-right", 50, 0, 50, 100),
                ],
            ),
        ]),
        json!([
            tab("workspace-source", "tab-source", 1),
            tab("workspace-target", "tab-target", 1),
        ]),
        json!([
            workspace("workspace-source", "tab-source", 1),
            workspace("workspace-target", "tab-target", 2),
        ]),
    );
    integration.write_state(
        &json!({
            "paneId": "pane-source",
            "tabId": "tab-source",
            "workspaceId": "workspace-source",
            "x": 0.75,
            "y": 0.5,
        })
        .to_string(),
    );

    let output = integration.run(
        "workspace-source",
        "tab-source",
        "pane-source",
        &["focus", "down"],
    );
    assert_success(&output);
    assert_eq!(integration.call(), ["pane.focus", "pane-right"]);
}

struct Integration {
    calls_path: PathBuf,
    directory: PathBuf,
    layouts: Value,
    state_path: PathBuf,
    tabs: Value,
    workspaces: Value,
    focused_context: Option<(String, String, String)>,
}

impl Integration {
    fn new(layouts: Value, tabs: Value, workspaces: Value) -> Self {
        let directory = temp_dir();
        fs::create_dir_all(&directory).unwrap();
        Self {
            calls_path: directory.join("calls.txt"),
            state_path: directory.join("state"),
            directory,
            layouts,
            tabs,
            workspaces,
            focused_context: None,
        }
    }

    fn with_focused_context(mut self, workspace_id: &str, tab_id: &str, pane_id: &str) -> Self {
        self.focused_context = Some((
            workspace_id.to_owned(),
            tab_id.to_owned(),
            pane_id.to_owned(),
        ));
        self
    }

    fn run(&self, workspace_id: &str, tab_id: &str, pane_id: &str, args: &[&str]) -> Output {
        let _ = fs::remove_file(&self.calls_path);
        let socket_path = self.directory.join("herdr.sock");
        let mut snapshot = json!({
            "layouts": self.layouts,
            "tabs": self.tabs,
            "workspaces": self.workspaces,
        });
        if let Some((workspace_id, tab_id, pane_id)) = &self.focused_context {
            snapshot["focused_workspace_id"] = workspace_id.clone().into();
            snapshot["focused_tab_id"] = tab_id.clone().into();
            snapshot["focused_pane_id"] = pane_id.clone().into();
        }
        let server = self.start_server(&socket_path, snapshot);
        let output = Command::new(env!("CARGO_BIN_EXE_herdr-move-pane"))
            .args(args)
            .env("HERDR_PANE_ID", pane_id)
            .env("HERDR_PLUGIN_STATE_DIR", &self.state_path)
            .env("HERDR_SOCKET_PATH", &socket_path)
            .env("HERDR_TAB_ID", tab_id)
            .env("HERDR_WORKSPACE_ID", workspace_id)
            .output()
            .unwrap();
        server.join().unwrap();
        output
    }

    fn start_server(&self, socket_path: &PathBuf, snapshot: Value) -> thread::JoinHandle<()> {
        let _ = fs::remove_file(socket_path);
        let listener = UnixListener::bind(socket_path).unwrap();
        listener.set_nonblocking(true).unwrap();
        let calls_path = self.calls_path.clone();
        thread::spawn(move || {
            for request_number in 0..2 {
                let Some((connection, _)) = (0..200).find_map(|_| match listener.accept() {
                    Ok(connection) => Some(connection),
                    Err(error) if error.kind() == ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(10));
                        None
                    }
                    Err(error) => panic!("accept failed: {error}"),
                }) else {
                    return;
                };
                connection.set_nonblocking(false).unwrap();
                let mut reader = BufReader::new(connection.try_clone().unwrap());
                let mut writer = connection;
                let mut request = String::new();
                reader.read_line(&mut request).unwrap();
                let request: Value = serde_json::from_str(&request).unwrap();
                let result = match request["method"].as_str().unwrap() {
                    "session.snapshot" => {
                        json!({"type": "session_snapshot", "snapshot": snapshot})
                    }
                    "pane.focus" => {
                        let pane_id = request["params"]["pane_id"].as_str().unwrap();
                        fs::write(&calls_path, format!("pane.focus\n{pane_id}\n")).unwrap();
                        json!({"pane": {"pane_id": pane_id}})
                    }
                    "pane.move" => {
                        let params = &request["params"];
                        let destination = &params["destination"];
                        fs::write(
                            &calls_path,
                            format!(
                                "pane.move\n{}\n{}\n{}\n{}\n{}\n",
                                params["pane_id"].as_str().unwrap(),
                                destination["tab_id"].as_str().unwrap(),
                                destination["split"].as_str().unwrap(),
                                destination["ratio"],
                                params["focus"],
                            ),
                        )
                        .unwrap();
                        json!({"type": "pane_move"})
                    }
                    method => panic!("unexpected socket method: {method}"),
                };
                if request_number == 0 {
                    assert_eq!(request["method"], "session.snapshot");
                }
                writeln!(writer, "{}", json!({"id": request["id"], "result": result})).unwrap();
            }
        })
    }

    fn call(&self) -> Vec<String> {
        fs::read_to_string(&self.calls_path)
            .unwrap()
            .lines()
            .map(str::to_owned)
            .collect()
    }

    fn write_state(&self, state: &str) {
        fs::create_dir_all(&self.state_path).unwrap();
        fs::write(self.state_path.join("focus-anchor.json"), state).unwrap();
    }
}

impl Drop for Integration {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.directory);
    }
}

fn anchor_integration() -> Integration {
    Integration::new(
        json!([layout(
            "tab-anchor",
            vec![
                pane("pane-a", 0, 0, 60, 100),
                pane("pane-b", 60, 0, 40, 50),
                pane("pane-c", 60, 50, 40, 50),
            ],
        )]),
        json!([tab("workspace-1", "tab-anchor", 1)]),
        json!([workspace("workspace-1", "tab-anchor", 1)]),
    )
}

fn standard_integration() -> Integration {
    Integration::new(standard_layouts(), standard_tabs(), standard_workspaces())
}

fn standard_tabs() -> Value {
    json!([
        tab("workspace-1", "tab-1-1", 1),
        tab("workspace-2", "tab-2-1", 1),
        tab("workspace-2", "tab-2-2", 2),
        tab("workspace-2", "tab-2-3", 3),
        tab("workspace-3", "tab-3-1", 1),
    ])
}

fn standard_workspaces() -> Value {
    json!([
        workspace("workspace-1", "tab-1-1", 1),
        workspace("workspace-2", "tab-2-2", 2),
        workspace("workspace-3", "tab-3-1", 3),
    ])
}

fn standard_layouts() -> Value {
    json!([
        pane_layout("tab-1-1", "pane-1"),
        pane_layout("tab-2-1", "pane-2-1"),
        pane_layout("tab-2-2", "pane-2-2"),
        pane_layout("tab-2-3", "pane-2-3"),
        pane_layout("tab-3-1", "pane-3"),
    ])
}

fn pane_layout(tab_id: &str, prefix: &str) -> Value {
    layout(
        tab_id,
        vec![
            pane(&format!("{prefix}-bottom-right"), 50, 40, 50, 40),
            pane(&format!("{prefix}-top-right"), 50, 0, 50, 40),
            pane(&format!("{prefix}-top-left"), 0, 0, 50, 80),
        ],
    )
}

fn layout(tab_id: &str, panes: Vec<Value>) -> Value {
    json!({"panes": panes, "tab_id": tab_id})
}

fn pane(id: &str, x: u16, y: u16, width: u16, height: u16) -> Value {
    json!({"pane_id": id, "rect": {"height": height, "width": width, "x": x, "y": y}})
}

fn tab(workspace_id: &str, tab_id: &str, number: u16) -> Value {
    json!({"workspace_id": workspace_id, "tab_id": tab_id, "number": number})
}

fn workspace(workspace_id: &str, active_tab_id: &str, number: u16) -> Value {
    json!({"workspace_id": workspace_id, "active_tab_id": active_tab_id, "number": number})
}

fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn temp_dir() -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("hmp-{}-{unique}-{sequence}", std::process::id()))
}
