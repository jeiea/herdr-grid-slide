#![cfg(unix)]

use std::collections::{HashMap, VecDeque};
use std::fs;
use std::io::{BufRead, BufReader, ErrorKind, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::process::{Command as ProcessCommand, Output};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

/// One scripted socket reply. `Err` is serialized as herdr's `error.message`.
type Reply = Result<Value, String>;

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[test]
fn moves_the_focused_pane_to_a_new_workspace_immediately_after_its_source() {
    // Snapshot array order differs from visible workspace number order, so the
    // source's visible insertion slot is 2 rather than its array slot 1.
    let herdr = FakeHerdr::new(focused(
        snapshot(
            json!([
                layout(
                    "tab-source",
                    vec![
                        pane("pane-live", 0, 0, 50, 100),
                        pane("pane-stays", 50, 0, 50, 100),
                    ],
                ),
                layout("tab-first", vec![pane("pane-first", 0, 0, 100, 100)]),
                layout("tab-last", vec![pane("pane-last", 0, 0, 100, 100)]),
            ]),
            json!([
                tab("workspace-source", "tab-source", 1),
                tab("workspace-first", "tab-first", 1),
                tab("workspace-last", "tab-last", 1),
            ]),
            json!([
                workspace("workspace-source", "tab-source", 2),
                workspace("workspace-first", "tab-first", 1),
                workspace("workspace-last", "tab-last", 3),
            ]),
        ),
        "workspace-source",
        "tab-source",
        "pane-live",
    ))
    .with_replies("pane.move", [new_workspace_reply("workspace-created")]);

    let run = herdr.run(
        "workspace-stale",
        "tab-stale",
        "pane-stale",
        &["to-new-workspace"],
    );

    run.assert_success();
    assert_eq!(
        run.requests,
        [
            snapshot_call(),
            move_to_new_workspace_call("pane-live"),
            workspace_move_call("workspace-created", 2),
        ]
    );
}

#[test]
fn leaves_a_new_workspace_at_the_end_when_its_source_was_last() {
    let herdr = FakeHerdr::new(focused(
        snapshot(
            json!([
                layout("tab-first", vec![pane("pane-first", 0, 0, 100, 100)]),
                layout(
                    "tab-source",
                    vec![
                        pane("pane-moving", 0, 0, 50, 100),
                        pane("pane-stays", 50, 0, 50, 100),
                    ],
                ),
            ]),
            json!([
                tab("workspace-first", "tab-first", 1),
                tab("workspace-source", "tab-source", 1),
            ]),
            json!([
                workspace("workspace-first", "tab-first", 1),
                workspace("workspace-source", "tab-source", 2),
            ]),
        ),
        "workspace-source",
        "tab-source",
        "pane-moving",
    ))
    .with_replies("pane.move", [move_reply()]);

    let run = herdr.run(
        "workspace-source",
        "tab-source",
        "pane-moving",
        &["to-new-workspace"],
    );

    run.assert_success();
    assert_eq!(
        run.requests,
        [snapshot_call(), move_to_new_workspace_call("pane-moving")]
    );
}

#[test]
fn ignores_moving_the_last_pane_out_of_its_workspace() {
    let herdr = FakeHerdr::new(focused(
        snapshot(
            json!([layout(
                "tab-source",
                vec![pane("pane-only", 0, 0, 100, 100)],
            )]),
            json!([tab("workspace-source", "tab-source", 1)]),
            json!([workspace("workspace-source", "tab-source", 1)]),
        ),
        "workspace-source",
        "tab-source",
        "pane-only",
    ));

    let run = herdr.run(
        "workspace-source",
        "tab-source",
        "pane-only",
        &["to-new-workspace"],
    );

    run.assert_success();
    assert_eq!(run.requests, [snapshot_call()]);
}

#[test]
fn moves_a_tabs_last_pane_when_another_tab_remains_in_the_workspace() {
    let herdr = FakeHerdr::new(focused(
        snapshot(
            json!([
                layout("tab-source", vec![pane("pane-moving", 0, 0, 100, 100)],),
                layout("tab-stays", vec![pane("pane-stays", 0, 0, 100, 100)],),
                layout("tab-next", vec![pane("pane-next", 0, 0, 100, 100)]),
            ]),
            json!([
                tab("workspace-source", "tab-source", 1),
                tab("workspace-source", "tab-stays", 2),
                tab("workspace-next", "tab-next", 1),
            ]),
            json!([
                workspace("workspace-source", "tab-source", 1),
                workspace("workspace-next", "tab-next", 2),
            ]),
        ),
        "workspace-source",
        "tab-source",
        "pane-moving",
    ))
    .with_replies("pane.move", [new_workspace_reply("workspace-created")]);

    let run = herdr.run(
        "workspace-source",
        "tab-source",
        "pane-moving",
        &["to-new-workspace"],
    );

    run.assert_success();
    assert_eq!(
        run.requests,
        [
            snapshot_call(),
            move_to_new_workspace_call("pane-moving"),
            workspace_move_call("workspace-created", 1),
        ]
    );
}

#[test]
fn stops_when_herdr_declines_the_new_workspace_move() {
    let herdr = FakeHerdr::new(focused(
        standard_snapshot(),
        "workspace-2",
        "tab-2-1",
        "pane-2-1-top-left",
    ))
    .with_replies("pane.move", [no_op_move_reply("source_changed")]);

    let run = herdr.run(
        "workspace-2",
        "tab-2-1",
        "pane-2-1-top-left",
        &["to-new-workspace"],
    );

    run.assert_success();
    assert_eq!(
        run.requests,
        [
            snapshot_call(),
            move_to_new_workspace_call("pane-2-1-top-left"),
        ]
    );
}

#[test]
fn reports_when_a_moved_pane_response_omits_the_created_workspace_id() {
    let herdr = FakeHerdr::new(focused(
        standard_snapshot(),
        "workspace-2",
        "tab-2-1",
        "pane-2-1-top-left",
    ))
    .with_replies("pane.move", [move_reply()]);

    let run = herdr.run(
        "workspace-2",
        "tab-2-1",
        "pane-2-1-top-left",
        &["to-new-workspace"],
    );

    assert_eq!(
        run.assert_failure(),
        "pane moved to a new workspace, but herdr api pane.move response is missing the created workspace id\n"
    );
    assert_eq!(
        run.requests,
        [
            snapshot_call(),
            move_to_new_workspace_call("pane-2-1-top-left"),
        ]
    );
}

#[test]
fn reports_when_the_pane_moved_but_the_new_workspace_could_not_be_repositioned() {
    let herdr = FakeHerdr::new(focused(
        standard_snapshot(),
        "workspace-2",
        "tab-2-1",
        "pane-2-1-top-left",
    ))
    .with_replies("pane.move", [new_workspace_reply("workspace-created")])
    .with_replies(
        "workspace.move",
        [Err("insert_index 2 is out of bounds".to_owned())],
    );

    let run = herdr.run(
        "workspace-2",
        "tab-2-1",
        "pane-2-1-top-left",
        &["to-new-workspace"],
    );

    assert_eq!(
        run.assert_failure(),
        "pane moved to a new workspace, but positioning workspace-created after workspace-2 failed: workspace.move failed: insert_index 2 is out of bounds\n"
    );
    assert_eq!(
        run.requests,
        [
            snapshot_call(),
            move_to_new_workspace_call("pane-2-1-top-left"),
            workspace_move_call("workspace-created", 2),
        ]
    );
}

#[test]
fn moves_the_focused_pane_to_a_new_tab_immediately_after_its_source() {
    // A tab in an earlier workspace makes a session-wide index differ from the
    // insertion slot inside workspace-2.
    let herdr = FakeHerdr::new(focused(
        snapshot(
            json!([
                layout("tab-1", vec![pane("pane-1", 0, 0, 100, 100)]),
                layout("tab-2-a", vec![pane("pane-a", 0, 0, 100, 100)]),
                layout(
                    "tab-2-source",
                    vec![
                        pane("pane-live", 0, 0, 50, 100),
                        pane("pane-stays", 50, 0, 50, 100),
                    ],
                ),
                layout("tab-2-c", vec![pane("pane-c", 0, 0, 100, 100)]),
            ]),
            json!([
                tab("workspace-1", "tab-1", 1),
                tab("workspace-2", "tab-2-a", 1),
                tab("workspace-2", "tab-2-source", 2),
                tab("workspace-2", "tab-2-c", 3),
            ]),
            json!([
                workspace("workspace-1", "tab-1", 1),
                workspace("workspace-2", "tab-2-source", 2),
            ]),
        ),
        "workspace-2",
        "tab-2-source",
        "pane-live",
    ))
    .with_replies("pane.move", [new_tab_reply("tab-created")])
    .with_replies("tab.move", [tab_move_reply()]);

    let run = herdr.run(
        "workspace-stale",
        "tab-stale",
        "pane-stale",
        &["to-new-tab"],
    );

    run.assert_success();
    assert_eq!(
        run.requests,
        [
            snapshot_call(),
            move_to_new_tab_call("pane-live", "workspace-2"),
            tab_move_call("tab-created", 2),
        ]
    );
}

#[test]
fn leaves_a_new_tab_at_the_end_when_its_source_was_last() {
    let herdr = FakeHerdr::new(focused(
        snapshot(
            json!([
                layout("tab-a", vec![pane("pane-a", 0, 0, 100, 100)]),
                layout(
                    "tab-source",
                    vec![
                        pane("pane-moving", 0, 0, 50, 100),
                        pane("pane-stays", 50, 0, 50, 100),
                    ],
                ),
            ]),
            json!([
                tab("workspace-1", "tab-a", 1),
                tab("workspace-1", "tab-source", 2),
            ]),
            json!([workspace("workspace-1", "tab-source", 1)]),
        ),
        "workspace-1",
        "tab-source",
        "pane-moving",
    ))
    .with_replies("pane.move", [move_reply()]);

    let run = herdr.run("workspace-1", "tab-source", "pane-moving", &["to-new-tab"]);

    run.assert_success();
    assert_eq!(
        run.requests,
        [
            snapshot_call(),
            move_to_new_tab_call("pane-moving", "workspace-1"),
        ]
    );
}

#[test]
fn ignores_moving_the_last_pane_out_of_its_tab() {
    let herdr = FakeHerdr::new(focused(
        snapshot(
            json!([layout(
                "tab-source",
                vec![pane("pane-only", 0, 0, 100, 100)],
            )]),
            json!([tab("workspace-1", "tab-source", 1)]),
            json!([workspace("workspace-1", "tab-source", 1)]),
        ),
        "workspace-1",
        "tab-source",
        "pane-only",
    ));

    let run = herdr.run("workspace-1", "tab-source", "pane-only", &["to-new-tab"]);

    run.assert_success();
    assert_eq!(run.requests, [snapshot_call()]);
}

#[test]
fn stops_when_herdr_declines_the_new_tab_move() {
    let herdr = FakeHerdr::new(focused(
        standard_snapshot(),
        "workspace-2",
        "tab-2-1",
        "pane-2-1-top-left",
    ))
    .with_replies("pane.move", [no_op_move_reply("source_changed")]);

    let run = herdr.run(
        "workspace-2",
        "tab-2-1",
        "pane-2-1-top-left",
        &["to-new-tab"],
    );

    run.assert_success();
    assert_eq!(
        run.requests,
        [
            snapshot_call(),
            move_to_new_tab_call("pane-2-1-top-left", "workspace-2"),
        ]
    );
}

#[test]
fn reports_when_a_moved_pane_response_omits_the_created_tab_id() {
    let herdr = FakeHerdr::new(focused(
        standard_snapshot(),
        "workspace-2",
        "tab-2-1",
        "pane-2-1-top-left",
    ))
    .with_replies("pane.move", [move_reply()]);

    let run = herdr.run(
        "workspace-2",
        "tab-2-1",
        "pane-2-1-top-left",
        &["to-new-tab"],
    );

    assert_eq!(
        run.assert_failure(),
        "pane moved to a new tab, but herdr api pane.move response is missing the created tab id\n"
    );
    assert_eq!(
        run.requests,
        [
            snapshot_call(),
            move_to_new_tab_call("pane-2-1-top-left", "workspace-2"),
        ]
    );
}

#[test]
fn reports_when_the_pane_moved_but_the_new_tab_could_not_be_repositioned() {
    let herdr = FakeHerdr::new(focused(
        standard_snapshot(),
        "workspace-2",
        "tab-2-1",
        "pane-2-1-top-left",
    ))
    .with_replies("pane.move", [new_tab_reply("tab-created")])
    .with_replies(
        "tab.move",
        [Err("insert_index 1 is out of bounds".to_owned())],
    );

    let run = herdr.run(
        "workspace-2",
        "tab-2-1",
        "pane-2-1-top-left",
        &["to-new-tab"],
    );

    assert_eq!(
        run.assert_failure(),
        "pane moved to a new tab, but positioning tab-created after tab-2-1 failed: tab.move failed: insert_index 1 is out of bounds\n"
    );
    assert_eq!(
        run.requests,
        [
            snapshot_call(),
            move_to_new_tab_call("pane-2-1-top-left", "workspace-2"),
            tab_move_call("tab-created", 1),
        ]
    );
}

#[test]
fn moves_wrap_by_visible_workspace_number_and_tab_order() {
    let herdr = FakeHerdr::new(standard_snapshot());

    for (scope, direction, workspace_id, tab_id, target_tab_id) in [
        ("workspace", "next", "workspace-3", "tab-3-1", "tab-1-1"),
        ("workspace", "previous", "workspace-1", "tab-1-1", "tab-3-1"),
        ("tab", "next", "workspace-2", "tab-2-3", "tab-2-1"),
        ("tab", "previous", "workspace-2", "tab-2-1", "tab-2-3"),
    ] {
        let run = herdr.run(workspace_id, tab_id, "pane-current", &[scope, direction]);

        run.assert_success();
        let mut expected = vec![
            call("session.snapshot", json!({})),
            call("pane.move", move_to_tab("pane-current", target_tab_id)),
        ];
        if scope == "tab" {
            expected.push(snapshot_call());
        }
        assert_eq!(run.requests, expected,);
    }
}

#[test]
fn moves_follow_reordered_tab_snapshot_order() {
    let herdr = FakeHerdr::new(reordered_tab_snapshot());

    for (direction, target_tab_id) in [("next", "tab-b"), ("previous", "tab-c")] {
        let run = herdr.run("workspace-1", "tab-a", "pane-a", &["tab", direction]);

        run.assert_success();
        assert_eq!(
            run.requests,
            [
                snapshot_call(),
                call("pane.move", move_to_tab("pane-a", target_tab_id)),
                snapshot_call(),
            ]
        );
    }
}

#[test]
fn move_workspace_steps_through_the_visible_number_order() {
    // The snapshot lists the workspaces out of number order, so a slot taken from the
    // array order would land the workspace next to the wrong neighbour.
    let herdr = FakeHerdr::new(shuffled_workspace_snapshot());

    for (workspace_id, tab_id, direction, insert_index) in [
        ("workspace-2", "tab-2-1", "next", 3),
        ("workspace-2", "tab-2-1", "previous", 0),
    ] {
        let run = herdr.run(
            workspace_id,
            tab_id,
            "pane-current",
            &["move-workspace", direction],
        );

        run.assert_success();
        assert_eq!(
            run.requests,
            [
                snapshot_call(),
                workspace_move_call(workspace_id, insert_index),
            ]
        );
    }
}

#[test]
fn move_workspace_wraps_across_the_ends_with_pre_removal_slots() {
    // herdr's workspace.move takes the insertion slot counted before the workspace is
    // removed: the last one wraps to slot 0 and the first one to one past the last.
    let herdr = FakeHerdr::new(shuffled_workspace_snapshot());

    for (workspace_id, tab_id, direction, insert_index) in [
        ("workspace-3", "tab-3-1", "next", 0),
        ("workspace-1", "tab-1-1", "previous", 3),
    ] {
        let run = herdr.run(
            workspace_id,
            tab_id,
            "pane-current",
            &["move-workspace", direction],
        );

        run.assert_success();
        assert_eq!(
            run.requests,
            [
                snapshot_call(),
                workspace_move_call(workspace_id, insert_index),
            ]
        );
    }
}

#[test]
fn move_workspace_uses_live_snapshot_context_instead_of_stale_environment() {
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
        &["move-workspace", "next"],
    );

    run.assert_success();
    assert_eq!(
        run.requests,
        [snapshot_call(), workspace_move_call("workspace-2", 3)]
    );
}

