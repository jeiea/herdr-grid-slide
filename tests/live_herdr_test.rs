//! Headless smoke tests against a real Herdr in isolated named sessions. They
//! verify final server state but cannot see which tab a connected shell client
//! displays, so cross-container focus (herdrdev/herdr#4153) needs a manual check
//! before release: in an isolated session with a linked copy of the plugin, use
//! the real left/right keys across a tab boundary, direct next-tab and
//! next-workspace moves, `to-new-workspace` from a multi-pane tab, a leading-pane
//! whole-tab move, and `to-new-tab`, with plain new-tab/new-workspace keys as
//! negative cases. After each, the displayed tab must match the server snapshot,
//! hooks must finish without repeating, and the next input must reach the
//! displayed pane. With two connected clients both views switch to the
//! destination; that is the expected limitation of `focus_moved_pane`.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use serde_json::Value;

const HERDR_VERSION: &str = "herdr 0.8.2";
const PLUGIN_ID: &str = "jeiea.grid-slide";
const POLL_INTERVAL: Duration = Duration::from_millis(25);
const READY_TIMEOUT: Duration = Duration::from_secs(10);
const ACTION_TIMEOUT: Duration = Duration::from_secs(10);

static SESSION_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[test]
#[ignore = "requires Herdr and starts an isolated named session"]
fn arranging_panes_across_tabs_and_workspaces_keeps_every_view_balanced_and_focused() {
    let mut live = LiveHerdr::start().expect("start isolated Herdr session");
    let initial = live
        .create_workspace("pane journey")
        .expect("create workspace");
    live.link_plugin().expect("link copied plugin");
    let snapshot = live.snapshot().expect("initial snapshot");
    let a = terminal_id(&snapshot, &initial.root_pane_id);
    live.assert_scene(&snapshot, &a, &[vec![vec![&a]]]);
    let mut added = Vec::new();
    for (stage, order) in [
        ("add B", vec![0, 1]),
        ("add C", vec![0, 2, 1]),
        ("add D", vec![0, 2, 3, 1]),
        ("add E", vec![0, 4, 2, 3, 1]),
        ("add F", vec![0, 4, 2, 3, 5, 1]),
    ] {
        let snapshot = live.step(stage, "new-pane", Some("pane.focused"));
        added.push(terminal_id(&snapshot, &focused_pane_id(&snapshot)));
        let terminals: Vec<_> = std::iter::once(a.as_str())
            .chain(added.iter().map(String::as_str))
            .collect();
        live.assert_scene(
            &snapshot,
            added.last().unwrap(),
            &[vec![order.iter().map(|&i| terminals[i]).collect()]],
        );
    }
    let [b, c, d, e, f] = added.as_slice() else {
        panic!("five added panes")
    };
    let grid = live.last_snapshot.as_ref().unwrap();
    // Matching left-nested splits under both root children prove two three-column rows.
    let split_ids: Vec<_> = tab_layout(grid, &initial.tab_id)["splits"]
        .as_array()
        .unwrap()
        .iter()
        .map(|split| split["id"].as_str().unwrap())
        .collect();
    assert_eq!(
        split_ids,
        [
            "split_0_root",
            "split_1_0",
            "split_2_00",
            "split_3_1",
            "split_4_10"
        ]
    );
    let layout = tab_layout(grid, &initial.tab_id).clone();
    let snapshot = live.step("explicit balance preserves grid", "balance", None);
    assert_eq!(tab_layout(&snapshot, &initial.tab_id), &layout);
    live.assert_scene(&snapshot, f, &[vec![vec![&a, e, c, d, f, b]]]);
    for (action, focused) in [
        ("focus-left", d),
        ("focus-up", &a),
        ("focus-right", e),
        ("focus-down", f),
    ] {
        let snapshot = live.step(action, action, Some("pane.focused"));
        live.assert_scene(&snapshot, focused, &[vec![vec![&a, e, c, d, f, b]]]);
    }
    let snapshot = live.step("move F left", "move-left", None);
    live.assert_scene(&snapshot, f, &[vec![vec![&a, e, c, f, d, b]]]);
    let snapshot = live.step("move F up", "move-up", None);
    live.assert_scene(&snapshot, f, &[vec![vec![f, e, c, &a, d, b]]]);
    let snapshot = live.step("detach F to next tab", "to-new-tab", None);
    live.assert_scene(&snapshot, f, &[vec![vec![e, c, &a, d, b], vec![f]]]);
    assert_ne!(
        pane_rect(
            &snapshot,
            &initial.tab_id,
            pane_by_terminal_id(&snapshot, c)["pane_id"]
                .as_str()
                .unwrap()
        )["y"],
        pane_rect(&snapshot, &initial.tab_id, &initial.root_pane_id)["y"],
        "background tab must still have two panes in its top row"
    );

    // The headless single-pane center lands on a row boundary. Use the existing
    // upper-half entry fixture to make the upper-right A unambiguous.
    live.begin_stage("upper-half entry fixture");
    let detached = focused_pane_id(&snapshot);
    let auxiliary = live
        .split_pane(&detached, "down", false)
        .expect("upper-half source");
    let snapshot = live.snapshot().expect("entry fixture snapshot");
    let auxiliary_terminal = terminal_id(&snapshot, &auxiliary);
    let snapshot = live.step(
        "enter balanced right edge A",
        "focus-left",
        Some("pane.focused"),
    );
    live.assert_scene(
        &snapshot,
        &a,
        &[vec![vec![e, c, &a, d, b], vec![f, &auxiliary_terminal]]],
    );
    let snapshot = live.step("return to detached F", "focus-right", Some("pane.focused"));
    live.assert_scene(
        &snapshot,
        f,
        &[vec![vec![e, c, &a, d, b], vec![f, &auxiliary_terminal]]],
    );
    let snapshot = live.terminate("close unfocused auxiliary through API", &auxiliary, false);
    live.assert_scene(&snapshot, f, &[vec![vec![e, c, &a, d, b], vec![f]]]);
    let snapshot = live.step(
        "move F into previous tab",
        "move-left",
        Some("pane.focused"),
    );
    live.assert_scene(&snapshot, f, &[vec![vec![e, c, &a, f, d, b]]]);
    let snapshot = live.terminate("exit focused F", &focused_pane_id(&snapshot), true);
    let survivor = terminal_id(&snapshot, &focused_pane_id(&snapshot));
    live.assert_scene(&snapshot, &survivor, &[vec![vec![e, c, &a, d, b]]]);

    let remaining: Vec<&str> = [e, c, &a, d, b]
        .into_iter()
        .map(String::as_str)
        .filter(|id| *id != survivor)
        .collect();
    let snapshot = live.step(
        "detach survivor to new workspace",
        "to-new-workspace",
        Some("pane.focused"),
    );
    live.assert_scene(
        &snapshot,
        &survivor,
        &[vec![remaining.clone()], vec![vec![&survivor]]],
    );
    // Enter the bottom edge of the 2x2 grid; the centered anchor ties, so the
    // reverse reading-order tie break selects the bottom-right pane.
    let snapshot = live.step("focus previous workspace", "focus-up", Some("pane.focused"));
    live.assert_scene(
        &snapshot,
        remaining[3],
        &[vec![remaining.clone()], vec![vec![&survivor]]],
    );
    let snapshot = live.step("focus next workspace", "focus-down", Some("pane.focused"));
    live.assert_scene(
        &snapshot,
        &survivor,
        &[vec![remaining.clone()], vec![vec![&survivor]]],
    );
    let snapshot = live.step("create next tab", "new-tab", Some("pane.focused"));
    let g = terminal_id(&snapshot, &focused_pane_id(&snapshot));
    live.assert_scene(
        &snapshot,
        &g,
        &[vec![remaining.clone()], vec![vec![&survivor], vec![&g]]],
    );
    let snapshot = live.step("add H in next tab", "new-pane", Some("pane.focused"));
    let h = terminal_id(&snapshot, &focused_pane_id(&snapshot));
    live.assert_scene(
        &snapshot,
        &h,
        &[vec![remaining.clone()], vec![vec![&survivor], vec![&g, &h]]],
    );
    live.begin_stage("label moving tab");
    live.run_json([
        "tab",
        "rename",
        snapshot["focused_tab_id"].as_str().unwrap(),
        "moving",
    ])
    .expect("label moving tab");
    let snapshot = live.step(
        "tab to previous workspace",
        "tab-to-previous-workspace",
        Some("pane.focused"),
    );
    live.assert_scene(
        &snapshot,
        &h,
        &[vec![remaining.clone(), vec![&g, &h]], vec![vec![&survivor]]],
    );
    live.assert_focused_tab_label(&snapshot, "moving");
    let snapshot = live.step(
        "tab round trip to next workspace",
        "tab-to-next-workspace",
        Some("pane.focused"),
    );
    live.assert_scene(
        &snapshot,
        &h,
        &[vec![remaining.clone()], vec![vec![&survivor], vec![&g, &h]]],
    );
    live.assert_focused_tab_label(&snapshot, "moving");
    let snapshot = live.step(
        "tab to new workspace",
        "tab-to-new-workspace",
        Some("pane.focused"),
    );
    live.assert_scene(
        &snapshot,
        &h,
        &[
            vec![remaining.clone()],
            vec![vec![&survivor]],
            vec![vec![&g, &h]],
        ],
    );
    live.assert_focused_tab_label(&snapshot, "moving");
    for (stage, action, middle, last) in [
        ("right side up", "move-up", vec![&survivor, &h], vec![&g]),
        (
            "right side down",
            "move-down",
            vec![&survivor],
            vec![&g, &h],
        ),
        (
            "select left side",
            "move-left",
            vec![&survivor],
            vec![&h, &g],
        ),
        ("left side up", "move-up", vec![&h, &survivor], vec![&g]),
        ("left side down", "move-down", vec![&survivor], vec![&h, &g]),
        (
            "restore right side",
            "move-right",
            vec![&survivor],
            vec![&g, &h],
        ),
    ] {
        let snapshot = live.step(
            stage,
            action,
            matches!(action, "move-up" | "move-down").then_some("pane.focused"),
        );
        live.assert_scene(
            &snapshot,
            &h,
            &[
                vec![remaining.clone()],
                vec![middle.into_iter().map(String::as_str).collect()],
                vec![last.into_iter().map(String::as_str).collect()],
            ],
        );
    }
    let snapshot = live.step(
        "reorder workspace before previous",
        "move-workspace-previous",
        Some("pane.focused"),
    );
    live.assert_scene(
        &snapshot,
        &h,
        &[
            vec![remaining.clone()],
            vec![vec![&g, &h]],
            vec![vec![&survivor]],
        ],
    );
    let snapshot = live.step(
        "move only tab and close source workspace",
        "tab-to-next-workspace",
        Some("pane.focused"),
    );
    live.assert_scene(
        &snapshot,
        &h,
        &[vec![remaining], vec![vec![&survivor], vec![&g, &h]]],
    );
    live.assert_focused_tab_label(&snapshot, "moving");
}

