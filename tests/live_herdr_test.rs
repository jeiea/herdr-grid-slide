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
fn adding_six_panes_aligns_columns_and_builds_matching_rows() {
    let mut live = LiveHerdr::start().expect("start isolated Herdr session");
    let fixture = live
        .create_workspace("grid-columns")
        .expect("create workspace");
    live.link_plugin().expect("link copied plugin");
    for _ in 0..5 {
        live.invoke_action_and_wait_for_balance("new-pane")
            .expect("add pane and settle grid");
    }
    let before = live.snapshot().expect("capture automatic grid");
    let order = panes_in_reading_order(&before, &fixture.tab_id);
    assert_eq!(order.len(), 6);
    let splits = tab_layout(&before, &fixture.tab_id)["splits"]
        .as_array()
        .unwrap();
    let ids: Vec<_> = splits
        .iter()
        .map(|split| split["id"].as_str().unwrap())
        .collect();
    assert_eq!(
        ids,
        [
            "split_0_root",
            "split_1_0",
            "split_2_00",
            "split_3_1",
            "split_4_10"
        ]
    );
    for column in 0..3 {
        assert_eq!(
            pane_x(&before, &fixture.tab_id, order[column]),
            pane_x(&before, &fixture.tab_id, order[column + 3])
        );
    }
    live.invoke_action("balance")
        .expect("explicitly balance again");
    let after = live.snapshot().expect("capture explicit grid");
    assert_eq!(
        tab_layout(&before, &fixture.tab_id),
        tab_layout(&after, &fixture.tab_id)
    );
}

#[test]
#[ignore = "requires Herdr and starts an isolated named session"]
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
#[ignore = "requires Herdr and starts an isolated named session"]
fn move_right_then_left_crosses_the_tab_boundary_and_keeps_focus() {
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

    let after_right = live.snapshot().expect("capture post-right snapshot");
    assert_eq!(
        pane_ids_in_tab(&after_right, &source.tab_id),
        BTreeSet::from([source.root_pane_id.clone()])
    );
    assert_eq!(
        pane_ids_in_tab(&after_right, &destination.tab_id),
        BTreeSet::from([destination.root_pane_id.clone(), moving.clone()])
    );
    assert_eq!(focused_pane_id(&after_right), moving);

    live.invoke_action("move-left")
        .expect("invoke move-left and wait for its log");

    let after_left = live.snapshot().expect("capture post-left snapshot");
    assert_eq!(
        pane_ids_in_tab(&after_left, &source.tab_id),
        BTreeSet::from([source.root_pane_id.clone(), moving.clone()])
    );
    assert_eq!(
        pane_ids_in_tab(&after_left, &destination.tab_id),
        BTreeSet::from([destination.root_pane_id.clone()])
    );
    assert_eq!(focused_pane_id(&after_left), moving);
}

#[test]
#[ignore = "requires Herdr and starts an isolated named session"]
fn to_new_tab_places_the_detached_pane_in_the_next_tab_and_sets_server_focus_to_it() {
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

    let before = live.snapshot().expect("capture pre-action snapshot");
    let moving_terminal_id = terminal_id(&before, &moving);
    assert_eq!(focused_pane_id(&before), moving);

    // The headless harness covers real server requests and state, not client views.
    live.invoke_action("to-new-tab")
        .expect("invoke to-new-tab and wait for its log");

    let after = live.snapshot().expect("capture post-action snapshot");
    let moved = pane_by_terminal_id(&after, &moving_terminal_id);
    let moved_pane_id = moved["pane_id"].as_str().expect("moved pane ID");
    let created_tab_id = moved["tab_id"].as_str().expect("created tab ID");
    let tab_order: Vec<_> = after["tabs"]
        .as_array()
        .expect("snapshot tab list")
        .iter()
        .filter(|tab| tab["workspace_id"] == source.workspace_id)
        .map(|tab| tab["tab_id"].as_str().expect("tab ID"))
        .collect();

    assert_eq!(
        tab_order,
        [
            source.tab_id.as_str(),
            created_tab_id,
            trailing.tab_id.as_str()
        ]
    );
    assert_eq!(moved["workspace_id"], source.workspace_id);
    assert_eq!(moved["focused"], true);
    assert_eq!(after["focused_tab_id"], created_tab_id);
    assert_eq!(focused_pane_id(&after), moved_pane_id);
    assert_eq!(
        pane_ids_in_tab(&after, &source.tab_id),
        BTreeSet::from([source.root_pane_id.clone()])
    );
    assert_eq!(
        pane_ids_in_tab(&after, created_tab_id),
        BTreeSet::from([moved_pane_id.to_owned()])
    );
    assert_eq!(
        pane_ids_in_tab(&after, &trailing.tab_id),
        BTreeSet::from([trailing.root_pane_id])
    );
}