#[test]
fn move_workspace_leaves_a_lone_workspace_alone() {
    let herdr = FakeHerdr::new(snapshot(
        json!([layout("tab-only", vec![pane("pane-only", 0, 0, 100, 80)])]),
        json!([tab("workspace-only", "tab-only", 1)]),
        json!([workspace("workspace-only", "tab-only", 1)]),
    ));

    let run = herdr.run(
        "workspace-only",
        "tab-only",
        "pane-only",
        &["move-workspace", "next"],
    );

    run.assert_success();
    assert_eq!(run.requests, [snapshot_call()]);
}

#[test]
fn move_workspace_reports_the_herdr_error_that_rejected_it() {
    let herdr = FakeHerdr::new(standard_snapshot()).with_replies(
        "workspace.move",
        [Err("insert_index 9 is out of bounds".to_owned())],
    );

    let run = herdr.run(
        "workspace-2",
        "tab-2-1",
        "pane-current",
        &["move-workspace", "next"],
    );

    assert_eq!(
        run.assert_failure(),
        "workspace.move failed: insert_index 9 is out of bounds\n"
    );
    assert_eq!(
        run.requests,
        [snapshot_call(), workspace_move_call("workspace-2", 3)]
    );
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
fn focus_wraps_across_reordered_tabs_in_snapshot_order() {
    let herdr = FakeHerdr::new(reordered_tab_snapshot());

    for (direction, expected) in [("left", "pane-c"), ("right", "pane-b")] {
        let run = herdr.run("workspace-1", "tab-a", "pane-a", &["focus", direction]);

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
    herdr.write_balance_state(
        "tab-2-1",
        &[
            "pane-2-1-bottom-right",
            "pane-2-1-top-left",
            "pane-2-1-top-right",
        ],
    );

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
            snapshot_call(),
        ]
    );
}

#[test]
fn tab_move_balances_the_destination_even_when_its_pane_set_is_cached() {
    let before = tab_move_before_snapshot();
    let destination = tab_move_after_snapshot("tab-destination", "pane-moving");
    let herdr = FakeHerdr::new(before.clone())
        .with_replies(
            "session.snapshot",
            [
                snapshot_reply(before),
                snapshot_reply(destination.clone()),
                snapshot_reply(destination),
            ],
        )
        .with_replies(
            "layout.export",
            [export_reply(split(
                "right",
                leaf("pane-destination"),
                leaf("pane-moving"),
            ))],
        );
    herdr.write_balance_state("tab-destination", &["pane-destination", "pane-moving"]);

    let run = herdr.run(
        "workspace-1",
        "tab-source",
        "pane-moving",
        &["tab", "previous"],
    );

    run.assert_success();
    assert_eq!(
        run.requests,
        [
            snapshot_call(),
            call("pane.move", move_to_tab("pane-moving", "tab-destination")),
            snapshot_call(),
            export_call_for("tab-destination"),
            ratio_call_for("tab-destination", &[], 0.5),
            snapshot_call(),
        ]
    );
}

#[test]
fn tab_move_skips_balance_when_herdr_does_not_move_the_pane() {
    let before = tab_move_before_snapshot();
    let herdr = FakeHerdr::new(before.clone())
        .with_replies("session.snapshot", [snapshot_reply(before)])
        .with_replies("pane.move", [no_op_move_reply("same_tab")]);
    herdr.write_balance_state("tab-destination", &["pane-destination"]);

    let run = herdr.run(
        "workspace-1",
        "tab-source",
        "pane-moving",
        &["tab", "previous"],
    );

    run.assert_success();
    assert_eq!(
        run.requests,
        [
            snapshot_call(),
            call("pane.move", move_to_tab("pane-moving", "tab-destination")),
        ]
    );
    assert_eq!(
        herdr.read_balance_state(),
        json!({"paneIds": ["pane-destination"], "tabId": "tab-destination"})
    );
}