#[test]
#[ignore = "requires Herdr and starts an isolated named session"]
fn crossing_tabs_inserts_at_the_entry_edge_and_leaves_both_tabs_open_on_return() {
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

    let before = live.snapshot().expect("snapshot before crossing");
    let stationary = terminal_id(&before, &source.root_pane_id);
    let moving = terminal_id(&before, &moving);
    let target = terminal_id(&before, &destination.root_pane_id);
    let after = live.step(
        "enter next tab before its first pane",
        "move-right",
        Some("pane.focused"),
    );
    live.assert_scene(
        &after,
        &moving,
        &[vec![vec![&stationary], vec![&moving, &target]]],
    );
    let after = live.step(
        "return while leaving the departure tab open",
        "move-left",
        None,
    );
    live.assert_scene(
        &after,
        &moving,
        &[vec![vec![&stationary, &moving], vec![&target]]],
    );
}

#[test]
#[ignore = "requires Herdr and starts an isolated named session"]
fn detaching_a_pane_inserts_and_focuses_a_new_tab_before_the_trailing_tab() {
    let mut live = LiveHerdr::start().expect("start isolated Herdr session");
    let source = live
        .create_workspace("new-tab-source")
        .expect("create source workspace");
    let moving = live
        .split_pane(&source.root_pane_id, "right", true)
        .expect("create focused moving pane");
    let trailing = live
        .create_tab(&source.workspace_id, "new-tab-trailing")
        .expect("create trailing tab");
    live.focus_tab(&source.tab_id).expect("focus source tab");
    live.link_plugin().expect("link copied plugin");

    let before = live.snapshot().expect("snapshot before detaching");
    let stationary = terminal_id(&before, &source.root_pane_id);
    let moving = terminal_id(&before, &moving);
    let trailing = terminal_id(&before, &trailing.root_pane_id);
    let after = live.step("detach before trailing tab", "to-new-tab", None);
    live.assert_scene(
        &after,
        &moving,
        &[vec![vec![&stationary], vec![&moving], vec![&trailing]]],
    );
}