#[test]
#[ignore = "requires Herdr and starts an isolated named session"]
fn vertical_round_trips_keep_the_side_selected_by_the_user() {
    let mut live = LiveHerdr::start().expect("start isolated Herdr session");
    let previous = live
        .create_workspace("previous-workspace")
        .expect("create previous workspace");
    let active_previous = live
        .create_tab(&previous.workspace_id, "active-previous-tab")
        .expect("create active tab in previous workspace");
    let original = live
        .create_workspace("original-workspace")
        .expect("create original workspace");
    let moving = live
        .split_pane(&original.root_pane_id, "right", true)
        .expect("create focused right pane");
    live.link_plugin().expect("link copied plugin");

    let before = live.snapshot().expect("capture pre-action snapshot");
    let moving_terminal_id = terminal_id(&before, &moving);
    let inactive_previous_panes = pane_ids_in_tab(&before, &previous.tab_id);
    assert!(
        workspace_number(&before, &previous.workspace_id)
            < workspace_number(&before, &original.workspace_id)
    );
    assert_eq!(focused_pane_id(&before), moving);

    let to_previous = (&original, &active_previous);
    let to_original = (&active_previous, &original);
    let unchanged = (previous.tab_id.as_str(), &inactive_previous_panes);
    let vertical = |live: &mut LiveHerdr, stage, action_id, route, moving_first| {
        assert_vertical_move(
            live,
            stage,
            action_id,
            &moving_terminal_id,
            route,
            moving_first,
            unchanged,
        );
    };
    let same_tab = |live: &mut LiveHerdr, stage, action_id, destination, moving_first| {
        live.invoke_action(action_id)
            .unwrap_or_else(|error| panic!("{stage}: {action_id} failed: {error}"));
        let snapshot = live
            .snapshot()
            .unwrap_or_else(|error| panic!("{stage}: capture snapshot after {action_id}: {error}"));
        assert_two_pane_position(
            &snapshot,
            stage,
            action_id,
            &moving_terminal_id,
            destination,
            moving_first,
        );
    };

    let stage = "right pure round trip";
    vertical(&mut live, stage, "move-up", to_previous, false);
    vertical(&mut live, stage, "move-down", to_original, false);

    let stage = "left selected before down";
    vertical(&mut live, stage, "move-up", to_previous, false);
    same_tab(&mut live, stage, "move-left", &active_previous, true);
    vertical(&mut live, stage, "move-down", to_original, true);

    let stage = "left pure round trip";
    vertical(&mut live, stage, "move-up", to_previous, true);
    vertical(&mut live, stage, "move-down", to_original, true);

    let stage = "left selected before up";
    same_tab(&mut live, stage, "move-right", &original, false);
    vertical(&mut live, stage, "move-down", to_previous, false);
    same_tab(&mut live, stage, "move-left", &active_previous, true);
    vertical(&mut live, stage, "move-up", to_original, true);
}