#[test]
fn tab_move_does_not_force_the_destination_after_focus_reaches_another_tab() {
    let before = tab_move_before_snapshot();
    let user_tab = tab_move_after_snapshot("tab-user", "pane-user-a");
    let herdr = FakeHerdr::new(before.clone()).with_replies(
        "session.snapshot",
        [snapshot_reply(before), snapshot_reply(user_tab)],
    );
    herdr.write_balance_state("tab-user", &["pane-user-a", "pane-user-b"]);

    let run = herdr.run(
        "workspace-1",
        "tab-source",
        "pane-moving",
        &["tab", "previous"],
    );

    run.assert_success();
    assert_eq!(
        run.requests,
        [
            snapshot_call(),
            call("pane.move", move_to_tab("pane-moving", "tab-destination")),
            snapshot_call(),
        ]
    );
}

#[test]
fn tab_move_consumes_the_forced_destination_then_balances_the_latest_user_tab() {
    let before = tab_move_before_snapshot();
    let destination = tab_move_after_snapshot("tab-destination", "pane-moving");
    let user_tab = tab_move_after_snapshot("tab-user", "pane-user-a");
    let herdr = FakeHerdr::new(before.clone())
        .with_replies(
            "session.snapshot",
            [
                snapshot_reply(before),
                snapshot_reply(destination),
                snapshot_reply(user_tab.clone()),
                snapshot_reply(user_tab),
            ],
        )
        .with_replies(
            "layout.export",
            [
                export_reply(split(
                    "right",
                    leaf("pane-destination"),
                    leaf("pane-moving"),
                )),
                export_reply(split("right", leaf("pane-user-a"), leaf("pane-user-b"))),
            ],
        );
    herdr.write_balance_state("tab-destination", &["pane-destination", "pane-moving"]);

    let run = herdr.run(
        "workspace-1",
        "tab-source",
        "pane-moving",
        &["tab", "previous"],
    );

    run.assert_success();
    assert!(
        run.requests
            .contains(&ratio_call_for("tab-destination", &[], 0.5))
    );
    assert!(run.requests.contains(&ratio_call_for("tab-user", &[], 0.5)));
}