#[test]
#[ignore = "requires Herdr and starts an isolated named session"]
fn wrapping_between_workspaces_uses_the_active_tab_and_preserves_the_selected_side() {
    let mut live = LiveHerdr::start().expect("start isolated Herdr session");
    let previous = live
        .create_workspace("previous")
        .expect("previous workspace");
    let active = live
        .create_tab(&previous.workspace_id, "active")
        .expect("active tab");
    let original = live
        .create_workspace("original")
        .expect("original workspace");
    let moving = live
        .split_pane(&original.root_pane_id, "right", true)
        .expect("right pane");
    live.link_plugin().expect("link copied plugin");
    let before = live.snapshot().expect("initial snapshot");
    let inactive = terminal_id(&before, &previous.root_pane_id);
    let target = terminal_id(&before, &active.root_pane_id);
    let stationary = terminal_id(&before, &original.root_pane_id);
    let moving = terminal_id(&before, &moving);
    let snapshot = live.step(
        "wrap down to first workspace active tab",
        "move-down",
        Some("pane.focused"),
    );
    live.assert_scene(
        &snapshot,
        &moving,
        &[
            vec![vec![&inactive], vec![&target, &moving]],
            vec![vec![&stationary]],
        ],
    );
    let snapshot = live.step("select left before wrapping back", "move-left", None);
    live.assert_scene(
        &snapshot,
        &moving,
        &[
            vec![vec![&inactive], vec![&moving, &target]],
            vec![vec![&stationary]],
        ],
    );
    let snapshot = live.step(
        "wrap up to last workspace on selected side",
        "move-up",
        Some("pane.focused"),
    );
    live.assert_scene(
        &snapshot,
        &moving,
        &[
            vec![vec![&inactive], vec![&target]],
            vec![vec![&moving, &stationary]],
        ],
    );
}