#[test]
#[ignore = "requires Herdr and starts an isolated named session"]
fn pane_termination_automatically_balances_the_surviving_tab() {
    for (stage, focus_terminated_pane, exit_process) in [
        ("focused API close", true, false),
        ("non-focused API close", false, false),
        ("focused process exit", true, true),
    ] {
        let mut live = LiveHerdr::start()
            .unwrap_or_else(|error| panic!("{stage}: start isolated Herdr session: {error}"));
        let fixture = live
            .create_workspace(stage)
            .unwrap_or_else(|error| panic!("{stage}: create workspace: {error}"));
        let terminated = live
            .split_pane(&fixture.root_pane_id, "right", focus_terminated_pane)
            .unwrap_or_else(|error| panic!("{stage}: create terminating pane: {error}"));
        let survivor = live
            .split_pane(&fixture.root_pane_id, "down", false)
            .unwrap_or_else(|error| panic!("{stage}: create lower surviving pane: {error}"));

        let before = live
            .snapshot()
            .unwrap_or_else(|error| panic!("{stage}: capture T-layout snapshot: {error}"));
        let surviving_terminals = [
            terminal_id(&before, &fixture.root_pane_id),
            terminal_id(&before, &survivor),
        ];
        assert_t_layout(
            &before,
            stage,
            &fixture.tab_id,
            &fixture.root_pane_id,
            &survivor,
            &terminated,
        );
        assert_eq!(
            focused_pane_id(&before),
            if focus_terminated_pane {
                terminated.as_str()
            } else {
                fixture.root_pane_id.as_str()
            },
            "{stage}: fixture should focus the intended pane"
        );
        live.link_plugin()
            .unwrap_or_else(|error| panic!("{stage}: link copied plugin: {error}"));
        let existing_log_ids = live
            .plugin_log_ids()
            .unwrap_or_else(|error| panic!("{stage}: list logs before termination: {error}"));
        let expected_event = if exit_process {
            "pane.exited"
        } else {
            "pane.closed"
        };

        if exit_process {
            live.exit_pane(&terminated)
                .unwrap_or_else(|error| panic!("{stage}: exit pane process: {error}"));
        } else {
            live.close_pane(&terminated)
                .unwrap_or_else(|error| panic!("{stage}: close pane through API: {error}"));
        }
        live.wait_for_pane_to_disappear(&terminated)
            .unwrap_or_else(|error| panic!("{stage}: wait for terminated pane: {error}"));
        live.wait_for_termination_hooks(&existing_log_ids, expected_event)
            .unwrap_or_else(|error| {
                panic!(
                    "{stage}: missing or failed {expected_event} termination balance hook: {error}"
                )
            });
        let after = live
            .snapshot()
            .unwrap_or_else(|error| panic!("{stage}: capture balanced snapshot: {error}"));

        let survivor_ids = [fixture.root_pane_id.as_str(), survivor.as_str()];
        assert_balanced_survivors(
            &after,
            stage,
            &fixture.tab_id,
            survivor_ids,
            &surviving_terminals,
        );
        let focused_survivor = focused_pane_id(&after);
        assert!(
            survivor_ids.contains(&focused_survivor.as_str()),
            "{stage}: termination should leave focus on a surviving pane, found {focused_survivor}"
        );
        live.run_and_wait_for_text(&focused_survivor, stage)
            .unwrap_or_else(|error| panic!("{stage}: next input failed: {error}"));
    }
}