#[test]
fn tab_move_reports_partial_success_when_automatic_balance_fails() {
    let before = tab_move_before_snapshot();
    let destination = tab_move_after_snapshot("tab-destination", "pane-moving");
    let herdr = FakeHerdr::new(before.clone())
        .with_replies(
            "session.snapshot",
            [
                snapshot_reply(before),
                snapshot_reply(destination.clone()),
                snapshot_reply(destination),
            ],
        )
        .with_replies(
            "layout.export",
            [export_reply(split(
                "right",
                leaf("pane-destination"),
                leaf("pane-moving"),
            ))],
        )
        .with_replies(
            "layout.set_split_ratio",
            [Err("destination resize rejected".to_owned())],
        );
    herdr.write_balance_state("tab-destination", &["pane-destination", "pane-moving"]);

    let run = herdr.run(
        "workspace-1",
        "tab-source",
        "pane-moving",
        &["tab", "previous"],
    );

    assert_eq!(
        run.assert_failure(),
        "pane moved, but automatic balance failed: could not automatically balance \
         tab-destination: layout.set_split_ratio failed: destination resize rejected\n"
    );
    assert!(
        run.requests
            .contains(&ratio_call_for("tab-destination", &[], 0.5))
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
fn split_and_new_pane_aliases_only_split_in_the_existing_direction() {
    // The focused pane is far taller than wide, but every split in the tab runs
    // rightwards, so the new pane joins that row instead of starting a column.
    let herdr = FakeHerdr::new(tab_snapshot(vec![
        pane("pane-a", 0, 0, 20, 100),
        pane("pane-b", 20, 0, 80, 100),
    ]))
    .with_replies(
        "layout.export",
        [export_reply(split("right", leaf("pane-a"), leaf("pane-b")))],
    )
    .with_replies("pane.split", [split_reply("pane-new")]);

    for action in ["split-pane", "new-pane"] {
        let run = herdr.run("workspace-1", "tab-main", "pane-a", &[action]);

        run.assert_success();
        assert_eq!(
            run.requests,
            [
                snapshot_call(),
                export_call(),
                split_call("pane-a", "right"),
            ]
        );
    }
}

#[test]
fn split_pane_splits_a_wide_pane_sideways_when_directions_are_mixed() {
    let herdr = FakeHerdr::new(tab_snapshot(vec![
        pane("pane-a", 0, 0, 100, 40),
        pane("pane-b", 100, 0, 100, 40),
        pane("pane-c", 0, 40, 200, 40),
    ]))
    .with_replies(
        "layout.export",
        [export_reply(split(
            "down",
            split("right", leaf("pane-a"), leaf("pane-b")),
            leaf("pane-c"),
        ))],
    )
    .with_replies("pane.split", [split_reply("pane-new")]);

    let run = run_split_pane(&herdr, "pane-a");

    run.assert_success();
    assert_eq!(
        run.requests,
        [
            snapshot_call(),
            export_call(),
            split_call("pane-a", "right"),
        ]
    );
}

#[test]
fn split_pane_splits_the_lone_pane_of_a_tab_by_its_shape() {
    let herdr = FakeHerdr::new(tab_snapshot(vec![pane("pane-a", 0, 0, 100, 30)]))
        .with_replies("layout.export", [export_reply(leaf("pane-a"))])
        .with_replies("pane.split", [split_reply("pane-new")]);

    let run = run_split_pane(&herdr, "pane-a");

    run.assert_success();
    assert_eq!(
        run.requests,
        [
            snapshot_call(),
            export_call(),
            split_call("pane-a", "right"),
        ]
    );
}

#[test]
fn split_pane_splits_downward_on_the_two_to_one_boundary() {
    // Cells are about twice as tall as wide, so a pane only counts as wide once
    // its width passes twice its height. The boundary itself splits downwards.
    let herdr = FakeHerdr::new(tab_snapshot(vec![pane("pane-a", 0, 0, 80, 40)]))
        .with_replies("layout.export", [export_reply(leaf("pane-a"))])
        .with_replies("pane.split", [split_reply("pane-new")]);

    let run = run_split_pane(&herdr, "pane-a");

    run.assert_success();
    assert_eq!(
        run.requests,
        [snapshot_call(), export_call(), split_call("pane-a", "down"),]
    );
}

#[test]
fn split_pane_stops_before_splitting_a_zoomed_tab() {
    let herdr = FakeHerdr::new(tab_snapshot(vec![
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

    let run = run_split_pane(&herdr, "pane-a");

    assert_eq!(
        run.assert_failure(),
        "tab-main is zoomed; unzoom it before adding a pane\n"
    );
    assert_eq!(run.requests, [snapshot_call(), export_call()]);
}

#[test]
fn split_pane_stops_before_splitting_when_snapshot_and_layout_disagree() {
    let herdr = FakeHerdr::new(tab_snapshot(vec![
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

    let run = run_split_pane(&herdr, "pane-a");

    assert_eq!(
        run.assert_failure(),
        "tab-main changed while it was being read; try again\n"
    );
    assert_eq!(run.requests, [snapshot_call(), export_call()]);
}

#[test]
fn pane_focused_balances_destination_then_source_after_an_external_move() {
    let mut herdr = FakeHerdr::new(focused(
        two_tab_snapshot(),
        "workspace-1",
        "tab-target",
        "pane-c",
    ))
    .with_replies(
        "layout.export",
        [export_reply(split("right", leaf("pane-b"), leaf("pane-c")))],
    );

    let destination = run_on_pane_focused(&herdr, "tab-target", "pane-c");

    destination.assert_success();
    assert_eq!(
        destination.requests,
        [
            snapshot_call(),
            export_call_for("tab-target"),
            ratio_call_for("tab-target", &[], 0.5),
            snapshot_call(),
        ]
    );

    herdr.snapshot = focused(two_tab_snapshot(), "workspace-1", "tab-source", "pane-a");
    herdr.replies.insert(
        "layout.export".to_owned(),
        [export_reply(split("right", leaf("pane-a"), leaf("pane-d")))]
            .into_iter()
            .collect(),
    );

    let source = run_on_pane_focused(&herdr, "tab-source", "pane-a");

    source.assert_success();
    assert_eq!(
        source.requests,
        [
            snapshot_call(),
            export_call_for("tab-source"),
            ratio_call_for("tab-source", &[], 0.5),
            snapshot_call(),
        ]
    );
}

#[test]
fn pane_focused_balances_the_same_tab_when_its_panes_changed() {
    // opt+n splits the focused tab without leaving it, so the entry state still
    // names the tab; the changed pane set is what tells a new pane, or a closed
    // one, apart from a plain focus move inside the tab.
    for previous_pane_ids in [&["pane-a"] as &[&str], &["pane-a", "pane-b", "pane-closed"]] {
        let herdr = FakeHerdr::new(focused(
            tab_snapshot(vec![
                pane("pane-a", 0, 0, 50, 40),
                pane("pane-b", 50, 0, 50, 40),
            ]),
            "workspace-1",
            "tab-main",
            "pane-b",
        ))
        .with_replies(
            "layout.export",
            [export_reply(split("right", leaf("pane-a"), leaf("pane-b")))],
        );
        herdr.write_balance_state("tab-main", previous_pane_ids);

        let run = run_on_pane_focused(&herdr, "tab-main", "pane-b");

        run.assert_success();
        assert_eq!(
            run.requests,
            [
                snapshot_call(),
                export_call(),
                ratio_call(&[], 0.5),
                snapshot_call(),
            ]
        );
        assert_eq!(
            herdr.read_balance_state(),
            json!({"paneIds": ["pane-a", "pane-b"], "tabId": "tab-main"}),
        );
    }
}

#[test]
fn pane_focused_treats_a_tab_only_state_file_as_no_entry() {
    // State files written before the pane set was recorded name only the tab.
    // Such a file must not pass for "already balanced with these panes".
    let herdr = FakeHerdr::new(focused(
        tab_snapshot(vec![
            pane("pane-a", 0, 0, 50, 40),
            pane("pane-b", 50, 0, 50, 40),
        ]),
        "workspace-1",
        "tab-main",
        "pane-b",
    ))
    .with_replies(
        "layout.export",
        [export_reply(split("right", leaf("pane-a"), leaf("pane-b")))],
    );
    fs::create_dir_all(&herdr.state_path).unwrap();
    fs::write(
        herdr.state_path.join("balance-focus.json"),
        json!({"tabId": "tab-main"}).to_string(),
    )
    .unwrap();

    let run = run_on_pane_focused(&herdr, "tab-main", "pane-b");

    run.assert_success();
    assert_eq!(
        run.requests,
        [
            snapshot_call(),
            export_call(),
            ratio_call(&[], 0.5),
            snapshot_call(),
        ]
    );
}

#[test]
fn pane_focused_quietly_ignores_non_entry_and_non_balanceable_events() {
    let same_tab = FakeHerdr::new(focused(
        two_tab_snapshot(),
        "workspace-1",
        "tab-source",
        "pane-d",
    ));
    same_tab.write_balance_state("tab-source", &["pane-a", "pane-d"]);
    let same_tab_run = run_on_pane_focused(&same_tab, "tab-source", "pane-d");
    same_tab_run.assert_success();
    assert_eq!(same_tab_run.requests, [snapshot_call()]);

    let single = FakeHerdr::new(focused(
        tab_snapshot(vec![pane("pane-a", 0, 0, 100, 40)]),
        "workspace-1",
        "tab-main",
        "pane-a",
    ));
    let single_run = run_on_pane_focused(&single, "tab-main", "pane-a");
    single_run.assert_success();
    assert_eq!(single_run.requests, [snapshot_call(), snapshot_call()]);

    let zoomed = FakeHerdr::new(focused(
        tab_snapshot(vec![
            pane("pane-a", 0, 0, 50, 40),
            pane("pane-b", 50, 0, 50, 40),
        ]),
        "workspace-1",
        "tab-main",
        "pane-a",
    ))
    .with_replies(
        "layout.export",
        [zoomed_export_reply(split(
            "right",
            leaf("pane-a"),
            leaf("pane-b"),
        ))],
    );
    let zoomed_run = run_on_pane_focused(&zoomed, "tab-main", "pane-a");
    zoomed_run.assert_success();
    assert_eq!(
        zoomed_run.requests,
        [snapshot_call(), export_call(), snapshot_call()]
    );

    let vanished = FakeHerdr::new(focused(
        tab_snapshot(vec![pane("pane-live", 0, 0, 100, 40)]),
        "workspace-1",
        "tab-main",
        "pane-live",
    ));
    vanished.write_balance_state("tab-main", &["pane-live"]);
    let vanished_run = run_on_pane_focused(&vanished, "tab-gone", "pane-gone");
    vanished_run.assert_success();
    assert_eq!(vanished_run.requests, [snapshot_call()]);
}

#[test]
fn pane_focused_coalesces_internal_swap_rebuild_and_scratch_focus_events() {
    let entered = focused(tangled_snapshot(), "workspace-1", "tab-main", "pane-c");
    let internally_focused = focused(tangled_snapshot(), "workspace-1", "tab-main", "pane-a");
    let herdr = tangled_herdr()
        .with_snapshot(entered.clone())
        .with_replies(
            "session.snapshot",
            [
                snapshot_reply(entered.clone()),
                snapshot_reply(entered.clone()),
                snapshot_reply(entered),
                snapshot_reply(internally_focused.clone()),
                snapshot_reply(internally_focused.clone()),
                snapshot_reply(internally_focused.clone()),
                snapshot_reply(internally_focused.clone()),
                snapshot_reply(internally_focused),
                snapshot_reply(focused(
                    tangled_snapshot(),
                    "workspace-1",
                    "tab-main",
                    "pane-c",
                )),
            ],
        )
        .with_replies(
            "layout.export",
            [
                export_reply(split(
                    "right",
                    leaf("pane-a"),
                    split(
                        "down",
                        leaf("pane-b"),
                        split("down", leaf("pane-c"), leaf("pane-d")),
                    ),
                )),
                export_reply(grid(&[&["pane-a", "pane-b"], &["pane-c", "pane-d"]])),
            ],
        )
        .with_replies(
            "pane.move",
            [
                new_tab_reply_with_source(
                    "tab-scratch",
                    focused_layout(
                        "tab-main",
                        "pane-c",
                        vec![
                            pane("pane-a", 0, 0, 100, 100),
                            pane("pane-c", 100, 0, 100, 50),
                            pane("pane-d", 100, 50, 100, 50),
                        ],
                    ),
                ),
                successful_move_reply(
                    ("pane-c", "workspace-1", "tab-main"),
                    ("pane-c", "workspace-1", "tab-scratch"),
                    (
                        Some(focused_layout(
                            "tab-main",
                            "pane-a",
                            vec![
                                pane("pane-a", 0, 0, 100, 100),
                                pane("pane-d", 100, 0, 100, 100),
                            ],
                        )),
                        focused_layout(
                            "tab-scratch",
                            "pane-b",
                            vec![
                                pane("pane-b", 0, 0, 100, 100),
                                pane("pane-c", 100, 0, 100, 100),
                            ],
                        ),
                    ),
                ),
                successful_move_reply(
                    ("pane-d", "workspace-1", "tab-main"),
                    ("pane-d", "workspace-1", "tab-scratch"),
                    (
                        Some(focused_layout(
                            "tab-main",
                            "pane-a",
                            vec![pane("pane-a", 0, 0, 100, 100)],
                        )),
                        focused_layout(
                            "tab-scratch",
                            "pane-b",
                            vec![
                                pane("pane-b", 0, 0, 100, 100),
                                pane("pane-c", 100, 0, 100, 100),
                                pane("pane-d", 200, 0, 100, 100),
                            ],
                        ),
                    ),
                ),
                move_reply(),
            ],
        );

    let first = run_on_pane_focused(&herdr, "tab-main", "pane-a");
    first.assert_success();
    assert!(first.requests.iter().any(|call| call.method == "pane.move"));
    assert!(first.requests.contains(&focus_call("pane-c")));
    assert_eq!(first.requests.last(), Some(&snapshot_call()));

    for (tab_id, pane_id) in [
        ("tab-main", "pane-b"),
        ("tab-scratch", "pane-c"),
        ("tab-main", "pane-a"),
    ] {
        let internal = run_on_pane_focused(&herdr, tab_id, pane_id);
        internal.assert_success();
        assert_eq!(internal.requests, [snapshot_call()]);
    }

    let before_swap = focused(
        tab_snapshot(vec![
            pane("pane-a", 0, 0, 50, 40),
            pane("pane-b", 50, 0, 50, 40),
        ]),
        "workspace-1",
        "tab-main",
        "pane-a",
    );
    let internal_swap_focus = focused(
        tab_snapshot(vec![
            pane("pane-a", 0, 0, 50, 40),
            pane("pane-b", 50, 0, 50, 40),
        ]),
        "workspace-1",
        "tab-main",
        "pane-b",
    );
    let swap_herdr = FakeHerdr::new(before_swap.clone())
        .with_replies(
            "session.snapshot",
            [
                snapshot_reply(before_swap.clone()),
                snapshot_reply(before_swap.clone()),
                snapshot_reply(internal_swap_focus),
                snapshot_reply(before_swap),
            ],
        )
        .with_replies(
            "layout.export",
            [export_reply(split("right", leaf("pane-b"), leaf("pane-a")))],
        )
        .with_replies("pane.swap", [swap_reply()]);
    swap_herdr.write_balance_state("tab-before", &[]);

    let swapped = run_on_pane_focused(&swap_herdr, "tab-main", "pane-a");

    swapped.assert_success();
    assert!(swapped.requests.contains(&swap_call("pane-b", "pane-a")));
    assert!(swapped.requests.contains(&focus_call("pane-a")));
    let internal = run_on_pane_focused(&swap_herdr, "tab-main", "pane-b");
    internal.assert_success();
    assert_eq!(internal.requests, [snapshot_call()]);

    let main = focused(tangled_snapshot(), "workspace-1", "tab-main", "pane-a");
    let other = focused(
        snapshot(
            json!([
                layout(
                    "tab-main",
                    vec![
                        pane("pane-a", 0, 0, 100, 100),
                        pane("pane-c", 100, 0, 100, 50),
                        pane("pane-d", 100, 50, 100, 50),
                    ],
                ),
                layout("tab-other", vec![pane("pane-other", 0, 0, 100, 100)]),
            ]),
            json!([
                tab("workspace-1", "tab-main", 1),
                tab("workspace-1", "tab-other", 2),
            ]),
            json!([workspace("workspace-1", "tab-other", 1)]),
        ),
        "workspace-1",
        "tab-other",
        "pane-other",
    );
    let interrupted = tangled_herdr()
        .with_snapshot(main.clone())
        .with_replies(
            "session.snapshot",
            [
                snapshot_reply(main.clone()),
                snapshot_reply(main),
                snapshot_reply(other.clone()),
                snapshot_reply(other.clone()),
                snapshot_reply(other),
            ],
        )
        .with_replies(
            "pane.move",
            [
                new_tab_reply_with_source(
                    "tab-scratch",
                    focused_layout(
                        "tab-main",
                        "pane-a",
                        vec![
                            pane("pane-a", 0, 0, 100, 100),
                            pane("pane-c", 100, 0, 100, 50),
                            pane("pane-d", 100, 50, 100, 50),
                        ],
                    ),
                ),
                Err("recovery move failed".to_owned()),
            ],
        );

    let interrupted_run = run_on_pane_focused(&interrupted, "tab-main", "pane-a");

    assert_eq!(
        interrupted_run.assert_failure(),
        "could not automatically balance tab-main: could not rebuild tab-main: focus left \
         tab-main while it was being balanced; pane-b left in tab-scratch\n"
    );
    assert!(
        interrupted_run
            .requests
            .contains(&move_call("pane-b", attach_destination("pane-a", "right")))
    );
}

#[test]
fn pane_focused_latest_user_tab_wins_during_success_and_failure() {
    for (first_ratio, should_fail) in [
        (Ok(json!({"type": "layout_split_ratio_set"})), false),
        (Err("first tab changed".to_owned()), true),
    ] {
        let first = focused(two_tab_snapshot(), "workspace-1", "tab-source", "pane-a");
        let latest = focused(two_tab_snapshot(), "workspace-1", "tab-target", "pane-c");
        let mut herdr = FakeHerdr::new(first.clone())
            .with_replies(
                "session.snapshot",
                [
                    snapshot_reply(first),
                    snapshot_reply(latest.clone()),
                    snapshot_reply(latest),
                ],
            )
            .with_replies(
                "layout.export",
                [
                    export_reply(split("right", leaf("pane-a"), leaf("pane-d"))),
                    export_reply(split("right", leaf("pane-b"), leaf("pane-c"))),
                ],
            )
            .with_replies(
                "layout.set_split_ratio",
                [first_ratio, Ok(json!({"type": "layout_split_ratio_set"}))],
            );
        herdr.write_balance_state("tab-before", &[]);

        let run = run_on_pane_focused(&herdr, "tab-source", "pane-a");

        if should_fail {
            assert!(run.assert_failure().contains("first tab changed"));
        } else {
            run.assert_success();
        }
        assert!(
            run.requests
                .contains(&ratio_call_for("tab-target", &[], 0.5))
        );
        assert!(!run.requests.iter().any(|call| call.method == "pane.focus"));

        if should_fail {
            herdr.snapshot = focused(two_tab_snapshot(), "workspace-1", "tab-source", "pane-a");
            herdr.replies.remove("session.snapshot");
            herdr.replies.insert(
                "layout.export".to_owned(),
                [export_reply(split("right", leaf("pane-a"), leaf("pane-d")))]
                    .into_iter()
                    .collect(),
            );
            herdr.replies.insert(
                "layout.set_split_ratio".to_owned(),
                [Ok(json!({"type": "layout_split_ratio_set"}))]
                    .into_iter()
                    .collect(),
            );

            let retry = run_on_pane_focused(&herdr, "tab-source", "pane-a");

            retry.assert_success();
            assert!(
                retry
                    .requests
                    .contains(&ratio_call_for("tab-source", &[], 0.5))
            );
        }
    }
}

#[test]
fn balance_accepts_rows_however_their_splits_are_nested() {
    // Both rows hold three panes but nest them the other way around, which is the
    // same grid, so the panes are only resized.
    let herdr = FakeHerdr::new(tab_snapshot(vec![
        pane("pane-a", 0, 0, 100, 50),
        pane("pane-b", 100, 0, 100, 50),
        pane("pane-c", 200, 0, 100, 50),
        pane("pane-d", 0, 50, 100, 50),
        pane("pane-e", 100, 50, 100, 50),
        pane("pane-f", 200, 50, 100, 50),
    ]))
    .with_replies(
        "layout.export",
        [export_reply(split(
            "down",
            split(
                "right",
                leaf("pane-a"),
                split("right", leaf("pane-b"), leaf("pane-c")),
            ),
            split(
                "right",
                split("right", leaf("pane-d"), leaf("pane-e")),
                leaf("pane-f"),
            ),
        ))],
    );

    let run = run_balance(&herdr, "pane-a");

    run.assert_success();
    assert_eq!(
        run.requests,
        [
            snapshot_call(),
            export_call(),
            ratio_call(&[], 0.5),
            ratio_call(&[false], 1.0 / 3.0),
            ratio_call(&[false, true], 0.5),
            ratio_call(&[true], 2.0 / 3.0),
            ratio_call(&[true, false], 0.5),
        ]
    );
}

#[test]
fn balance_swaps_only_the_panes_that_sit_out_of_reading_order() {
    // The grid already has the right shape, but the tree holds pane-c where reading
    // order wants pane-b, so one swap is enough and the focus is put back.
    let herdr = FakeHerdr::new(tab_snapshot(vec![
        pane("pane-a", 0, 0, 100, 50),
        pane("pane-b", 100, 0, 100, 50),
        pane("pane-c", 0, 50, 100, 50),
        pane("pane-d", 100, 50, 100, 50),
    ]))
    .with_replies(
        "layout.export",
        [export_reply(split(
            "down",
            split("right", leaf("pane-a"), leaf("pane-c")),
            split("right", leaf("pane-b"), leaf("pane-d")),
        ))],
    )
    .with_replies("pane.swap", [swap_reply()]);

    let run = run_balance(&herdr, "pane-a");

    run.assert_success();
    assert_eq!(
        run.requests,
        [
            snapshot_call(),
            export_call(),
            swap_call("pane-c", "pane-b"),
            ratio_call(&[], 0.5),
            ratio_call(&[false], 0.5),
            ratio_call(&[true], 0.5),
            focus_call("pane-a"),
        ]
    );
}

#[test]
fn balance_untangles_a_three_way_rotation_without_repeating_a_swap() {
    // The row holds [b, c, a] but should read [a, b, c]. Rotating three panes takes
    // two swaps, and keeping the local copy in step stops a settled pane from being
    // swapped away again.
    let herdr = FakeHerdr::new(tab_snapshot(vec![
        pane("pane-a", 0, 0, 100, 50),
        pane("pane-b", 100, 0, 100, 50),
        pane("pane-c", 200, 0, 100, 50),
    ]))
    .with_replies(
        "layout.export",
        [export_reply(split(
            "right",
            split("right", leaf("pane-b"), leaf("pane-c")),
            leaf("pane-a"),
        ))],
    )
    .with_replies("pane.swap", [swap_reply()]);

    let run = run_balance(&herdr, "pane-a");

    run.assert_success();
    assert_eq!(
        run.requests,
        [
            snapshot_call(),
            export_call(),
            swap_call("pane-b", "pane-a"),
            swap_call("pane-c", "pane-b"),
            ratio_call(&[], 2.0 / 3.0),
            ratio_call(&[false], 0.5),
            focus_call("pane-a"),
        ]
    );
}

#[test]
fn balance_puts_the_focus_back_when_a_later_swap_fails() {
    let herdr = FakeHerdr::new(tab_snapshot(vec![
        pane("pane-a", 0, 0, 100, 50),
        pane("pane-b", 100, 0, 100, 50),
        pane("pane-c", 200, 0, 100, 50),
    ]))
    .with_replies(
        "layout.export",
        [export_reply(split(
            "right",
            split("right", leaf("pane-b"), leaf("pane-c")),
            leaf("pane-a"),
        ))],
    )
    .with_replies("pane.swap", [swap_reply(), Err("pane is gone".to_owned())]);

    let run = run_balance(&herdr, "pane-a");

    assert_eq!(
        run.assert_failure(),
        "swapped some panes but could not finish balancing: pane.swap failed: pane is gone\n"
    );
    assert_eq!(
        run.requests,
        [
            snapshot_call(),
            export_call(),
            swap_call("pane-b", "pane-a"),
            swap_call("pane-c", "pane-b"),
            focus_call("pane-a"),
        ]
    );
}

#[test]
fn balance_puts_the_focus_back_when_resizing_fails_after_a_swap() {
    let herdr = FakeHerdr::new(tab_snapshot(vec![
        pane("pane-a", 0, 0, 100, 50),
        pane("pane-b", 100, 0, 100, 50),
        pane("pane-c", 0, 50, 100, 50),
        pane("pane-d", 100, 50, 100, 50),
    ]))
    .with_replies(
        "layout.export",
        [export_reply(split(
            "down",
            split("right", leaf("pane-a"), leaf("pane-c")),
            split("right", leaf("pane-b"), leaf("pane-d")),
        ))],
    )
    .with_replies("pane.swap", [swap_reply()])
    .with_replies(
        "layout.set_split_ratio",
        [Err("path is out of date".to_owned())],
    );

    let run = run_balance(&herdr, "pane-a");

    assert_eq!(
        run.assert_failure(),
        "swapped some panes but could not finish balancing: \
         layout.set_split_ratio failed: path is out of date\n"
    );
    assert_eq!(
        run.requests,
        [
            snapshot_call(),
            export_call(),
            swap_call("pane-c", "pane-b"),
            ratio_call(&[], 0.5),
            focus_call("pane-a"),
        ]
    );
}

#[test]
fn balance_keeps_the_error_alone_when_the_first_swap_fails() {
    // Nothing moved yet, so there is no focus to put back.
    let herdr = FakeHerdr::new(tab_snapshot(vec![
        pane("pane-a", 0, 0, 100, 50),
        pane("pane-b", 100, 0, 100, 50),
        pane("pane-c", 0, 50, 100, 50),
        pane("pane-d", 100, 50, 100, 50),
    ]))
    .with_replies(
        "layout.export",
        [export_reply(split(
            "down",
            split("right", leaf("pane-a"), leaf("pane-c")),
            split("right", leaf("pane-b"), leaf("pane-d")),
        ))],
    )
    .with_replies("pane.swap", [Err("pane is gone".to_owned())]);

    let run = run_balance(&herdr, "pane-a");

    assert_eq!(run.assert_failure(), "pane.swap failed: pane is gone\n");
    assert_eq!(
        run.requests,
        [
            snapshot_call(),
            export_call(),
            swap_call("pane-c", "pane-b")
        ]
    );
}

#[test]
fn balance_stops_before_touching_a_tab_with_no_area() {
    let herdr = FakeHerdr::new(tab_snapshot(vec![
        pane("pane-a", 0, 0, 0, 0),
        pane("pane-b", 0, 0, 0, 0),
    ]))
    .with_replies(
        "layout.export",
        [export_reply(split("right", leaf("pane-a"), leaf("pane-b")))],
    );

    let run = run_balance(&herdr, "pane-a");

    assert_eq!(
        run.assert_failure(),
        "herdr reported a tab with no area; cannot balance\n"
    );
    assert_eq!(run.requests, [snapshot_call(), export_call()]);
}

#[test]
fn balance_leaves_a_tab_with_one_pane_alone() {
    let herdr = FakeHerdr::new(tab_snapshot(vec![pane("pane-a", 0, 0, 200, 100)]))
        .with_replies("layout.export", [export_reply(leaf("pane-a"))]);

    let run = run_balance(&herdr, "pane-a");

    run.assert_success();
    assert_eq!(run.requests, [snapshot_call(), export_call()]);
}

#[test]
fn balance_stops_before_touching_a_zoomed_tab() {
    let herdr = FakeHerdr::new(tab_snapshot(vec![
        pane("pane-a", 0, 0, 100, 50),
        pane("pane-b", 100, 0, 100, 50),
    ]))
    .with_replies(
        "layout.export",
        [zoomed_export_reply(split(
            "right",
            leaf("pane-a"),
            leaf("pane-b"),
        ))],
    );

    let run = run_balance(&herdr, "pane-a");

    assert_eq!(
        run.assert_failure(),
        "tab-main is zoomed; unzoom it before balancing\n"
    );
    assert_eq!(run.requests, [snapshot_call(), export_call()]);
}

#[test]
fn balance_settles_after_one_pass() {
    let panes = vec![
        pane("pane-a", 0, 0, 100, 50),
        pane("pane-b", 100, 0, 100, 50),
        pane("pane-c", 0, 50, 100, 50),
        pane("pane-d", 100, 50, 100, 50),
    ];
    let tangled = FakeHerdr::new(tab_snapshot(panes.clone()))
        .with_replies(
            "layout.export",
            [export_reply(split(
                "down",
                split("right", leaf("pane-a"), leaf("pane-c")),
                split("right", leaf("pane-b"), leaf("pane-d")),
            ))],
        )
        .with_replies("pane.swap", [swap_reply()]);

    let first = run_balance(&tangled, "pane-a");

    first.assert_success();
    assert!(first.requests.contains(&swap_call("pane-c", "pane-b")));

    // The swap above trades the two pane ids, and setting ratios leaves the shape
    // alone, so this is the layout the first pass left behind.
    let settled = FakeHerdr::new(tab_snapshot(panes)).with_replies(
        "layout.export",
        [export_reply(split(
            "down",
            split("right", leaf("pane-a"), leaf("pane-b")),
            split("right", leaf("pane-c"), leaf("pane-d")),
        ))],
    );

    let second = run_balance(&settled, "pane-a");

    second.assert_success();
    assert_eq!(
        second.requests,
        [
            snapshot_call(),
            export_call(),
            ratio_call(&[], 0.5),
            ratio_call(&[false], 0.5),
            ratio_call(&[true], 0.5),
        ]
    );
}

#[test]
fn balance_rebuilds_a_tangled_tab_through_a_scratch_tab() {
    // The right half is a column of three, so the tab is not a grid of flat rows and
    // has to be taken apart. herdr will not move a pane inside its own tab, so every
    // pane but the anchor goes out to one scratch tab and comes back in the order
    // that draws the target grid. No pane is created and no layout is applied: the
    // request list below is the whole story.
    let herdr = FakeHerdr::new(tab_snapshot(vec![
        pane("pane-a", 0, 0, 100, 100),
        pane("pane-b", 100, 0, 100, 33),
        pane("pane-c", 100, 33, 100, 33),
        pane("pane-d", 100, 66, 100, 34),
    ]))
    .with_replies(
        "layout.export",
        [
            export_reply(split(
                "right",
                leaf("pane-a"),
                split(
                    "down",
                    leaf("pane-b"),
                    split("down", leaf("pane-c"), leaf("pane-d")),
                ),
            )),
            export_reply(split(
                "down",
                split("right", leaf("pane-a"), leaf("pane-b")),
                split("right", leaf("pane-c"), leaf("pane-d")),
            )),
        ],
    )
    .with_replies("pane.move", [new_tab_reply("tab-scratch"), move_reply()]);

    let run = run_balance(&herdr, "pane-a");

    run.assert_success();
    assert_eq!(
        run.requests,
        [
            snapshot_call(),
            export_call(),
            move_call("pane-b", new_tab_destination()),
            move_call("pane-c", scratch_destination()),
            move_call("pane-d", scratch_destination()),
            move_call("pane-c", attach_destination("pane-a", "down")),
            move_call("pane-b", attach_destination("pane-a", "right")),
            move_call("pane-d", attach_destination("pane-c", "right")),
            export_call(),
            ratio_call(&[], 0.5),
            ratio_call(&[false], 0.5),
            ratio_call(&[true], 0.5),
            focus_call("pane-a"),
        ]
    );
}

#[test]
fn balance_halves_the_rows_it_rebuilds_so_no_split_runs_lopsided() {
    // Five panes make rows of 2, 2 and 1. Joining three rows by halving attaches the
    // last row to the anchor first and the middle row underneath it, which keeps
    // every ratio well inside the range herdr accepts.
    let herdr = FakeHerdr::new(tab_snapshot(vec![
        pane("pane-a", 0, 0, 80, 33),
        pane("pane-b", 80, 0, 80, 33),
        pane("pane-c", 0, 33, 80, 33),
        pane("pane-d", 80, 33, 80, 33),
        pane("pane-e", 0, 66, 160, 34),
    ]))
    .with_replies(
        "layout.export",
        [
            export_reply(split(
                "right",
                leaf("pane-a"),
                split(
                    "down",
                    leaf("pane-b"),
                    split(
                        "down",
                        leaf("pane-c"),
                        split("down", leaf("pane-d"), leaf("pane-e")),
                    ),
                ),
            )),
            export_reply(split(
                "down",
                split(
                    "down",
                    split("right", leaf("pane-a"), leaf("pane-b")),
                    split("right", leaf("pane-c"), leaf("pane-d")),
                ),
                leaf("pane-e"),
            )),
        ],
    )
    .with_replies("pane.move", [new_tab_reply("tab-scratch"), move_reply()]);

    let run = run_balance(&herdr, "pane-a");

    run.assert_success();
    assert_eq!(
        run.requests,
        [
            snapshot_call(),
            export_call(),
            move_call("pane-b", new_tab_destination()),
            move_call("pane-c", scratch_destination()),
            move_call("pane-d", scratch_destination()),
            move_call("pane-e", scratch_destination()),
            move_call("pane-e", attach_destination("pane-a", "down")),
            move_call("pane-c", attach_destination("pane-a", "down")),
            move_call("pane-b", attach_destination("pane-a", "right")),
            move_call("pane-d", attach_destination("pane-c", "right")),
            export_call(),
            ratio_call(&[], 2.0 / 3.0),
            ratio_call(&[false], 0.5),
            ratio_call(&[false, false], 0.5),
            ratio_call(&[false, true], 0.5),
            focus_call("pane-a"),
        ]
    );
}

#[test]
fn balance_stops_when_herdr_reports_a_move_it_did_not_make() {
    // herdr answers a refused move with changed: false rather than an error, which
    // must not read as success.
    let herdr = tangled_herdr().with_replies(
        "pane.move",
        [
            new_tab_reply("tab-scratch"),
            no_op_move_reply("same_tab"),
            move_reply(),
        ],
    );

    let run = run_balance(&herdr, "pane-a");

    assert_eq!(
        run.assert_failure(),
        "could not rebuild tab-main: herdr refused to move pane-c: same_tab\n"
    );
    assert_eq!(
        run.requests,
        [
            snapshot_call(),
            export_call(),
            move_call("pane-b", new_tab_destination()),
            move_call("pane-c", scratch_destination()),
            move_call("pane-b", attach_destination("pane-a", "right")),
            focus_call("pane-a"),
        ]
    );
}

#[test]
fn balance_puts_staged_panes_back_when_a_move_fails() {
    let herdr = tangled_herdr().with_replies(
        "pane.move",
        [
            new_tab_reply("tab-scratch"),
            move_reply(),
            Err("pane is gone".to_owned()),
            move_reply(),
        ],
    );

    let run = run_balance(&herdr, "pane-a");

    assert_eq!(
        run.assert_failure(),
        "could not rebuild tab-main: pane.move failed: pane is gone\n"
    );
    assert_eq!(
        run.requests,
        [
            snapshot_call(),
            export_call(),
            move_call("pane-b", new_tab_destination()),
            move_call("pane-c", scratch_destination()),
            move_call("pane-d", scratch_destination()),
            move_call("pane-b", attach_destination("pane-a", "right")),
            move_call("pane-c", attach_destination("pane-a", "right")),
            focus_call("pane-a"),
        ]
    );
}

#[test]
fn balance_brings_back_only_what_is_still_in_the_scratch_tab() {
    // The second reattachment fails once pane-c is already home, so only pane-b and
    // pane-d are still parked and only they are fetched back.
    let herdr = tangled_herdr().with_replies(
        "pane.move",
        [
            new_tab_reply("tab-scratch"),
            move_reply(),
            move_reply(),
            move_reply(),
            Err("pane is gone".to_owned()),
            move_reply(),
        ],
    );

    let run = run_balance(&herdr, "pane-a");

    assert_eq!(
        run.assert_failure(),
        "could not rebuild tab-main: pane.move failed: pane is gone\n"
    );
    assert_eq!(
        run.requests,
        [
            snapshot_call(),
            export_call(),
            move_call("pane-b", new_tab_destination()),
            move_call("pane-c", scratch_destination()),
            move_call("pane-d", scratch_destination()),
            move_call("pane-c", attach_destination("pane-a", "down")),
            // The reattachment that fails, and then the same pane on its way home.
            move_call("pane-b", attach_destination("pane-a", "right")),
            move_call("pane-b", attach_destination("pane-a", "right")),
            move_call("pane-d", attach_destination("pane-a", "right")),
            focus_call("pane-a"),
        ]
    );
}

#[test]
fn balance_puts_the_focus_back_on_the_pane_it_started_from() {
    // The panes are all home by the time the resizing fails, but they moved through
    // another tab to get there, so the focus still has to be put back -- and on the
    // pane the user was in, which is not the anchor the rebuild hung everything off.
    let herdr = tangled_herdr()
        .with_replies(
            "layout.export",
            [
                export_reply(split(
                    "right",
                    leaf("pane-a"),
                    split(
                        "down",
                        leaf("pane-b"),
                        split("down", leaf("pane-c"), leaf("pane-d")),
                    ),
                )),
                export_reply(split(
                    "down",
                    split("right", leaf("pane-a"), leaf("pane-b")),
                    split("right", leaf("pane-c"), leaf("pane-d")),
                )),
            ],
        )
        .with_replies("pane.move", [new_tab_reply("tab-scratch"), move_reply()])
        .with_replies(
            "layout.set_split_ratio",
            [Err("path is out of date".to_owned())],
        );

    let run = run_balance(&herdr, "pane-c");

    assert_eq!(
        run.assert_failure(),
        "rebuilt tab-main but could not even out the sizes: \
         layout.set_split_ratio failed: path is out of date\n"
    );
    assert_eq!(run.requests.last(), Some(&focus_call("pane-c")));
}

#[test]
fn balance_names_the_panes_it_could_not_bring_back() {
    // Losing a terminal is worse than a crooked layout, so a failed recovery says
    // exactly which panes are still sitting in which tab.
    let herdr = tangled_herdr().with_replies(
        "pane.move",
        [
            new_tab_reply("tab-scratch"),
            move_reply(),
            Err("pane is gone".to_owned()),
        ],
    );

    let run = run_balance(&herdr, "pane-a");

    assert_eq!(
        run.assert_failure(),
        "could not rebuild tab-main: pane.move failed: pane is gone; \
         pane-b, pane-c left in tab-scratch\n"
    );
}

#[test]
fn balance_stops_when_the_rebuilt_tab_is_not_the_grid() {
    // A pane went missing while the tab was being rebuilt, so the paths worked out
    // from this tree would point at the wrong panes.
    let herdr = tangled_herdr()
        .with_replies(
            "layout.export",
            [
                export_reply(split(
                    "right",
                    leaf("pane-a"),
                    split(
                        "down",
                        leaf("pane-b"),
                        split("down", leaf("pane-c"), leaf("pane-d")),
                    ),
                )),
                export_reply(split(
                    "down",
                    split("right", leaf("pane-a"), leaf("pane-b")),
                    leaf("pane-c"),
                )),
            ],
        )
        .with_replies("pane.move", [new_tab_reply("tab-scratch"), move_reply()]);

    let run = run_balance(&herdr, "pane-a");

    assert_eq!(
        run.assert_failure(),
        "tab-main did not come out as the expected grid; left the sizes alone\n"
    );
    assert!(
        !run.requests
            .iter()
            .any(|call| call.method == "layout.set_split_ratio")
    );
    assert_eq!(run.requests.last(), Some(&focus_call("pane-a")));
}

#[test]
fn every_manifest_entrypoint_is_a_command_the_plugin_accepts() {
    // The manifest is what herdr actually runs, so a typo there only shows up when an
    // action or event fires. Running the real binary on each binding catches it here.
    let manifest = fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/herdr-plugin.toml"))
        .expect("the manifest sits next to Cargo.toml");
    let commands = manifest_commands(&manifest);
    let events: Vec<_> = manifest
        .lines()
        .filter_map(|line| line.strip_prefix("on = "))
        .collect();

    assert_eq!(events, ["\"pane.focused\""]);

    assert!(
        !commands.is_empty(),
        "the manifest binds no action to the plugin"
    );
    assert_eq!(
        commands.len(),
        manifest.matches("[[actions]]").count() + manifest.matches("[[events]]").count(),
        "a manifest entrypoint whose command does not run the plugin binary would slip past this test"
    );
    for arguments in commands {
        let output = ProcessCommand::new(env!("CARGO_BIN_EXE_herdr-move-pane"))
            .args(&arguments)
            .env_clear()
            .output()
            .unwrap();

        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            !stderr.starts_with("usage:"),
            "the manifest binds {arguments:?}, which the plugin rejects: {stderr}"
        );
    }
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

    fn with_snapshot(mut self, snapshot: Value) -> Self {
        self.snapshot = snapshot;
        self
    }

    fn run(&self, workspace_id: &str, tab_id: &str, pane_id: &str, args: &[&str]) -> Run {
        let socket_path = self.directory.join("herdr.sock");
        let stop = Arc::new(AtomicBool::new(false));
        let server = self.serve(&socket_path, Arc::clone(&stop));
        let output = ProcessCommand::new(env!("CARGO_BIN_EXE_herdr-move-pane"))
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

    fn write_balance_state(&self, tab_id: &str, pane_ids: &[&str]) {
        fs::create_dir_all(&self.state_path).unwrap();
        fs::write(
            self.state_path.join("balance-focus.json"),
            json!({"paneIds": pane_ids, "tabId": tab_id}).to_string(),
        )
        .unwrap();
    }

    fn read_balance_state(&self) -> Value {
        let state = fs::read_to_string(self.state_path.join("balance-focus.json")).unwrap();
        serde_json::from_str(&state).unwrap()
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
/// `changed`, `reason` and `created_tab` live, while `layout.set_split_ratio` echoes
/// a layout and `workspace.move` the reordered workspace list, neither of which the
/// plugin reads, so only their acknowledgements are modelled. Any other method must
/// be scripted with [`FakeHerdr::with_replies`]; an unscripted one fails loudly
/// instead of succeeding on an invented response.
fn default_reply(call: &Call, snapshot: &Value) -> Reply {
    Ok(match call.method.as_str() {
        "session.snapshot" => json!({"type": "session_snapshot", "snapshot": snapshot}),
        "pane.focus" => json!({"pane": {"pane_id": call.params["pane_id"]}}),
        "pane.move" => default_move_reply(call, snapshot),
        "layout.set_split_ratio" => json!({"type": "layout_split_ratio_set"}),
        "workspace.move" => json!({
            "type": "workspace_list",
            "workspaces": snapshot["workspaces"].clone(),
        }),
        method => panic!("unscripted socket method: {method}"),
    })
}

fn default_move_reply(call: &Call, snapshot: &Value) -> Value {
    let pane_id = call.params["pane_id"].as_str().unwrap();
    let tab_id = call.params["destination"]["tab_id"]
        .as_str()
        .unwrap_or("tab-moved");
    let workspace_id = snapshot["tabs"]
        .as_array()
        .and_then(|tabs| tabs.iter().find(|tab| tab["tab_id"] == tab_id))
        .and_then(|tab| tab["workspace_id"].as_str())
        .or_else(|| call.params["destination"]["workspace_id"].as_str())
        .unwrap_or("workspace-1");
    json!({
        "type": "pane_move",
        "move_result": {
            "changed": true,
            "previous_pane_id": pane_id,
            "previous_workspace_id": "workspace-source",
            "previous_tab_id": "tab-source",
            "pane": {
                "pane_id": pane_id,
                "workspace_id": workspace_id,
                "tab_id": tab_id,
            },
            "source_layout": null,
            "target_layout": layout(tab_id, vec![pane(pane_id, 0, 0, 100, 100)]),
            "focused_pane_id": pane_id,
        },
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

fn reordered_tab_snapshot() -> Value {
    snapshot(
        json!([
            layout("tab-a", vec![pane("pane-a", 0, 0, 100, 80)]),
            layout("tab-b", vec![pane("pane-b", 0, 0, 100, 80)]),
            layout("tab-c", vec![pane("pane-c", 0, 0, 100, 80)]),
        ]),
        json!([
            tab("workspace-1", "tab-a", 72),
            tab("workspace-1", "tab-b", 82),
            tab("workspace-1", "tab-c", 80),
        ]),
        json!([workspace("workspace-1", "tab-a", 1)]),
    )
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

/// The standard session with its workspaces listed out of number order, which the
/// plugin has to sort back into the visible order before picking a slot.
fn shuffled_workspace_snapshot() -> Value {
    snapshot(
        standard_layouts(),
        standard_tabs(),
        json!([
            workspace("workspace-3", "tab-3-1", 3),
            workspace("workspace-1", "tab-1-1", 1),
            workspace("workspace-2", "tab-2-2", 2),
        ]),
    )
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

/// Every `split-pane` scenario runs the same command against the same tab, so only
/// the tree and the focused pane change between them.
fn run_split_pane(herdr: &FakeHerdr, pane_id: &str) -> Run {
    herdr.run("workspace-1", "tab-main", pane_id, &["split-pane"])
}

fn run_on_pane_focused(herdr: &FakeHerdr, tab_id: &str, pane_id: &str) -> Run {
    herdr.run("workspace-1", tab_id, pane_id, &["on-pane-focused"])
}

fn run_balance(herdr: &FakeHerdr, pane_id: &str) -> Run {
    herdr.run("workspace-1", "tab-main", pane_id, &["balance"])
}

/// Four panes that need a rebuild: a tall column beside the anchor, which reading
/// order wants as two rows of two. Only the `pane.move` replies differ between the
/// scenarios that exercise the rebuild going wrong.
fn tangled_herdr() -> FakeHerdr {
    FakeHerdr::new(tangled_snapshot()).with_replies(
        "layout.export",
        [export_reply(split(
            "right",
            leaf("pane-a"),
            split(
                "down",
                leaf("pane-b"),
                split("down", leaf("pane-c"), leaf("pane-d")),
            ),
        ))],
    )
}

fn tangled_snapshot() -> Value {
    tab_snapshot(vec![
        pane("pane-a", 0, 0, 100, 100),
        pane("pane-b", 100, 0, 100, 33),
        pane("pane-c", 100, 33, 100, 33),
        pane("pane-d", 100, 66, 100, 34),
    ])
}

/// A tab holding exactly the given panes, which the grid scenarios build on.
fn tab_snapshot(panes: Vec<Value>) -> Value {
    snapshot(
        json!([layout("tab-main", panes)]),
        json!([tab("workspace-1", "tab-main", 1)]),
        json!([workspace("workspace-1", "tab-main", 1)]),
    )
}

fn two_tab_snapshot() -> Value {
    snapshot(
        json!([
            layout(
                "tab-source",
                vec![pane("pane-a", 0, 0, 50, 40), pane("pane-d", 50, 0, 50, 40),],
            ),
            layout(
                "tab-target",
                vec![pane("pane-b", 0, 0, 50, 40), pane("pane-c", 50, 0, 50, 40),],
            ),
        ]),
        json!([
            tab("workspace-1", "tab-source", 1),
            tab("workspace-1", "tab-target", 2),
        ]),
        json!([workspace("workspace-1", "tab-source", 1)]),
    )
}

fn tab_move_before_snapshot() -> Value {
    focused(
        snapshot(
            json!([
                layout(
                    "tab-destination",
                    vec![pane("pane-destination", 0, 0, 100, 40)],
                ),
                layout(
                    "tab-source",
                    vec![
                        pane("pane-source", 0, 0, 50, 40),
                        pane("pane-moving", 50, 0, 50, 40),
                    ],
                ),
                layout(
                    "tab-user",
                    vec![
                        pane("pane-user-a", 0, 0, 60, 40),
                        pane("pane-user-b", 60, 0, 40, 40),
                    ],
                ),
            ]),
            tab_move_tabs(),
            json!([workspace("workspace-1", "tab-source", 1)]),
        ),
        "workspace-1",
        "tab-source",
        "pane-moving",
    )
}

fn tab_move_after_snapshot(focused_tab_id: &str, focused_pane_id: &str) -> Value {
    focused(
        snapshot(
            json!([
                layout(
                    "tab-destination",
                    vec![
                        pane("pane-destination", 0, 0, 70, 40),
                        pane("pane-moving", 70, 0, 30, 40),
                    ],
                ),
                layout("tab-source", vec![pane("pane-source", 0, 0, 100, 40)],),
                layout(
                    "tab-user",
                    vec![
                        pane("pane-user-a", 0, 0, 60, 40),
                        pane("pane-user-b", 60, 0, 40, 40),
                    ],
                ),
            ]),
            tab_move_tabs(),
            json!([workspace("workspace-1", focused_tab_id, 1)]),
        ),
        "workspace-1",
        focused_tab_id,
        focused_pane_id,
    )
}

fn tab_move_tabs() -> Value {
    json!([
        tab("workspace-1", "tab-destination", 1),
        tab("workspace-1", "tab-source", 2),
        tab("workspace-1", "tab-user", 3),
    ])
}

fn snapshot_reply(snapshot: Value) -> Reply {
    Ok(json!({"type": "session_snapshot", "snapshot": snapshot}))
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
            "tab_id": "tab-main",
            "zoomed": zoomed,
            "focused_pane_id": "pane-a",
            "root": root,
        },
    }))
}

fn split_reply(pane_id: &str) -> Reply {
    Ok(json!({"type": "pane_info", "pane": {"pane_id": pane_id}}))
}

fn swap_reply() -> Reply {
    Ok(json!({"type": "pane_swap"}))
}

fn tab_move_reply() -> Reply {
    Ok(json!({"type": "tab_list", "tabs": []}))
}

fn move_reply() -> Reply {
    successful_move_reply(
        ("pane-moved", "workspace-1", "tab-main"),
        ("pane-moved", "workspace-1", "tab-main"),
        (
            None,
            layout("tab-main", vec![pane("pane-moved", 0, 0, 100, 100)]),
        ),
    )
}

fn successful_move_reply(
    previous: (&str, &str, &str),
    moved: (&str, &str, &str),
    layouts: (Option<Value>, Value),
) -> Reply {
    let (previous_pane_id, previous_workspace_id, previous_tab_id) = previous;
    let (pane_id, workspace_id, tab_id) = moved;
    let (source_layout, target_layout) = layouts;
    Ok(json!({
        "type": "pane_move",
        "move_result": {
            "changed": true,
            "previous_pane_id": previous_pane_id,
            "previous_workspace_id": previous_workspace_id,
            "previous_tab_id": previous_tab_id,
            "pane": {
                "pane_id": pane_id,
                "workspace_id": workspace_id,
                "tab_id": tab_id,
            },
            "source_layout": source_layout,
            "target_layout": target_layout,
            "focused_pane_id": pane_id,
        },
    }))
}

fn new_tab_reply(tab_id: &str) -> Reply {
    new_tab_reply_with_source(tab_id, Value::Null)
}

fn new_workspace_reply(workspace_id: &str) -> Reply {
    Ok(json!({
        "type": "pane_move",
        "move_result": {
            "changed": true,
            "created_workspace": {"workspace_id": workspace_id},
            "focused_pane_id": "pane-moved",
            "pane": {
                "pane_id": "pane-moved",
                "tab_id": "tab-created",
                "workspace_id": workspace_id,
            },
            "previous_workspace_id": "workspace-source",
            "source_layout": null,
            "target_layout": layout(
                "tab-created",
                vec![pane("pane-moved", 0, 0, 100, 100)],
            ),
        },
    }))
}

fn new_tab_reply_with_source(tab_id: &str, source_layout: Value) -> Reply {
    Ok(json!({
        "type": "pane_move",
        "move_result": {
            "changed": true,
            "created_tab": {"tab_id": tab_id},
            "focused_pane_id": "pane-moved",
            "pane": {
                "pane_id": "pane-moved",
                "tab_id": tab_id,
                "workspace_id": "workspace-1",
            },
            "previous_workspace_id": "workspace-1",
            "source_layout": source_layout,
            "target_layout": layout(
                tab_id,
                vec![pane("pane-moved", 0, 0, 100, 100)],
            ),
        },
    }))
}

/// herdr answers a move it declined with `changed: false` and a reason instead of an
/// error, for instance when the destination turns out to be the pane's own tab.
fn no_op_move_reply(reason: &str) -> Reply {
    Ok(json!({
        "type": "pane_move",
        "move_result": {
            "changed": false,
            "focused_pane_id": "pane-moved",
            "pane": {
                "pane_id": "pane-moved",
                "tab_id": "tab-main",
                "workspace_id": "workspace-1",
            },
            "previous_workspace_id": "workspace-1",
            "reason": reason,
            "source_layout": null,
            "target_layout": layout(
                "tab-main",
                vec![pane("pane-moved", 0, 0, 100, 100)],
            ),
        },
    }))
}

/// The ratio a fake tree carries is irrelevant: balancing recomputes every ratio it
/// touches from the shape of the tree.
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

fn grid(rows: &[&[&str]]) -> Value {
    rows.iter()
        .map(|row| split_run(row))
        .reduce(|first, second| split("down", first, second))
        .unwrap()
}

fn split_run(pane_ids: &[&str]) -> Value {
    pane_ids
        .iter()
        .map(|pane_id| leaf(pane_id))
        .reduce(|first, second| split("right", first, second))
        .unwrap()
}

fn export_call() -> Call {
    export_call_for("tab-main")
}

fn export_call_for(tab_id: &str) -> Call {
    call("layout.export", json!({"tab_id": tab_id}))
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

/// A rebuild never carries the focus along; it is put back once at the end.
fn move_call(pane_id: &str, destination: Value) -> Call {
    call(
        "pane.move",
        json!({"pane_id": pane_id, "destination": destination, "focus": false}),
    )
}

fn new_tab_destination() -> Value {
    json!({"type": "new_tab", "workspace_id": "workspace-1"})
}

fn scratch_destination() -> Value {
    json!({"type": "tab", "tab_id": "tab-scratch", "split": "right", "ratio": 0.5})
}

fn attach_destination(target_pane_id: &str, split: &str) -> Value {
    json!({
        "type": "tab",
        "tab_id": "tab-main",
        "target_pane_id": target_pane_id,
        "split": split,
        "ratio": 0.5,
    })
}

fn swap_call(source_pane_id: &str, target_pane_id: &str) -> Call {
    call(
        "pane.swap",
        json!({"source_pane_id": source_pane_id, "target_pane_id": target_pane_id}),
    )
}

fn ratio_call(path: &[bool], ratio: f64) -> Call {
    ratio_call_for("tab-main", path, ratio)
}

fn ratio_call_for(tab_id: &str, path: &[bool], ratio: f64) -> Call {
    call(
        "layout.set_split_ratio",
        json!({"tab_id": tab_id, "path": path, "ratio": ratio}),
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

fn focused_layout(tab_id: &str, focused_pane_id: &str, panes: Vec<Value>) -> Value {
    json!({"focused_pane_id": focused_pane_id, "panes": panes, "tab_id": tab_id})
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

fn workspace_move_call(workspace_id: &str, insert_index: usize) -> Call {
    call(
        "workspace.move",
        json!({"workspace_id": workspace_id, "insert_index": insert_index}),
    )
}

fn tab_move_call(tab_id: &str, insert_index: usize) -> Call {
    call(
        "tab.move",
        json!({"tab_id": tab_id, "insert_index": insert_index}),
    )
}

fn move_to_new_tab_call(pane_id: &str, workspace_id: &str) -> Call {
    call(
        "pane.move",
        json!({
            "pane_id": pane_id,
            "destination": {"type": "new_tab", "workspace_id": workspace_id},
            "focus": true,
        }),
    )
}

fn move_to_new_workspace_call(pane_id: &str) -> Call {
    call(
        "pane.move",
        json!({
            "pane_id": pane_id,
            "destination": {"type": "new_workspace"},
            "focus": true,
        }),
    )
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

/// The arguments each manifest action or event passes to the plugin binary, skipping
/// the build command and anything else that does not run it.
fn manifest_commands(manifest: &str) -> Vec<Vec<String>> {
    manifest
        .lines()
        .filter_map(|line| line.strip_prefix("command = ["))
        .map(|list| {
            list.trim_end_matches(']')
                .split(',')
                .map(|item| item.trim().trim_matches('"').to_owned())
                .collect::<Vec<_>>()
        })
        .filter(|command| {
            command
                .first()
                .is_some_and(|program| program.ends_with("/herdr-move-pane"))
        })
        .map(|command| command[1..].to_vec())
        .collect()
}

fn temp_dir() -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("hmp-{}-{unique}-{sequence}", std::process::id()))
}