#[test]
#[ignore = "requires Herdr and starts an isolated named session"]
fn closing_focused_and_unfocused_panes_through_the_api_balances_multiple_survivors() {
    let mut live = LiveHerdr::start().expect("start isolated Herdr session");
    let fixture = live
        .create_workspace("focused API close")
        .expect("create workspace");
    let terminated = live
        .split_pane(&fixture.root_pane_id, "right", true)
        .expect("terminating pane");
    let survivor = live
        .split_pane(&fixture.root_pane_id, "down", false)
        .expect("lower survivor");
    let before = live.snapshot().expect("T-layout snapshot");
    assert_t_layout(
        &before,
        "before close",
        &fixture.tab_id,
        &fixture.root_pane_id,
        &survivor,
        &terminated,
    );
    assert_eq!(focused_pane_id(&before), terminated);
    let terminals = [
        terminal_id(&before, &fixture.root_pane_id),
        terminal_id(&before, &survivor),
    ];
    live.link_plugin().expect("link copied plugin");
    let after = live.terminate("focused API close", &terminated, false);
    let focused = terminal_id(&after, &focused_pane_id(&after));
    live.assert_scene(
        &after,
        &focused,
        &[vec![terminals.iter().map(String::as_str).collect()]],
    );
    let before = live.step(
        "add a third pane before unfocused close",
        "new-pane",
        Some("pane.focused"),
    );
    let focused = terminal_id(&before, &focused_pane_id(&before));
    live.assert_scene(
        &before,
        &focused,
        &[vec![vec![&terminals[0], &focused, &terminals[1]]]],
    );
    // Closing the unfocused leftmost pane leaves a 2:1 split. Only pane.closed
    // can start the resize to equal columns; no focus change starts another hook.
    let logs = live.plugin_log_ids().expect("logs before unfocused close");
    let after = live.terminate(
        "close unfocused left pane and equalize two survivors",
        &fixture.root_pane_id,
        false,
    );
    live.assert_scene(&after, &focused, &[vec![vec![&focused, &terminals[1]]]]);
    assert!(
        !live
            .plugin_logs()
            .expect("logs after unfocused close")
            .iter()
            .any(|log| log["event"] == "pane.focused"
                && !logs.contains(log["log_id"].as_str().unwrap())),
        "unfocused API close must balance without a pane.focused hook"
    );
}

#[test]
#[ignore = "requires Herdr and starts an isolated named session"]
fn moving_a_pane_or_tab_to_a_new_workspace_inserts_it_before_trailing_workspaces() {
    let mut live = LiveHerdr::start().expect("start isolated Herdr session");
    let pane_source = live
        .create_workspace("workspace-pane-source")
        .expect("create pane source workspace");
    let detached = live
        .split_pane(&pane_source.root_pane_id, "right", true)
        .expect("create pane to detach");
    let tab_source = live
        .create_workspace("workspace-tab-source")
        .expect("create tab source workspace");
    let moving = live
        .create_tab(&tab_source.workspace_id, "moving")
        .expect("create moving tab beside the workspace's first tab");
    let middle = live
        .split_pane(&moving.root_pane_id, "right", true)
        .expect("create middle pane in the moving tab");
    let focused = live
        .split_pane(&middle, "right", true)
        .expect("create focused right pane in the moving tab");
    let trailing = live
        .create_workspace("workspace-trailing")
        .expect("create trailing workspace");
    live.focus_workspace(&pane_source.workspace_id)
        .expect("focus pane source workspace");
    live.focus_tab(&pane_source.tab_id)
        .expect("focus pane source tab");
    live.link_plugin().expect("link copied plugin");
    let before = live
        .snapshot()
        .expect("snapshot before inserting workspaces");
    let stationary = terminal_id(&before, &pane_source.root_pane_id);
    let detached = terminal_id(&before, &detached);
    let inactive = terminal_id(&before, &tab_source.root_pane_id);
    let left = terminal_id(&before, &moving.root_pane_id);
    let center = terminal_id(&before, &middle);
    let right = terminal_id(&before, &focused);
    let trailing = terminal_id(&before, &trailing.root_pane_id);
    let after = live.step(
        "detach before trailing workspaces",
        "to-new-workspace",
        Some("pane.focused"),
    );
    live.assert_scene(
        &after,
        &detached,
        &[
            vec![vec![&stationary]],
            vec![vec![&detached]],
            vec![vec![&inactive], vec![&left, &center, &right]],
            vec![vec![&trailing]],
        ],
    );

    let after = live.step(
        "enter moving tab in next workspace",
        "focus-down",
        Some("pane.focused"),
    );
    live.assert_scene(
        &after,
        &center,
        &[
            vec![vec![&stationary]],
            vec![vec![&detached]],
            vec![vec![&inactive], vec![&left, &center, &right]],
            vec![vec![&trailing]],
        ],
    );
    let after = live.step(
        "select right pane in moving tab",
        "focus-right",
        Some("pane.focused"),
    );
    live.assert_scene(
        &after,
        &right,
        &[
            vec![vec![&stationary]],
            vec![vec![&detached]],
            vec![vec![&inactive], vec![&left, &center, &right]],
            vec![vec![&trailing]],
        ],
    );
    let after = live.step(
        "move tab before trailing workspace",
        "tab-to-new-workspace",
        Some("pane.focused"),
    );
    live.assert_scene(
        &after,
        &right,
        &[
            vec![vec![&stationary]],
            vec![vec![&detached]],
            vec![vec![&inactive]],
            vec![vec![&left, &center, &right]],
            vec![vec![&trailing]],
        ],
    );
    live.assert_focused_tab_label(&after, "moving");
}