#[test]
#[ignore = "requires Herdr and starts an isolated named session"]
fn new_workspace_actions_move_one_pane_or_the_whole_tab_and_keep_focus() {
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
    live.link_plugin().expect("link copied plugin");

    live.focus_workspace(&pane_source.workspace_id)
        .expect("focus pane source workspace");
    live.focus_tab(&pane_source.tab_id)
        .expect("focus pane source tab");
    let pane_before = live.snapshot().expect("capture pane pre-action snapshot");
    let detached_terminal_id = terminal_id(&pane_before, &detached);
    assert_eq!(focused_pane_id(&pane_before), detached);

    live.invoke_action("to-new-workspace")
        .expect("invoke to-new-workspace and wait for its log");

    let pane_after = live.snapshot().expect("capture pane post-action snapshot");
    let detached_pane = pane_by_terminal_id(&pane_after, &detached_terminal_id);
    let detached_pane_id = detached_pane["pane_id"].as_str().expect("detached pane ID");
    let pane_workspace_id = detached_pane["workspace_id"]
        .as_str()
        .expect("pane workspace ID");
    assert_ne!(pane_workspace_id, pane_source.workspace_id);
    assert_eq!(focused_pane_id(&pane_after), detached_pane_id);
    assert_eq!(
        pane_ids_in_tab(&pane_after, &pane_source.tab_id),
        BTreeSet::from([pane_source.root_pane_id.clone()])
    );
    assert_eq!(
        pane_ids_in_tab(&pane_after, detached_pane["tab_id"].as_str().unwrap()).len(),
        1
    );
    assert!(
        workspace_number(&pane_after, &pane_source.workspace_id)
            < workspace_number(&pane_after, pane_workspace_id)
    );
    assert!(
        workspace_number(&pane_after, pane_workspace_id)
            < workspace_number(&pane_after, &tab_source.workspace_id)
    );
    live.run_and_wait_for_text(detached_pane_id, "to-new-workspace")
        .expect("send input to detached pane");

    live.focus_workspace(&tab_source.workspace_id)
        .expect("focus tab source workspace");
    live.focus_tab(&moving.tab_id).expect("focus moving tab");
    live.focus_right_of(&moving.root_pane_id)
        .expect("focus middle moving pane");
    live.focus_right_of(&middle)
        .expect("focus right moving pane");
    let before = live.snapshot().expect("capture pre-action snapshot");
    let left_terminal_id = terminal_id(&before, &moving.root_pane_id);
    let middle_terminal_id = terminal_id(&before, &middle);
    let right_terminal_id = terminal_id(&before, &focused);
    assert_eq!(focused_pane_id(&before), focused);

    live.invoke_action("tab-to-new-workspace")
        .expect("invoke tab-to-new-workspace and wait for its log");

    let after = live.snapshot().expect("capture post-action snapshot");
    let left = pane_by_terminal_id(&after, &left_terminal_id);
    let middle = pane_by_terminal_id(&after, &middle_terminal_id);
    let right = pane_by_terminal_id(&after, &right_terminal_id);
    let created_workspace_id = left["workspace_id"].as_str().expect("created workspace ID");
    let created_tab_id = left["tab_id"].as_str().expect("created tab ID");
    let moved_ids: Vec<&str> = [left, middle, right]
        .map(|pane| pane["pane_id"].as_str().expect("moved pane ID"))
        .to_vec();
    assert_ne!(created_workspace_id, tab_source.workspace_id);
    for pane in [middle, right] {
        assert_eq!(pane["workspace_id"], created_workspace_id);
        assert_eq!(pane["tab_id"], created_tab_id);
    }
    assert_eq!(right["focused"], true);
    assert_eq!(focused_pane_id(&after), moved_ids[2]);
    assert_eq!(pane_ids_in_tab(&after, created_tab_id).len(), 3);
    assert!(pane_ids_in_tab(&after, &moving.tab_id).is_empty());
    assert_eq!(
        pane_ids_in_tab(&after, &tab_source.tab_id),
        BTreeSet::from([tab_source.root_pane_id.clone()])
    );
    assert_eq!(tab(&after, created_tab_id)["label"], "moving");
    assert!(
        workspace_number(&after, &tab_source.workspace_id)
            < workspace_number(&after, created_workspace_id)
    );
    assert!(
        workspace_number(&after, created_workspace_id)
            < workspace_number(&after, &trailing.workspace_id)
    );
    assert_eq!(panes_in_reading_order(&after, created_tab_id), moved_ids);
    live.run_and_wait_for_text(moved_ids[2], "tab-to-new-workspace")
        .expect("send input to focused whole-tab pane");
}

