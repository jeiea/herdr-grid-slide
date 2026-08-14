#![cfg(unix)]

use std::collections::{HashMap, VecDeque};
use std::fs;
use std::io::{BufRead, BufReader, ErrorKind, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

/// One scripted socket reply. `Err` is serialized as herdr's `error.message`.
type Reply = Result<Value, String>;

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[test]
fn moves_wrap_by_visible_workspace_and_tab_number() {
    let herdr = FakeHerdr::new(standard_snapshot());

    for (scope, direction, workspace_id, tab_id, target_tab_id) in [
        ("workspace", "next", "workspace-3", "tab-3-1", "tab-1-1"),
        ("workspace", "previous", "workspace-1", "tab-1-1", "tab-3-1"),
        ("tab", "next", "workspace-2", "tab-2-3", "tab-2-1"),
        ("tab", "previous", "workspace-2", "tab-2-1", "tab-2-3"),
    ] {
        let run = herdr.run(workspace_id, tab_id, "pane-current", &[scope, direction]);

        run.assert_success();
        assert_eq!(
            run.requests,
            [
                call("session.snapshot", json!({})),
                call("pane.move", move_to_tab("pane-current", target_tab_id)),
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
        let herdr = FakeHerdr::new(snapshot(
            json!([layout(
                "tab-current",
                vec![
                    pane("pane-current", 40, 40, 40, 40),
                    pane(target_id, target_rect.0, target_rect.1, 40, 40),
                ],
            )]),
            json!([tab("workspace-1", "tab-current", 1)]),
            json!([workspace("workspace-1", "tab-current", 1)]),
        ));

        let run = herdr.run(
            "workspace-1",
            "tab-current",
            "pane-current",
            &["focus", direction],
        );

        run.assert_success();
        assert_eq!(run.requests, [snapshot_call(), focus_call(target_id)]);
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
        let herdr = FakeHerdr::new(snapshot(layouts, standard_tabs(), standard_workspaces()));

        let run = herdr.run(workspace_id, tab_id, "pane-current", &["focus", direction]);

        run.assert_success();
        assert_eq!(run.requests, [snapshot_call(), focus_call(expected)]);
    }
}

#[test]
fn focus_remembers_the_cross_axis_anchor_across_panes() {
    let herdr = anchor_herdr();

    for (pane_id, direction, expected) in [
        ("pane-a", "right", "pane-b"),
        ("pane-b", "down", "pane-c"),
        ("pane-c", "left", "pane-a"),
        ("pane-a", "right", "pane-c"),
    ] {
        let run = herdr.run("workspace-1", "tab-anchor", pane_id, &["focus", direction]);

        run.assert_success();
        assert_eq!(run.requests, [snapshot_call(), focus_call(expected)]);
    }

    let state_files: Vec<_> = fs::read_dir(&herdr.state_path)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(state_files, ["focus-anchor.json"]);
}

#[test]
fn focus_uses_live_snapshot_context_instead_of_stale_environment() {
    let herdr = FakeHerdr::new(focused(
        anchor_snapshot(),
        "workspace-1",
        "tab-anchor",
        "pane-b",
    ));

    let run = herdr.run(
        "workspace-stale",
        "tab-stale",
        "pane-stale",
        &["focus", "down"],
    );

    run.assert_success();
    assert_eq!(run.requests, [snapshot_call(), focus_call("pane-c")]);
}

#[test]
fn move_uses_live_snapshot_context_instead_of_stale_environment() {
    let herdr = FakeHerdr::new(focused(
        standard_snapshot(),
        "workspace-2",
        "tab-2-1",
        "pane-live",
    ));

    let run = herdr.run(
        "workspace-stale",
        "tab-stale",
        "pane-stale",
        &["tab", "next"],
    );

    run.assert_success();
    assert_eq!(
        run.requests,
        [
            snapshot_call(),
            call("pane.move", move_to_tab("pane-live", "tab-2-2")),
        ]
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
        let herdr = anchor_herdr();
        herdr.write_state(&state);

        let run = herdr.run("workspace-1", "tab-anchor", "pane-a", &["focus", "right"]);

        run.assert_success();
        assert_eq!(run.requests, [snapshot_call(), focus_call("pane-b")]);
    }
}

#[test]
fn focus_preserves_anchor_while_wrapping_across_tabs() {
    let herdr = FakeHerdr::new(snapshot(
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
    ));
    herdr.write_state(
        &json!({
            "paneId": "pane-source",
            "tabId": "tab-source",
            "workspaceId": "workspace-1",
            "x": 0.5,
            "y": 0.75,
        })
        .to_string(),
    );

    let run = herdr.run(
        "workspace-1",
        "tab-source",
        "pane-source",
        &["focus", "right"],
    );

    run.assert_success();
    assert_eq!(run.requests, [snapshot_call(), focus_call("pane-bottom")]);
}

#[test]
fn focus_preserves_anchor_while_wrapping_across_workspaces() {
    let herdr = FakeHerdr::new(snapshot(
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
    ));
    herdr.write_state(
        &json!({
            "paneId": "pane-source",
            "tabId": "tab-source",
            "workspaceId": "workspace-source",
            "x": 0.75,
            "y": 0.5,
        })
        .to_string(),
    );

    let run = herdr.run(
        "workspace-source",
        "tab-source",
        "pane-source",
        &["focus", "down"],
    );

    run.assert_success();
    assert_eq!(run.requests, [snapshot_call(), focus_call("pane-right")]);
}

#[test]
fn reports_the_herdr_error_that_rejected_an_action() {
    let herdr = FakeHerdr::new(standard_snapshot())
        .with_replies("pane.move", [Err("target tab is gone".to_owned())]);

    let run = herdr.run("workspace-2", "tab-2-1", "pane-current", &["tab", "next"]);

    assert_eq!(
        run.assert_failure(),
        "pane.move failed: target tab is gone\n"
    );
    assert_eq!(
        run.requests,
        [
            snapshot_call(),
            call("pane.move", move_to_tab("pane-current", "tab-2-2")),
        ]
    );
}

#[test]
fn new_pane_follows_the_only_split_direction_in_the_tab() {
    // The focused pane is far taller than wide, but every split in the tab runs
    // rightwards, so the new pane joins that row instead of starting a column.
    let herdr = FakeHerdr::new(new_pane_snapshot(vec![
        pane("pane-a", 0, 0, 20, 100),
        pane("pane-b", 20, 0, 80, 100),
    ]))
    .with_replies(
        "layout.export",
        [
            export_reply(split("right", leaf("pane-a"), leaf("pane-b"))),
            export_reply(split(
                "right",
                split("right", leaf("pane-a"), leaf("pane-new")),
                leaf("pane-b"),
            )),
        ],
    )
    .with_replies("pane.split", [split_reply("pane-new")]);

    let run = run_new_pane(&herdr, "pane-a");

    run.assert_success();
    assert_eq!(
        run.requests,
        [
            snapshot_call(),
            export_call(),
            split_call("pane-a", "right"),
            export_call(),
            ratio_call(&[], 2.0 / 3.0),
            ratio_call(&[false], 0.5),
        ]
    );
}

#[test]
fn new_pane_splits_a_wide_pane_sideways_when_directions_are_mixed() {
    let herdr = FakeHerdr::new(new_pane_snapshot(vec![
        pane("pane-a", 0, 0, 100, 40),
        pane("pane-b", 100, 0, 100, 40),
        pane("pane-c", 0, 40, 200, 40),
    ]))
    .with_replies(
        "layout.export",
        [
            export_reply(split(
                "down",
                split("right", leaf("pane-a"), leaf("pane-b")),
                leaf("pane-c"),
            )),
            export_reply(split(
                "down",
                split(
                    "right",
                    split("right", leaf("pane-a"), leaf("pane-new")),
                    leaf("pane-b"),
                ),
                leaf("pane-c"),
            )),
        ],
    )
    .with_replies("pane.split", [split_reply("pane-new")]);

    let run = run_new_pane(&herdr, "pane-a");

    run.assert_success();
    assert_eq!(
        run.requests,
        [
            snapshot_call(),
            export_call(),
            split_call("pane-a", "right"),
            export_call(),
            ratio_call(&[false], 2.0 / 3.0),
            ratio_call(&[false, false], 0.5),
        ]
    );
}

#[test]
fn new_pane_splits_the_lone_pane_of_a_tab_by_its_shape() {
    let herdr = FakeHerdr::new(new_pane_snapshot(vec![pane("pane-a", 0, 0, 100, 30)]))
        .with_replies(
            "layout.export",
            [
                export_reply(leaf("pane-a")),
                export_reply(split("right", leaf("pane-a"), leaf("pane-new"))),
            ],
        )
        .with_replies("pane.split", [split_reply("pane-new")]);

    let run = run_new_pane(&herdr, "pane-a");

    run.assert_success();
    assert_eq!(
        run.requests,
        [
            snapshot_call(),
            export_call(),
            split_call("pane-a", "right"),
            export_call(),
            ratio_call(&[], 0.5),
        ]
    );
}

#[test]
fn new_pane_splits_downward_on_the_two_to_one_boundary() {
    // Cells are about twice as tall as wide, so a pane only counts as wide once
    // its width passes twice its height. The boundary itself splits downwards.
    let herdr = FakeHerdr::new(new_pane_snapshot(vec![pane("pane-a", 0, 0, 80, 40)]))
        .with_replies(
            "layout.export",
            [
                export_reply(leaf("pane-a")),
                export_reply(split("down", leaf("pane-a"), leaf("pane-new"))),
            ],
        )
        .with_replies("pane.split", [split_reply("pane-new")]);

    let run = run_new_pane(&herdr, "pane-a");

    run.assert_success();
    assert_eq!(
        run.requests,
        [
            snapshot_call(),
            export_call(),
            split_call("pane-a", "down"),
            export_call(),
            ratio_call(&[], 0.5),
        ]
    );
}

#[test]
fn new_pane_evens_out_every_split_of_the_nested_run() {
    let herdr = FakeHerdr::new(new_pane_snapshot(vec![
        pane("pane-a", 0, 0, 50, 100),
        pane("pane-b", 50, 0, 25, 100),
        pane("pane-c", 75, 0, 25, 100),
    ]))
    .with_replies(
        "layout.export",
        [
            export_reply(split(
                "right",
                split("right", leaf("pane-a"), leaf("pane-b")),
                leaf("pane-c"),
            )),
            export_reply(split(
                "right",
                split(
                    "right",
                    split("right", leaf("pane-a"), leaf("pane-new")),
                    leaf("pane-b"),
                ),
                leaf("pane-c"),
            )),
        ],
    )
    .with_replies("pane.split", [split_reply("pane-new")]);

    let run = run_new_pane(&herdr, "pane-a");

    run.assert_success();
    assert_eq!(
        run.requests,
        [
            snapshot_call(),
            export_call(),
            split_call("pane-a", "right"),
            export_call(),
            ratio_call(&[], 0.75),
            ratio_call(&[false], 2.0 / 3.0),
            ratio_call(&[false, false], 0.5),
        ]
    );
}

#[test]
fn new_pane_counts_a_crosswise_subtree_as_one_slot() {
    // The right half is a column of two panes. It stays one slot of the row and
    // keeps its own ratio, so the row splits 2:1 rather than 2:2.
    let herdr = FakeHerdr::new(new_pane_snapshot(vec![
        pane("pane-a", 0, 0, 100, 40),
        pane("pane-b", 100, 0, 100, 20),
        pane("pane-c", 100, 20, 100, 20),
    ]))
    .with_replies(
        "layout.export",
        [
            export_reply(split(
                "right",
                leaf("pane-a"),
                split("down", leaf("pane-b"), leaf("pane-c")),
            )),
            export_reply(split(
                "right",
                split("right", leaf("pane-a"), leaf("pane-new")),
                split("down", leaf("pane-b"), leaf("pane-c")),
            )),
        ],
    )
    .with_replies("pane.split", [split_reply("pane-new")]);

    let run = run_new_pane(&herdr, "pane-a");

    run.assert_success();
    assert_eq!(
        run.requests,
        [
            snapshot_call(),
            export_call(),
            split_call("pane-a", "right"),
            export_call(),
            ratio_call(&[], 2.0 / 3.0),
            ratio_call(&[false], 0.5),
        ]
    );
}

#[test]
fn new_pane_stops_before_splitting_a_zoomed_tab() {
    let herdr = FakeHerdr::new(new_pane_snapshot(vec![
        pane("pane-a", 0, 0, 100, 40),
        pane("pane-b", 100, 0, 100, 40),
    ]))
    .with_replies(
        "layout.export",
        [zoomed_export_reply(split(
            "right",
            leaf("pane-a"),
            leaf("pane-b"),
        ))],
    );

    let run = run_new_pane(&herdr, "pane-a");

    assert_eq!(
        run.assert_failure(),
        "tab-new is zoomed; unzoom it before adding a pane\n"
    );
    assert_eq!(run.requests, [snapshot_call(), export_call()]);
}

#[test]
fn new_pane_stops_before_splitting_when_snapshot_and_layout_disagree() {
    let herdr = FakeHerdr::new(new_pane_snapshot(vec![
        pane("pane-a", 0, 0, 100, 40),
        pane("pane-b", 100, 0, 100, 40),
    ]))
    .with_replies(
        "layout.export",
        [export_reply(split(
            "right",
            leaf("pane-a"),
            leaf("pane-late"),
        ))],
    );

    let run = run_new_pane(&herdr, "pane-a");

    assert_eq!(
        run.assert_failure(),
        "tab-new changed while it was being read; try again\n"
    );
    assert_eq!(run.requests, [snapshot_call(), export_call()]);
}

#[test]
fn new_pane_leaves_the_ratios_alone_when_the_created_pane_is_missing() {
    let herdr = FakeHerdr::new(new_pane_snapshot(vec![
        pane("pane-a", 0, 0, 100, 40),
        pane("pane-b", 100, 0, 100, 40),
    ]))
    .with_replies(
        "layout.export",
        [
            export_reply(split("right", leaf("pane-a"), leaf("pane-b"))),
            export_reply(split("right", leaf("pane-a"), leaf("pane-b"))),
        ],
    )
    .with_replies("pane.split", [split_reply("pane-new")]);

    let run = run_new_pane(&herdr, "pane-a");

    assert_eq!(
        run.assert_failure(),
        "created pane-new but it is missing from tab-new; left the sizes alone\n"
    );
    assert_eq!(
        run.requests,
        [
            snapshot_call(),
            export_call(),
            split_call("pane-a", "right"),
            export_call(),
        ]
    );
}

#[test]
fn new_pane_reports_a_created_pane_that_could_not_be_evened_out() {
    let herdr = FakeHerdr::new(new_pane_snapshot(vec![
        pane("pane-a", 0, 0, 100, 40),
        pane("pane-b", 100, 0, 100, 40),
    ]))
    .with_replies(
        "layout.export",
        [
            export_reply(split("right", leaf("pane-a"), leaf("pane-b"))),
            export_reply(split(
                "right",
                split("right", leaf("pane-a"), leaf("pane-new")),
                leaf("pane-b"),
            )),
        ],
    )
    .with_replies("pane.split", [split_reply("pane-new")])
    .with_replies(
        "layout.set_split_ratio",
        [Err("path is out of date".to_owned())],
    );

    let run = run_new_pane(&herdr, "pane-a");

    assert_eq!(
        run.assert_failure(),
        "created pane-new but could not even out the sizes: \
         layout.set_split_ratio failed: path is out of date\n"
    );
    assert_eq!(
        run.requests,
        [
            snapshot_call(),
            export_call(),
            split_call("pane-a", "right"),
            export_call(),
            ratio_call(&[], 2.0 / 3.0),
        ]
    );
}

#[test]
fn new_pane_evens_out_a_run_that_leans_on_its_second_side() {
    // Splitting the last pane of a row grows the run down the second branch, so the
    // shares run 1:3, then 1:2, then 1:1 from the root down.
    let herdr = FakeHerdr::new(new_pane_snapshot(vec![
        pane("pane-a", 0, 0, 34, 100),
        pane("pane-b", 34, 0, 33, 100),
        pane("pane-c", 67, 0, 33, 100),
    ]))
    .with_replies(
        "layout.export",
        [
            export_reply(split(
                "right",
                leaf("pane-a"),
                split("right", leaf("pane-b"), leaf("pane-c")),
            )),
            export_reply(split(
                "right",
                leaf("pane-a"),
                split(
                    "right",
                    leaf("pane-b"),
                    split("right", leaf("pane-c"), leaf("pane-new")),
                ),
            )),
        ],
    )
    .with_replies("pane.split", [split_reply("pane-new")]);

    let run = run_new_pane(&herdr, "pane-c");

    run.assert_success();
    assert_eq!(
        run.requests,
        [
            snapshot_call(),
            export_call(),
            split_call("pane-c", "right"),
            export_call(),
            ratio_call(&[], 0.25),
            ratio_call(&[true], 1.0 / 3.0),
            ratio_call(&[true, true], 0.5),
        ]
    );
}

#[test]
fn new_pane_reports_a_created_pane_it_could_not_read_back() {
    let herdr = FakeHerdr::new(new_pane_snapshot(vec![
        pane("pane-a", 0, 0, 100, 40),
        pane("pane-b", 100, 0, 100, 40),
    ]))
    .with_replies(
        "layout.export",
        [
            export_reply(split("right", leaf("pane-a"), leaf("pane-b"))),
            Err("tab is gone".to_owned()),
        ],
    )
    .with_replies("pane.split", [split_reply("pane-new")]);

    let run = run_new_pane(&herdr, "pane-a");

    assert_eq!(
        run.assert_failure(),
        "created pane-new but could not read tab-new back: layout.export failed: tab is gone\n"
    );
    assert_eq!(
        run.requests,
        [
            snapshot_call(),
            export_call(),
            split_call("pane-a", "right"),
            export_call(),
        ]
    );
}

#[test]
fn new_pane_stops_evening_out_at_the_first_rejected_ratio() {
    let herdr = FakeHerdr::new(new_pane_snapshot(vec![
        pane("pane-a", 0, 0, 50, 100),
        pane("pane-b", 50, 0, 25, 100),
        pane("pane-c", 75, 0, 25, 100),
    ]))
    .with_replies(
        "layout.export",
        [
            export_reply(split(
                "right",
                split("right", leaf("pane-a"), leaf("pane-b")),
                leaf("pane-c"),
            )),
            export_reply(split(
                "right",
                split(
                    "right",
                    split("right", leaf("pane-a"), leaf("pane-new")),
                    leaf("pane-b"),
                ),
                leaf("pane-c"),
            )),
        ],
    )
    .with_replies("pane.split", [split_reply("pane-new")])
    .with_replies(
        "layout.set_split_ratio",
        [ratio_reply(), Err("path is out of date".to_owned())],
    );

    let run = run_new_pane(&herdr, "pane-a");

    assert_eq!(
        run.assert_failure(),
        "created pane-new but could not even out the sizes: \
         layout.set_split_ratio failed: path is out of date\n"
    );
    // The third ratio of the run is never attempted.
    assert_eq!(
        run.requests,
        [
            snapshot_call(),
            export_call(),
            split_call("pane-a", "right"),
            export_call(),
            ratio_call(&[], 0.75),
            ratio_call(&[false], 2.0 / 3.0),
        ]
    );
}

/// A fake herdr socket server. It speaks the real protocol -- one connection per
/// request, one JSON line in and one out -- and records every request in order.
struct FakeHerdr {
    directory: PathBuf,
    replies: HashMap<String, VecDeque<Reply>>,
    snapshot: Value,
    state_path: PathBuf,
}

/// One plugin execution: how the process ended and what it asked herdr for.
struct Run {
    output: Output,
    requests: Vec<Call>,
}

/// One recorded socket request.
#[derive(Clone, Debug, PartialEq)]
struct Call {
    method: String,
    params: Value,
}

impl FakeHerdr {
    fn new(snapshot: Value) -> Self {
        let directory = temp_dir();
        fs::create_dir_all(&directory).unwrap();
        Self {
            replies: HashMap::new(),
            snapshot,
            state_path: directory.join("state"),
            directory,
        }
    }

    /// Answers successive calls to `method` with these replies in order, repeating
    /// the last one once the queue runs dry, so a scenario only lists the replies
    /// that differ. Every [`FakeHerdr::run`] starts over from this initial queue,
    /// which keeps repeated runs on one fake independent. Methods left unscripted
    /// fall back to [`default_reply`].
    fn with_replies(mut self, method: &str, replies: impl IntoIterator<Item = Reply>) -> Self {
        self.replies
            .insert(method.to_owned(), replies.into_iter().collect());
        self
    }

    fn run(&self, workspace_id: &str, tab_id: &str, pane_id: &str, args: &[&str]) -> Run {
        let socket_path = self.directory.join("herdr.sock");
        let stop = Arc::new(AtomicBool::new(false));
        let server = self.serve(&socket_path, Arc::clone(&stop));
        let output = Command::new(env!("CARGO_BIN_EXE_herdr-move-pane"))
            .args(args)
            .env("HERDR_PANE_ID", pane_id)
            .env("HERDR_PLUGIN_STATE_DIR", &self.state_path)
            .env("HERDR_SOCKET_PATH", &socket_path)
            .env("HERDR_TAB_ID", tab_id)
            .env("HERDR_WORKSPACE_ID", workspace_id)
            .output()
            .unwrap();
        stop.store(true, Ordering::Relaxed);
        let requests = server.join().unwrap();
        Run { output, requests }
    }

    /// Serves requests until `stop` is set, then hands back everything it saw.
    fn serve(&self, socket_path: &Path, stop: Arc<AtomicBool>) -> thread::JoinHandle<Vec<Call>> {
        let _ = fs::remove_file(socket_path);
        let listener = UnixListener::bind(socket_path).unwrap();
        listener.set_nonblocking(true).unwrap();
        let mut replies = self.replies.clone();
        let snapshot = self.snapshot.clone();
        thread::spawn(move || {
            let mut requests = Vec::new();
            loop {
                match listener.accept() {
                    Ok((connection, _)) => {
                        connection.set_nonblocking(false).unwrap();
                        requests.push(serve_request(connection, &mut replies, &snapshot));
                    }
                    Err(error) if error.kind() == ErrorKind::WouldBlock => {
                        if stop.load(Ordering::Relaxed) {
                            return requests;
                        }
                        thread::sleep(Duration::from_millis(1));
                    }
                    Err(error) => panic!("accept failed: {error}"),
                }
            }
        })
    }

    fn write_state(&self, state: &str) {
        fs::create_dir_all(&self.state_path).unwrap();
        fs::write(self.state_path.join("focus-anchor.json"), state).unwrap();
    }
}

impl Drop for FakeHerdr {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.directory);
    }
}

impl Run {
    fn assert_success(&self) {
        assert!(self.output.status.success(), "stderr: {}", self.stderr());
    }

    fn assert_failure(&self) -> String {
        assert!(!self.output.status.success(), "stdout: {}", self.stdout());
        self.stderr()
    }

    fn stdout(&self) -> String {
        String::from_utf8_lossy(&self.output.stdout).into_owned()
    }

    fn stderr(&self) -> String {
        String::from_utf8_lossy(&self.output.stderr).into_owned()
    }
}

fn serve_request(
    connection: UnixStream,
    replies: &mut HashMap<String, VecDeque<Reply>>,
    snapshot: &Value,
) -> Call {
    let mut reader = BufReader::new(connection.try_clone().unwrap());
    let mut writer = connection;
    let mut line = String::new();
    reader.read_line(&mut line).unwrap();
    let request: Value = serde_json::from_str(&line).unwrap();
    let call = call(
        request["method"].as_str().unwrap(),
        request["params"].clone(),
    );
    let reply = next_reply(replies, &call.method).unwrap_or_else(|| default_reply(&call, snapshot));
    let response = match reply {
        Ok(result) => json!({"id": request["id"], "result": result}),
        Err(message) => json!({"id": request["id"], "error": {"message": message}}),
    };
    writeln!(writer, "{response}").unwrap();
    call
}

fn next_reply(replies: &mut HashMap<String, VecDeque<Reply>>, method: &str) -> Option<Reply> {
    let queue = replies.get_mut(method)?;
    if queue.len() > 1 {
        queue.pop_front()
    } else {
        queue.front().cloned()
    }
}

/// Absorbs the requests every scenario answers the same way, shaped like the live
/// herdr 0.8 responses -- `pane.move` reports its outcome under `move_result`, where
/// `changed`, `reason` and `created_tab` live, and `layout.set_split_ratio` echoes a
/// layout the plugin never reads, so only its acknowledgement is modelled. Any other
/// method must be scripted with [`FakeHerdr::with_replies`]; an unscripted one fails
/// loudly instead of succeeding on an invented response.
fn default_reply(call: &Call, snapshot: &Value) -> Reply {
    Ok(match call.method.as_str() {
        "session.snapshot" => json!({"type": "session_snapshot", "snapshot": snapshot}),
        "pane.focus" => json!({"pane": {"pane_id": call.params["pane_id"]}}),
        "pane.move" => json!({"type": "pane_move", "move_result": {"changed": true}}),
        "layout.set_split_ratio" => json!({"type": "layout_split_ratio_set"}),
        method => panic!("unscripted socket method: {method}"),
    })
}

fn anchor_herdr() -> FakeHerdr {
    FakeHerdr::new(anchor_snapshot())
}

fn anchor_snapshot() -> Value {
    snapshot(
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

fn standard_snapshot() -> Value {
    snapshot(standard_layouts(), standard_tabs(), standard_workspaces())
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

/// Every `new-pane` scenario runs the same command against the same tab, so only
/// the tree and the focused pane change between them.
fn run_new_pane(herdr: &FakeHerdr, pane_id: &str) -> Run {
    herdr.run("workspace-1", "tab-new", pane_id, &["new-pane"])
}

/// A tab holding exactly the given panes, which every `new-pane` scenario builds on.
fn new_pane_snapshot(panes: Vec<Value>) -> Value {
    snapshot(
        json!([layout("tab-new", panes)]),
        json!([tab("workspace-1", "tab-new", 1)]),
        json!([workspace("workspace-1", "tab-new", 1)]),
    )
}

fn export_reply(root: Value) -> Reply {
    layout_export_reply(root, false)
}

fn zoomed_export_reply(root: Value) -> Reply {
    layout_export_reply(root, true)
}

fn layout_export_reply(root: Value, zoomed: bool) -> Reply {
    Ok(json!({
        "type": "layout_export",
        "layout": {
            "workspace_id": "workspace-1",
            "tab_id": "tab-new",
            "zoomed": zoomed,
            "focused_pane_id": "pane-a",
            "root": root,
        },
    }))
}

fn split_reply(pane_id: &str) -> Reply {
    Ok(json!({"type": "pane_info", "pane": {"pane_id": pane_id}}))
}

fn ratio_reply() -> Reply {
    Ok(json!({"type": "layout_split_ratio_set"}))
}

/// The ratio a fake tree carries is irrelevant: `new-pane` recomputes every ratio
/// it touches from the shape of the tree.
fn split(direction: &str, first: Value, second: Value) -> Value {
    json!({
        "type": "split",
        "direction": direction,
        "ratio": 0.5,
        "first": first,
        "second": second,
    })
}

fn leaf(pane_id: &str) -> Value {
    json!({"type": "pane", "pane_id": pane_id})
}

fn export_call() -> Call {
    call("layout.export", json!({"tab_id": "tab-new"}))
}

fn split_call(target_pane_id: &str, direction: &str) -> Call {
    call(
        "pane.split",
        json!({
            "target_pane_id": target_pane_id,
            "direction": direction,
            "ratio": 0.5,
            "focus": true,
        }),
    )
}

fn ratio_call(path: &[bool], ratio: f64) -> Call {
    call(
        "layout.set_split_ratio",
        json!({"tab_id": "tab-new", "path": path, "ratio": ratio}),
    )
}

fn snapshot(layouts: Value, tabs: Value, workspaces: Value) -> Value {
    json!({"layouts": layouts, "tabs": tabs, "workspaces": workspaces})
}

/// A snapshot where herdr reports live focus, which the plugin trusts over the
/// environment variables it was launched with.
fn focused(mut snapshot: Value, workspace_id: &str, tab_id: &str, pane_id: &str) -> Value {
    snapshot["focused_workspace_id"] = workspace_id.into();
    snapshot["focused_tab_id"] = tab_id.into();
    snapshot["focused_pane_id"] = pane_id.into();
    snapshot
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

fn snapshot_call() -> Call {
    call("session.snapshot", json!({}))
}

fn focus_call(pane_id: &str) -> Call {
    call("pane.focus", json!({"pane_id": pane_id}))
}

fn move_to_tab(pane_id: &str, tab_id: &str) -> Value {
    json!({
        "pane_id": pane_id,
        "destination": {"type": "tab", "tab_id": tab_id, "split": "right", "ratio": 0.5},
        "focus": true,
    })
}

fn call(method: &str, params: Value) -> Call {
    Call {
        method: method.to_owned(),
        params,
    }
}

fn temp_dir() -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("hmp-{}-{unique}-{sequence}", std::process::id()))
}