#[test]
#[ignore = "requires Herdr and starts an isolated named session"]
fn moving_the_only_single_pane_tab_closes_its_workspace_and_stops_at_the_last_workspace() {
    let mut live = LiveHerdr::start().expect("start isolated Herdr session");
    let target = live.create_workspace("first").expect("target workspace");
    let source = live.create_workspace("second").expect("source workspace");
    let before = live
        .snapshot()
        .expect("snapshot before labeling and moving");
    live.begin_stage("label only tab");
    live.run_json(["tab", "rename", &source.tab_id, "moving"])
        .expect("label source tab");
    live.link_plugin().expect("link copied plugin");
    let moving = terminal_id(&before, &source.root_pane_id);
    let stationary = terminal_id(&before, &target.root_pane_id);
    let after = live.step(
        "move only single-pane tab to previous workspace",
        "tab-to-previous-workspace",
        Some("pane.focused"),
    );
    live.assert_scene(&after, &moving, &[vec![vec![&stationary], vec![&moving]]]);
    assert_eq!(after["focused_workspace_id"], target.workspace_id);
    assert_eq!(workspace_number(&after, &target.workspace_id), 1);
    assert_ne!(focused_pane_id(&after), source.root_pane_id);
    live.assert_focused_tab_label(&after, "moving");
    for field in ["tabs", "layouts"] {
        assert!(
            !after[field]
                .as_array()
                .unwrap()
                .iter()
                .any(|tab| tab["tab_id"] == source.tab_id),
            "{field}: source tab must be removed"
        );
    }
    for no_op in ["tab-to-next-workspace", "tab-to-previous-workspace"] {
        let unchanged = live.step(no_op, no_op, None);
        live.assert_scene(
            &unchanged,
            &moving,
            &[vec![vec![&stationary], vec![&moving]]],
        );
        for field in ["workspaces", "tabs", "layouts", "focused_pane_id"] {
            assert_eq!(unchanged[field], after[field], "{no_op}: {field}");
        }
    }
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
    stage: String,
    previous_snapshot: Option<Value>,
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
        let temporary = if cfg!(windows) {
            std::env::temp_dir()
        } else {
            PathBuf::from("/tmp")
        };
        let root = temporary.join(&name);
        let config_home = root.join("config");
        let state_home = root.join("state");
        let runtime_dir = root.join("runtime");
        let config_path = root.join("herdr-config.toml");
        let plugin_root = root.join("plugin");
        let plugin_binary = plugin_root.join(format!(
            "bin/herdr-grid-slide{}",
            std::env::consts::EXE_SUFFIX
        ));
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
        // Older servers subtract UI chrome from the headless area. Keep enough
        // width for a three-column grid on every supported version.
        fs::write(&config_path, "onboarding = false\n[ui]\nsidebar_start_collapsed = true\nsidebar_collapsed_mode = 'hidden'\n")
            .map_err(|error| format!("write isolated Herdr config: {error}"))?;

        fs::copy(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("herdr-plugin.toml"),
            plugin_root.join("herdr-plugin.toml"),
        )
        .map_err(|error| format!("copy plugin manifest: {error}"))?;
        fs::copy(
            Path::new(env!("CARGO_BIN_EXE_herdr-grid-slide")),
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
            stage: "setup".into(),
            previous_snapshot: None,
        };
        live.require_version()?;
        live.spawn_server()?;
        live.wait_until_ready()?;
        Ok(live)
    }

    fn require_version(&self) -> Result<(), String> {
        let output = self.run_raw(["--version"])?;
        let version = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let expected =
            std::env::var("HERDR_TEST_VERSION").unwrap_or_else(|_| HERDR_VERSION.to_owned());
        if version != expected {
            return Err(format!("live smoke requires {expected}, found {version:?}"));
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
        let mut executable = self.plugin_root.join(entrypoint);
        if cfg!(windows) {
            executable.set_extension(std::env::consts::EXE_EXTENSION);
        }
        assert_same_path(executable, &self.plugin_binary)
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

    fn close_pane(&self, pane_id: &str) -> Result<(), String> {
        self.run_json(["pane", "close", pane_id]).map(|_| ())
    }

    fn exit_pane(&self, pane_id: &str) -> Result<(), String> {
        self.run_raw(["pane", "run", pane_id, "exit"]).map(|_| ())
    }

    fn wait_for_pane_to_disappear(&mut self, pane_id: &str) -> Result<Value, String> {
        let deadline = Instant::now() + ACTION_TIMEOUT;
        while Instant::now() < deadline {
            let snapshot = self.snapshot()?;
            if !snapshot["panes"]
                .as_array()
                .ok_or_else(|| format!("snapshot omitted panes: {snapshot}"))?
                .iter()
                .any(|pane| pane["pane_id"] == pane_id)
            {
                return Ok(snapshot);
            }
            std::thread::sleep(POLL_INTERVAL);
        }
        Err(format!(
            "pane {pane_id} did not disappear within {ACTION_TIMEOUT:?}"
        ))
    }

    fn run_and_wait_for_text(&self, pane_id: &str, stage: &str) -> Result<(), String> {
        let marker = format!("stage-next-input-{}", stage.replace(' ', "-"));
        self.run_raw(["pane", "run", pane_id, "printf", marker.as_str()])?;
        self.run_raw([
            "pane",
            "wait-output",
            pane_id,
            "--match",
            marker.as_str(),
            "--timeout",
            "5000",
        ])?;
        Ok(())
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
            if let Some(log) = self
                .plugin_logs()?
                .iter()
                .find(|log| log["log_id"] == log_id)
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

    fn begin_stage(&mut self, stage: &str) {
        self.stage = stage.into();
        self.previous_snapshot = self.last_snapshot.clone();
    }

    /// Require the event promised by this stage. Moving a pane within a workspace
    /// can retain its focused ID and emit no focus event, as can swaps and no-ops.
    fn step(&mut self, stage: &str, action: &str, required_event: Option<&str>) -> Value {
        self.begin_stage(stage);
        let logs = self.plugin_log_ids().expect("logs before action");
        self.invoke_action(action)
            .unwrap_or_else(|error| panic!("{stage}: {error}"));
        self.wait_for_hooks(&logs, required_event)
            .unwrap_or_else(|error| panic!("{stage}: {error}"));
        self.snapshot()
            .unwrap_or_else(|error| panic!("{stage}: {error}"))
    }

    fn terminate(&mut self, stage: &str, pane_id: &str, exit_process: bool) -> Value {
        self.begin_stage(stage);
        let logs = self.plugin_log_ids().expect("logs before termination");
        let result = if exit_process {
            self.exit_pane(pane_id)
        } else {
            self.close_pane(pane_id)
        };
        result.unwrap_or_else(|error| panic!("{stage}: {error}"));
        self.wait_for_pane_to_disappear(pane_id)
            .unwrap_or_else(|error| panic!("{stage}: {error}"));
        self.wait_for_hooks(
            &logs,
            Some(if exit_process {
                "pane.exited"
            } else {
                "pane.closed"
            }),
        )
        .unwrap_or_else(|error| panic!("{stage}: {error}"));
        self.snapshot()
            .unwrap_or_else(|error| panic!("{stage}: {error}"))
    }

    /// A quiet, completed hook set also permits actions which emit no focus event.
    fn wait_for_hooks(
        &self,
        existing: &BTreeSet<String>,
        required_event: Option<&str>,
    ) -> Result<(), String> {
        let deadline = Instant::now() + ACTION_TIMEOUT;
        let mut previous = Vec::new();
        let mut quiet_polls = 0;
        while Instant::now() < deadline {
            let hooks: Vec<_> = self
                .plugin_logs()?
                .into_iter()
                .filter(|log| {
                    matches!(
                        log["event"].as_str(),
                        Some("pane.focused" | "pane.closed" | "pane.exited")
                    ) && log["log_id"]
                        .as_str()
                        .is_some_and(|id| !existing.contains(id))
                })
                .collect();
            if hooks.iter().any(|log| log["status"] == "failed") {
                return Err(format!("balance hook failed: {hooks:?}"));
            }
            let complete = hooks.iter().all(|log| {
                log["status"] == "succeeded"
                    && log["finished_unix_ms"].as_u64().is_some()
                    && log["exit_code"] == 0
            }) && required_event
                .is_none_or(|event| hooks.iter().any(|log| log["event"] == event));
            quiet_polls = if hooks == previous {
                quiet_polls + 1
            } else {
                0
            };
            if complete && quiet_polls >= 2 {
                return Ok(());
            }
            previous = hooks;
            std::thread::sleep(POLL_INTERVAL);
        }
        Err(format!(
            "hooks did not settle within {ACTION_TIMEOUT:?}: {previous:?}"
        ))
    }

    /// Expected workspaces, tabs, then terminal IDs in reading order. Background
    /// tabs may defer balancing; the entered tab must already have an aligned grid.
    /// Sends a marker command to the focused pane and waits for its output.
    fn assert_scene(&self, snapshot: &Value, focused_terminal: &str, expected: &[Vec<Vec<&str>>]) {
        let stage = &self.stage;
        let mut workspaces: Vec<_> = snapshot["workspaces"].as_array().unwrap().iter().collect();
        workspaces.sort_by_key(|workspace| workspace["number"].as_u64().unwrap());
        let actual: Vec<Vec<Vec<String>>> = workspaces
            .iter()
            .map(|workspace| {
                snapshot["tabs"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter(|tab| tab["workspace_id"] == workspace["workspace_id"])
                    .map(|tab| {
                        panes_in_reading_order(snapshot, tab["tab_id"].as_str().unwrap())
                            .into_iter()
                            .map(|id| terminal_id(snapshot, id))
                            .collect()
                    })
                    .collect()
            })
            .collect();
        assert_eq!(
            actual, expected,
            "{stage}: workspace/tab/pane count and reading order"
        );
        assert_eq!(
            snapshot["panes"].as_array().unwrap().len(),
            expected.iter().flatten().map(Vec::len).sum::<usize>(),
            "{stage}: no stranded panes"
        );
        let focused = pane_by_terminal_id(snapshot, focused_terminal);
        assert_eq!(
            snapshot["focused_pane_id"], focused["pane_id"],
            "{stage}: focused pane"
        );
        assert_eq!(
            snapshot["focused_tab_id"], focused["tab_id"],
            "{stage}: focused tab"
        );
        assert_eq!(
            snapshot["focused_workspace_id"], focused["workspace_id"],
            "{stage}: focused workspace"
        );
        assert_eq!(focused["focused"], true, "{stage}: focused terminal");
        let tab_id = focused["tab_id"].as_str().unwrap();
        let layout = tab_layout(snapshot, tab_id);
        let order = panes_in_reading_order(snapshot, tab_id);
        // Explicit expectations for the one-to-six-pane, 120-column fixture.
        let columns = match order.len() {
            1 => 1,
            2 | 4 => 2,
            3 | 5 | 6 => 3,
            _ => panic!("{stage}: unexpected grid size"),
        };
        let rows = order.len().div_ceil(columns);
        let area = &layout["area"];
        let boundary = |axis: &str, extent: &str, index: usize, slots: usize| {
            area[axis].as_u64().unwrap()
                + (area[extent].as_f64().unwrap() * index as f64 / slots as f64).round() as u64
        };
        for (row, panes) in order.chunks(columns).enumerate() {
            for (column, id) in panes.iter().enumerate() {
                let rect = pane_rect(snapshot, tab_id, id);
                let x = boundary("x", "width", column, panes.len());
                let y = boundary("y", "height", row, rows);
                assert_eq!(
                    rect,
                    &serde_json::json!({"x": x, "y": y,
                    "width": boundary("x", "width", column + 1, panes.len()) - x,
                    "height": boundary("y", "height", row + 1, rows) - y}),
                    "{stage}: aligned grid at row {row}, column {column}"
                );
            }
        }
        self.run_and_wait_for_text(focused["pane_id"].as_str().unwrap(), stage)
            .unwrap_or_else(|error| panic!("{stage}: next input: {error}"));
    }

    fn assert_focused_tab_label(&self, snapshot: &Value, expected: &str) {
        assert_eq!(
            tab(snapshot, snapshot["focused_tab_id"].as_str().unwrap())["label"],
            expected,
            "{}: focused tab label",
            self.stage
        );
    }

    fn plugin_log_ids(&self) -> Result<BTreeSet<String>, String> {
        self.plugin_logs()?
            .iter()
            .map(|log| response_string(log, &["log_id"]))
            .collect()
    }

    fn plugin_logs(&self) -> Result<Vec<Value>, String> {
        let response = self.run_json([
            "plugin", "log", "list", "--plugin", PLUGIN_ID, "--limit", "50",
        ])?;
        Ok(response["result"]["logs"]
            .as_array()
            .ok_or_else(|| format!("plugin log response omitted logs: {response}"))?
            .to_owned())
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
        for (key, _) in std::env::vars_os() {
            if key.to_string_lossy().starts_with("HERDR_") {
                command.env_remove(key);
            }
        }
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
            eprintln!("failed stage: {}", self.stage);
            if let Some(snapshot) = &self.previous_snapshot {
                eprintln!("snapshot before stage: {snapshot}");
            }
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

fn assert_t_layout(
    snapshot: &Value,
    stage: &str,
    tab_id: &str,
    upper: &str,
    lower: &str,
    right: &str,
) {
    let upper = pane_rect(snapshot, tab_id, upper);
    let lower = pane_rect(snapshot, tab_id, lower);
    let right = pane_rect(snapshot, tab_id, right);
    assert_eq!(
        upper["x"], lower["x"],
        "{stage}: survivors should start stacked"
    );
    assert!(
        upper["y"].as_u64().unwrap() < lower["y"].as_u64().unwrap(),
        "{stage}: survivor fixture should be vertical"
    );
    assert!(
        upper["x"].as_u64().unwrap() < right["x"].as_u64().unwrap(),
        "{stage}: terminating pane should occupy the right half"
    );
}

fn unique_suffix() -> String {
    let sequence = SESSION_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    format!("{:x}-{sequence:x}", std::process::id())
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

fn tab_layout<'a>(snapshot: &'a Value, tab_id: &str) -> &'a Value {
    snapshot["layouts"]
        .as_array()
        .expect("snapshot layout list")
        .iter()
        .find(|layout| layout["tab_id"] == tab_id)
        .unwrap_or_else(|| panic!("tab {tab_id} layout missing from snapshot: {snapshot}"))
}

fn pane_rect<'a>(snapshot: &'a Value, tab_id: &str, pane_id: &str) -> &'a Value {
    tab_layout(snapshot, tab_id)["panes"]
        .as_array()
        .expect("layout pane list")
        .iter()
        .find(|pane| pane["pane_id"] == pane_id)
        .map(|pane| &pane["rect"])
        .unwrap_or_else(|| panic!("pane {pane_id} layout missing from snapshot: {snapshot}"))
}

fn pane_by_terminal_id<'a>(snapshot: &'a Value, terminal_id: &str) -> &'a Value {
    snapshot["panes"]
        .as_array()
        .expect("snapshot pane list")
        .iter()
        .find(|pane| pane["terminal_id"] == terminal_id)
        .unwrap_or_else(|| panic!("terminal {terminal_id} missing from snapshot: {snapshot}"))
}