#[test]
#[ignore = "requires Herdr and starts an isolated named session"]
fn tab_workspace_actions_keep_the_tab_independent_on_a_round_trip() {
    let mut live = LiveHerdr::start().expect("start isolated Herdr session");
    let source = live.create_workspace("source").expect("create source");
    let moving = live
        .create_tab(&source.workspace_id, "moving")
        .expect("create moving tab");
    let middle = live
        .split_pane(&moving.root_pane_id, "right", true)
        .expect("split middle");
    let right = live
        .split_pane(&middle, "right", true)
        .expect("split right");
    let target = live.create_workspace("target").expect("create target");
    let target_last = live
        .create_tab(&target.workspace_id, "target-last")
        .expect("create last destination tab");
    live.focus_workspace(&source.workspace_id)
        .expect("focus source");
    live.focus_tab(&moving.tab_id).expect("focus moving tab");
    live.link_plugin().expect("link isolated plugin");
    let before = live.snapshot().expect("snapshot before moving");
    let terminals = [&moving.root_pane_id, &middle, &right].map(|id| terminal_id(&before, id));
    let workspace_order = |snapshot: &Value| {
        let mut workspaces: Vec<_> = snapshot["workspaces"]
            .as_array()
            .unwrap()
            .iter()
            .map(|workspace| {
                (
                    workspace["number"].as_u64().unwrap(),
                    workspace["workspace_id"].as_str().unwrap().to_owned(),
                )
            })
            .collect();
        workspaces.sort();
        workspaces
    };
    let original_order = workspace_order(&before);
    let mut previous_tab_id = moving.tab_id.clone();
    for (action, destination) in [
        ("tab-to-next-workspace", &target.workspace_id),
        ("tab-to-previous-workspace", &source.workspace_id),
    ] {
        let before_move = live.snapshot().expect("snapshot before action");
        let existing_tabs: Vec<_> = before_move["tabs"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|tab| tab["workspace_id"] == *destination)
            .map(|tab| tab["tab_id"].as_str().unwrap().to_owned())
            .collect();
        live.invoke_action(action)
            .expect("invoke registered tab move action");
        let after = live.snapshot().expect("snapshot after action");
        let panes = terminals
            .each_ref()
            .map(|id| pane_by_terminal_id(&after, id));
        let moved_ids = panes.map(|pane| pane["pane_id"].as_str().unwrap());
        let moved_tab = panes[0]["tab_id"].as_str().unwrap();
        for pane in panes {
            assert_eq!(pane["workspace_id"], *destination);
            assert_eq!(pane["tab_id"], moved_tab);
        }
        assert_eq!(focused_pane_id(&after), moved_ids[2]);
        assert_eq!(tab(&after, moved_tab)["label"], "moving");
        assert_eq!(panes_in_reading_order(&after, moved_tab), moved_ids);
        assert_eq!(pane_ids_in_tab(&after, moved_tab).len(), 3);
        assert!(pane_ids_in_tab(&after, &previous_tab_id).is_empty());
        let destination_tabs: Vec<_> = after["tabs"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|tab| tab["workspace_id"] == *destination)
            .map(|tab| tab["tab_id"].as_str().unwrap().to_owned())
            .collect();
        assert_eq!(
            destination_tabs,
            existing_tabs
                .into_iter()
                .chain([moved_tab.to_owned()])
                .collect::<Vec<_>>()
        );
        for fixture in [&source, &target, &target_last] {
            assert_eq!(
                pane_ids_in_tab(&after, &fixture.tab_id),
                BTreeSet::from([fixture.root_pane_id.clone()])
            );
        }
        assert_eq!(workspace_order(&after), original_order);
        previous_tab_id = moved_tab.to_owned();
    }
}