fn terminal_id(snapshot: &Value, pane_id: &str) -> String {
    pane(snapshot, pane_id)["terminal_id"]
        .as_str()
        .expect("pane terminal ID")
        .to_string()
}

fn tab<'a>(snapshot: &'a Value, tab_id: &str) -> &'a Value {
    snapshot["tabs"]
        .as_array()
        .expect("snapshot tab list")
        .iter()
        .find(|tab| tab["tab_id"] == tab_id)
        .unwrap_or_else(|| panic!("tab {tab_id} missing from snapshot: {snapshot}"))
}

fn workspace_number(snapshot: &Value, workspace_id: &str) -> u64 {
    snapshot["workspaces"]
        .as_array()
        .expect("snapshot workspace list")
        .iter()
        .find(|workspace| workspace["workspace_id"] == workspace_id)
        .and_then(|workspace| workspace["number"].as_u64())
        .unwrap_or_else(|| panic!("workspace {workspace_id} missing from snapshot: {snapshot}"))
}

/// Pane IDs of a tab sorted by their top edge, then their left edge.
fn panes_in_reading_order<'a>(snapshot: &'a Value, tab_id: &str) -> Vec<&'a str> {
    let mut panes: Vec<(u64, u64, &str)> = snapshot["layouts"]
        .as_array()
        .expect("snapshot layout list")
        .iter()
        .find(|layout| layout["tab_id"] == tab_id)
        .and_then(|layout| layout["panes"].as_array())
        .unwrap_or_else(|| panic!("tab {tab_id} layout missing from snapshot: {snapshot}"))
        .iter()
        .map(|pane| {
            (
                pane["rect"]["y"].as_u64().expect("pane top edge"),
                pane["rect"]["x"].as_u64().expect("pane left edge"),
                pane["pane_id"].as_str().expect("pane ID"),
            )
        })
        .collect();
    panes.sort_unstable();
    panes.into_iter().map(|(_, _, pane_id)| pane_id).collect()
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