#[test]
#[ignore = "requires Herdr and starts an isolated named session"]
fn only_tab_actions_close_the_source_and_preserve_all_panes() {
    for (action, pane_count) in [
        ("tab-to-next-workspace", 1),
        ("tab-to-next-workspace", 2),
        ("tab-to-previous-workspace", 1),
        ("tab-to-previous-workspace", 2),
    ] {
        let mut live = LiveHerdr::start().expect("start isolated Herdr session");
        let first = live
            .create_workspace("first")
            .expect("create first workspace");
        let created = live
            .create_workspace("second")
            .expect("create second workspace");
        let (source, target) = if action == "tab-to-next-workspace" {
            (first, created)
        } else {
            (created, first)
        };
        live.focus_workspace(&source.workspace_id)
            .expect("focus source");
        live.run_json(["tab", "rename", &source.tab_id, "moving"])
            .expect("label source tab");
        let mut original_ids = vec![source.root_pane_id.clone()];
        if pane_count == 2 {
            original_ids.push(
                live.split_pane(&source.root_pane_id, "right", true)
                    .expect("split last pane"),
            );
        }
        live.link_plugin().expect("link isolated plugin");
        let before = live.snapshot().expect("snapshot before move");
        assert_eq!(before["workspaces"].as_array().unwrap().len(), 2);
        assert_eq!(focused_pane_id(&before), *original_ids.last().unwrap());
        assert_eq!(
            workspace_number(&before, &target.workspace_id),
            if action == "tab-to-next-workspace" {
                2
            } else {
                1
            }
        );
        let terminals: Vec<_> = original_ids
            .iter()
            .map(|id| terminal_id(&before, id))
            .collect();
        let target_terminal = terminal_id(&before, &target.root_pane_id);
        live.invoke_action(action).expect("move only tab");
        let after = live.snapshot().expect("snapshot after move");
        assert_eq!(after["workspaces"].as_array().unwrap().len(), 1);
        assert_eq!(after["workspaces"][0]["workspace_id"], target.workspace_id);
        assert_eq!(workspace_number(&after, &target.workspace_id), 1);
        assert!(
            !after["tabs"]
                .as_array()
                .unwrap()
                .iter()
                .any(|tab| tab["tab_id"] == source.tab_id)
        );
        assert!(
            !after["layouts"]
                .as_array()
                .unwrap()
                .iter()
                .any(|layout| layout["tab_id"] == source.tab_id)
        );
        let moved: Vec<_> = terminals
            .iter()
            .map(|id| pane_by_terminal_id(&after, id))
            .collect();
        let moved_tab = moved[0]["tab_id"].as_str().unwrap();
        let moved_ids: Vec<_> = moved
            .iter()
            .map(|pane| pane["pane_id"].as_str().unwrap())
            .collect();
        for (pane, original_id) in moved.iter().zip(&original_ids) {
            assert_eq!(pane["workspace_id"], target.workspace_id);
            assert_eq!(pane["tab_id"], moved_tab);
            assert_ne!(pane["pane_id"], *original_id);
        }
        assert_eq!(focused_pane_id(&after), *moved_ids.last().unwrap());
        assert_eq!(tab(&after, moved_tab)["label"], "moving");
        assert_eq!(panes_in_reading_order(&after, moved_tab), moved_ids);
        assert_eq!(pane_ids_in_tab(&after, moved_tab).len(), pane_count);
        assert_eq!(
            after["tabs"]
                .as_array()
                .unwrap()
                .iter()
                .map(|tab| tab["tab_id"].as_str().unwrap())
                .collect::<Vec<_>>(),
            [&target.tab_id, moved_tab]
        );
        assert_eq!(
            pane_ids_in_tab(&after, &target.tab_id),
            BTreeSet::from([target.root_pane_id.clone()])
        );
        assert_eq!(terminal_id(&after, &target.root_pane_id), target_terminal);
        if pane_count == 2 {
            let layout = after["layouts"]
                .as_array()
                .unwrap()
                .iter()
                .find(|layout| layout["tab_id"] == moved_tab)
                .unwrap();
            assert_eq!(layout["splits"][0]["ratio"], 0.5);
        }
        for no_op in ["tab-to-next-workspace", "tab-to-previous-workspace"] {
            live.invoke_action(no_op)
                .expect("ignore move with one workspace");
            let unchanged = live.snapshot().expect("snapshot after ignored move");
            for field in ["workspaces", "tabs", "layouts", "focused_pane_id"] {
                assert_eq!(unchanged[field], after[field], "{no_op}: {field}");
            }
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
        fs::write(&config_path, "onboarding = false\n")
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
        let marker = format!("termination-next-input-{}", stage.replace(' ', "-"));
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

    fn focus_right_of(&self, pane_id: &str) -> Result<(), String> {
        self.run_json(["pane", "focus", "--direction", "right", "--pane", pane_id])
            .map(|_| ())
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

    fn invoke_action_and_wait_for_balance(&self, action_id: &str) -> Result<(), String> {
        let existing_log_ids = self.plugin_log_ids()?;
        self.invoke_action(action_id)?;
        let deadline = Instant::now() + ACTION_TIMEOUT;
        let mut last_hooks = Vec::new();
        let mut unchanged_polls = 0;

        while Instant::now() < deadline {
            let hooks: Vec<_> = self
                .plugin_logs()?
                .into_iter()
                .filter(|log| {
                    log["event"] == "pane.focused"
                        && log["log_id"]
                            .as_str()
                            .is_some_and(|id| !existing_log_ids.contains(id))
                })
                .collect();
            if hooks.iter().any(|log| log["status"] == "failed") {
                return Err(format!("automatic balance hook failed: {hooks:?}"));
            }
            let all_finished = !hooks.is_empty()
                && hooks.iter().all(|log| {
                    log["status"] == "succeeded"
                        && log["finished_unix_ms"].as_u64().is_some()
                        && log["exit_code"].as_i64() == Some(0)
                });
            if hooks == last_hooks {
                unchanged_polls += 1;
            } else {
                last_hooks = hooks;
                unchanged_polls = 0;
            }
            if all_finished && unchanged_polls >= 2 {
                return Ok(());
            }
            std::thread::sleep(POLL_INTERVAL);
        }
        Err(format!(
            "automatic balance after {action_id} did not settle within {ACTION_TIMEOUT:?}; last hooks: {last_hooks:?}"
        ))
    }

    fn wait_for_termination_hooks(
        &self,
        existing_log_ids: &BTreeSet<String>,
        expected_event: &str,
    ) -> Result<(), String> {
        let deadline = Instant::now() + ACTION_TIMEOUT;
        let mut last_hooks = Vec::new();

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
                        .is_some_and(|id| !existing_log_ids.contains(id))
                })
                .collect();
            if hooks.iter().any(|log| log["status"] == "failed") {
                return Err(format!("automatic balance hook failed: {hooks:?}"));
            }
            let expected_finished = hooks.iter().any(|log| {
                log["event"] == expected_event
                    && log["status"] == "succeeded"
                    && log["finished_unix_ms"].as_u64().is_some()
                    && log["exit_code"].as_i64() == Some(0)
            });
            let all_observed_finished = hooks.iter().all(|log| {
                log["status"] == "succeeded"
                    && log["finished_unix_ms"].as_u64().is_some()
                    && log["exit_code"].as_i64() == Some(0)
            });
            if expected_finished && all_observed_finished {
                return Ok(());
            }
            last_hooks = hooks;
            std::thread::sleep(POLL_INTERVAL);
        }
        Err(format!(
            "{expected_event} hook did not finish within {ACTION_TIMEOUT:?}; last related hooks: {last_hooks:?}"
        ))
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

fn assert_vertical_move(
    live: &mut LiveHerdr,
    stage: &str,
    action_id: &str,
    moving_terminal_id: &str,
    route: (&Fixture, &Fixture),
    moving_first: bool,
    unchanged_tab: (&str, &BTreeSet<String>),
) {
    let (source, destination) = route;
    live.invoke_action_and_wait_for_balance(action_id)
        .unwrap_or_else(|error| {
            panic!("{stage}: {action_id} and automatic balancing failed: {error}")
        });
    let snapshot = live
        .snapshot()
        .unwrap_or_else(|error| panic!("{stage}: capture snapshot after {action_id}: {error}"));

    assert_two_pane_position(
        &snapshot,
        stage,
        action_id,
        moving_terminal_id,
        destination,
        moving_first,
    );
    assert_eq!(
        pane_ids_in_tab(&snapshot, &source.tab_id),
        BTreeSet::from([source.root_pane_id.clone()]),
        "{stage}: {action_id} should leave only the stationary pane in the source tab"
    );
    assert_eq!(
        pane_ids_in_tab(&snapshot, unchanged_tab.0),
        *unchanged_tab.1,
        "{stage}: {action_id} should preserve panes in the unaffected tab"
    );
}

fn assert_two_pane_position(
    snapshot: &Value,
    stage: &str,
    action_id: &str,
    moving_terminal_id: &str,
    destination: &Fixture,
    moving_first: bool,
) {
    let moved = pane_by_terminal_id(snapshot, moving_terminal_id);
    let moved_pane_id = moved["pane_id"].as_str().expect("moved pane ID");
    let stationary_pane_id = destination.root_pane_id.as_str();
    let expected_order = if moving_first {
        [moved_pane_id, stationary_pane_id]
    } else {
        [stationary_pane_id, moved_pane_id]
    };
    let expected_side = if moving_first { "left" } else { "right" };

    assert_eq!(
        moved["workspace_id"], destination.workspace_id,
        "{stage}: {action_id} should reach the destination workspace"
    );
    assert_eq!(
        moved["tab_id"], destination.tab_id,
        "{stage}: {action_id} should reach the destination tab"
    );
    assert_eq!(
        panes_in_reading_order(snapshot, &destination.tab_id),
        expected_order,
        "{stage}: {action_id} should place the moving pane on the {expected_side}"
    );
    assert_eq!(
        pane_ids_in_tab(snapshot, &destination.tab_id),
        BTreeSet::from([destination.root_pane_id.clone(), moved_pane_id.to_owned()]),
        "{stage}: {action_id} should preserve the destination pane set"
    );
    assert_eq!(
        moved["focused"], true,
        "{stage}: {action_id} should focus the moving pane"
    );
    assert_eq!(
        focused_pane_id(snapshot),
        moved_pane_id,
        "{stage}: {action_id} should keep server focus on the moving pane"
    );
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

fn assert_balanced_survivors(
    snapshot: &Value,
    stage: &str,
    tab_id: &str,
    survivor_ids: [&str; 2],
    terminal_ids: &[String; 2],
) {
    assert_eq!(
        pane_ids_in_tab(snapshot, tab_id),
        survivor_ids.iter().map(|id| (*id).to_owned()).collect(),
        "{stage}: termination should preserve the surviving pane IDs"
    );
    assert_eq!(
        terminal_ids
            .iter()
            .map(|id| {
                pane_by_terminal_id(snapshot, id)["pane_id"]
                    .as_str()
                    .expect("surviving pane ID")
            })
            .collect::<BTreeSet<_>>(),
        survivor_ids.into_iter().collect(),
        "{stage}: termination should preserve the surviving terminals"
    );
    let layout = tab_layout(snapshot, tab_id);
    assert_eq!(
        layout["splits"].as_array().map(Vec::len),
        Some(1),
        "{stage}: termination should leave one split for two survivors"
    );
    assert_eq!(
        layout["splits"][0]["direction"], "right",
        "{stage}: survivors remained vertical instead of a horizontal grid"
    );
    assert_eq!(
        layout["splits"][0]["ratio"], 0.5,
        "{stage}: horizontal grid should be 50:50"
    );
    let first = pane_rect(snapshot, tab_id, survivor_ids[0]);
    let second = pane_rect(snapshot, tab_id, survivor_ids[1]);
    assert_eq!(
        first["y"], second["y"],
        "{stage}: survivors should share a row"
    );
    assert_eq!(
        first["width"], second["width"],
        "{stage}: survivors should occupy equal columns"
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
