use std::collections::{HashMap, VecDeque};
use std::fs::{self, File};
use std::io::{BufRead, BufReader, ErrorKind, Write};
#[cfg(unix)]
use std::os::unix::net::{UnixListener, UnixStream as LocalStream};
use std::path::{Path, PathBuf};
use std::process::{Command as ProcessCommand, Output};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[cfg(windows)]
use interprocess::{
    TryClone,
    local_socket::{
        GenericNamespaced, ListenerNonblockingMode, ListenerOptions, Stream as LocalStream,
        prelude::*,
    },
};
use serde_json::{Value, json};

/// One scripted socket reply. `Err` is serialized as herdr's `error.message`.
type Reply = Result<Value, String>;

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[cfg(windows)]
#[path = "support/windows.rs"]
mod windows;

#[test]
fn moving_a_focused_leader_tab_in_visible_workspace_order_focuses_its_new_destination_id() {
    for (direction, number, target) in [
        ("next", 2, "workspace-last"),
        ("previous", 2, "workspace-first"),
        ("next", 4, "workspace-first"),
        ("previous", 0, "workspace-last"),
    ] {
        let mut before = tab_workspace_before_snapshot(Some(target));
        before["workspaces"][0]["number"] = number.into();
        before["layouts"][0]["panes"]
            .as_array_mut()
            .unwrap()
            .truncate(1);
        let run = FakeHerdr::new(before)
            .with_replies("pane.move", [new_tab_reply("tab-created")])
            .run(
                "workspace-stale",
                "tab-stale",
                "pane-stale",
                &["move-tab-workspace", direction],
            );
        run.assert_success();
        assert_eq!(
            run.requests,
            [
                snapshot_call(),
                move_to_new_workspace_call_with(
                    "pane-live",
                    json!({"type": "new_tab", "workspace_id": target})
                ),
                focus_call("pane-moved"),
            ]
        );
    }
}

#[test]
fn ignores_moving_a_tab_when_there_is_only_one_workspace() {
    for (direction, tab_count) in [("next", 1), ("previous", 1), ("next", 2), ("previous", 2)] {
        let mut before = single_pane_tab_snapshot();
        before["workspaces"].as_array_mut().unwrap().truncate(1);
        before["tabs"].as_array_mut().unwrap().truncate(tab_count);
        before["layouts"]
            .as_array_mut()
            .unwrap()
            .truncate(tab_count);
        let run = FakeHerdr::new(before).run(
            "workspace-source",
            "tab-source",
            "pane-moving",
            &["move-tab-workspace", direction],
        );
        run.assert_success();
        assert_eq!(run.requests, [snapshot_call()]);
    }
}

#[test]
fn moving_a_focused_leader_tab_to_a_new_workspace_focuses_its_new_id_before_placement() {
    let before = tab_to_new_workspace_before_snapshot();
    let after = tab_workspace_after_snapshot(None);
    let herdr = FakeHerdr::new(before.clone())
        .with_replies(
            "session.snapshot",
            [
                snapshot_reply(before),
                snapshot_reply(after.clone()),
                snapshot_reply(after),
            ],
        )
        .with_replies(
            "pane.move",
            [new_workspace_reply("workspace-created"), move_reply()],
        )
        .with_replies(
            "layout.export",
            [export_reply(split(
                "right",
                leaf("pane-live"),
                leaf("pane-stays"),
            ))],
        );

    let run = herdr.run(
        "workspace-stale",
        "tab-stale",
        "pane-stale",
        &["tab-to-new-workspace"],
    );

    run.assert_success();
    assert_eq!(
        run.requests,
        [
            snapshot_call(),
            move_to_new_workspace_call("pane-live"),
            move_call("pane-stays", join_destination("tab-created", "pane-moved")),
            focus_call("pane-moved"),
            workspace_move_call("workspace-created", 2),
            snapshot_call(),
            export_call_for("tab-created"),
            ratio_call_for("tab-created", &[], 0.5),
            snapshot_call(),
        ]
    );
}

#[test]
fn pane_to_new_workspace_moves_only_the_focused_pane_and_keeps_the_source_tab() {
    let mut before = tab_to_new_workspace_before_snapshot();
    remove_tab(&mut before, "tab-other");
    let herdr = FakeHerdr::new(before)
        .with_replies("pane.move", [new_workspace_reply("workspace-created")]);

    let run = herdr.run(
        "workspace-source",
        "tab-source",
        "pane-live",
        &["to-new-workspace"],
    );

    run.assert_success();
    assert_eq!(
        run.requests,
        [
            snapshot_call(),
            move_to_new_workspace_call("pane-live"),
            focus_call("pane-moved"),
            workspace_move_call("workspace-created", 2),
        ]
    );
}

#[test]
fn pane_to_new_workspace_ignores_the_only_pane_in_the_workspaces_only_tab() {
    let herdr = FakeHerdr::new(tab_snapshot(vec![pane("pane-only", 0, 0, 100, 100)]));

    let run = herdr.run(
        "workspace-1",
        "tab-main",
        "pane-only",
        &["to-new-workspace"],
    );

    run.assert_success();
    assert_eq!(run.requests, [snapshot_call()]);
}

#[test]
fn pane_to_new_workspace_does_not_require_plugin_state_or_copy_the_source_tab_label() {
    let mut before = tab_to_new_workspace_before_snapshot();
    before["tabs"][0]["label"] = "build".into();
    let herdr = FakeHerdr::new(before)
        .with_replies("pane.move", [new_workspace_reply("workspace-created")]);

    let run = herdr.run_without_state(
        "workspace-source",
        "tab-source",
        "pane-live",
        &["to-new-workspace"],
    );

    run.assert_success();
    assert_eq!(
        run.requests,
        [
            snapshot_call(),
            move_to_new_workspace_call("pane-live"),
            focus_call("pane-moved"),
            workspace_move_call("workspace-created", 2),
        ]
    );
}

#[test]
fn pane_to_new_workspace_moves_a_single_pane_tab_when_another_tab_remains_at_the_end() {
    let mut before = single_pane_tab_snapshot();
    before["workspaces"][0]["number"] = 2.into();
    before["workspaces"][1]["number"] = 1.into();
    let herdr = FakeHerdr::new(before).with_replies("pane.move", [move_reply()]);

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
            focus_call("pane-moved"),
        ]
    );
}

#[test]
fn pane_to_new_workspace_stops_when_herdr_declines_the_move() {
    let herdr = FakeHerdr::new(tab_to_new_workspace_before_snapshot())
        .with_replies("pane.move", [no_op_move_reply("source_changed")]);

    let run = herdr.run(
        "workspace-source",
        "tab-source",
        "pane-live",
        &["to-new-workspace"],
    );

    run.assert_success();
    assert_eq!(
        run.requests,
        [snapshot_call(), move_to_new_workspace_call("pane-live")]
    );
}

#[test]
fn pane_to_new_workspace_reports_when_the_move_completed_but_focus_failed() {
    let herdr = FakeHerdr::new(tab_to_new_workspace_before_snapshot())
        .with_replies("pane.move", [new_workspace_reply("workspace-created")])
        .with_replies("pane.focus", [Err("pane not found".to_owned())]);

    let run = herdr.run(
        "workspace-source",
        "tab-source",
        "pane-live",
        &["to-new-workspace"],
    );

    assert_eq!(
        run.assert_failure(),
        "pane moved to a new workspace, but focusing moved pane pane-moved failed: pane.focus failed: pane not found\n"
    );
    assert_eq!(
        run.requests,
        [
            snapshot_call(),
            move_to_new_workspace_call("pane-live"),
            focus_call("pane-moved"),
        ]
    );
}

#[test]
fn pane_to_new_workspace_reports_when_the_completed_move_cannot_be_repositioned() {
    for (reply, error) in [
        (
            move_reply(),
            "pane moved to a new workspace, but herdr api pane.move response is missing the created workspace id\n",
        ),
        (
            new_workspace_reply("workspace-created"),
            "pane moved to a new workspace, but positioning workspace-created after workspace-source failed: workspace.move failed: insert index rejected\n",
        ),
    ] {
        let herdr = FakeHerdr::new(tab_to_new_workspace_before_snapshot())
            .with_replies("pane.move", [reply])
            .with_replies("workspace.move", [Err("insert index rejected".to_owned())]);

        let run = herdr.run(
            "workspace-source",
            "tab-source",
            "pane-live",
            &["to-new-workspace"],
        );

        assert_eq!(run.assert_failure(), error);
        let mut expected = vec![
            snapshot_call(),
            move_to_new_workspace_call("pane-live"),
            focus_call("pane-moved"),
        ];
        if error.contains("positioning") {
            expected.push(workspace_move_call("workspace-created", 2));
        }
        assert_eq!(run.requests, expected);
    }
}

#[test]
fn waits_for_the_balance_lock_before_moving_the_tab() {
    for (target, arguments) in tab_workspace_commands("workspace-last") {
        // A pane.focused hook from an earlier action may still hold the lock; moving
        // while it balances would let it hand focus back to the leader halfway through.
        // The hold only has to outlast the plugin's start-up, after which an unlocked
        // plugin would already have sent its first request. A freshly built binary
        // starts slowly the first time macOS validates it, so it is started once before.
        const HOLD: Duration = Duration::from_millis(300);
        let before = tab_workspace_before_snapshot(target);
        let after = tab_workspace_after_snapshot(target);
        let herdr = FakeHerdr::new(before.clone())
            .with_replies(
                "session.snapshot",
                [
                    snapshot_reply(before),
                    snapshot_reply(after.clone()),
                    snapshot_reply(after),
                ],
            )
            .with_replies("pane.move", [tab_workspace_reply(target), move_reply()])
            .with_replies(
                "layout.export",
                [export_reply(split(
                    "right",
                    leaf("pane-live"),
                    leaf("pane-stays"),
                ))],
            );
        herdr.warm_up_binary();
        let held = herdr.hold_balance_lock();

        let run = thread::scope(|scope| {
            let run =
                scope.spawn(|| herdr.run("workspace-source", "tab-source", "pane-live", arguments));
            thread::sleep(HOLD);
            drop(held);
            run.join().unwrap()
        });

        run.assert_success();
        assert_eq!(herdr.premature_requests(), 0);
        let expected = vec![
            snapshot_call(),
            move_to_new_workspace_call_with("pane-live", tab_workspace_destination(target, None)),
            move_call("pane-stays", join_destination("tab-created", "pane-moved")),
            focus_call("pane-moved"),
            workspace_move_call("workspace-created", 2),
            snapshot_call(),
            export_call_for("tab-created"),
            ratio_call_for("tab-created", &[], 0.5),
            snapshot_call(),
        ];
        assert_eq!(run.requests, tab_workspace_calls(target, expected));
    }
}

#[test]
fn restores_focus_to_the_later_pane_after_moving_a_tab_between_workspaces() {
    for (target, arguments) in [
        (None, &["tab-to-new-workspace"][..]),
        (Some("workspace-last"), &["move-tab-workspace", "next"][..]),
        (
            Some("workspace-first"),
            &["move-tab-workspace", "previous"][..],
        ),
    ] {
        let mut before = focused(
            tab_workspace_before_snapshot(target),
            "workspace-source",
            "tab-source",
            "pane-stays",
        );
        before["tabs"][0]["label"] = "build".into();
        let after = focused(
            tab_workspace_after_snapshot(target),
            target.unwrap_or("workspace-created"),
            "tab-created",
            "pane-stays",
        );
        let herdr = FakeHerdr::new(before.clone())
            .with_replies(
                "session.snapshot",
                [
                    snapshot_reply(before),
                    snapshot_reply(after.clone()),
                    snapshot_reply(after),
                ],
            )
            .with_replies("pane.move", [tab_workspace_reply(target), move_reply()])
            .with_replies(
                "layout.export",
                [export_reply(split(
                    "right",
                    leaf("pane-live"),
                    leaf("pane-stays"),
                ))],
            );

        let run = herdr.run("workspace-source", "tab-source", "pane-stays", arguments);

        run.assert_success();
        let expected = vec![
            snapshot_call(),
            move_to_new_workspace_call_with(
                "pane-live",
                tab_workspace_destination(target, Some("build")),
            ),
            move_call("pane-stays", join_destination("tab-created", "pane-moved")),
            focus_call("pane-stays"),
            workspace_move_call("workspace-created", 2),
            snapshot_call(),
            export_call_for("tab-created"),
            ratio_call_for("tab-created", &[], 0.5),
            snapshot_call(),
        ];
        assert_eq!(run.requests, tab_workspace_calls(target, expected));
    }
}

#[test]
fn reports_when_the_tab_moved_but_refocusing_failed() {
    for (target, arguments) in tab_workspace_commands("workspace-last") {
        for (focused_pane_id, moved_focus_id) in
            [("pane-live", "pane-moved"), ("pane-stays", "pane-stays")]
        {
            let before = focused(
                tab_workspace_before_snapshot(target),
                "workspace-source",
                "tab-source",
                focused_pane_id,
            );
            let herdr = FakeHerdr::new(before)
                .with_replies("pane.move", [tab_workspace_reply(target), move_reply()])
                .with_replies("pane.focus", [Err("pane not found".to_owned())]);

            let run = herdr.run("workspace-source", "tab-source", focused_pane_id, arguments);

            assert_eq!(
                run.assert_failure(),
                format!(
                    "tab moved, but refocusing {moved_focus_id} failed: pane.focus failed: pane not found\n"
                )
            );
            let expected = vec![
                snapshot_call(),
                move_to_new_workspace_call_with(
                    "pane-live",
                    tab_workspace_destination(target, None),
                ),
                move_call("pane-stays", join_destination("tab-created", "pane-moved")),
                focus_call(moved_focus_id),
            ];
            assert_eq!(run.requests, expected);
        }
    }
}

#[test]
fn leaves_a_new_workspace_at_the_end_when_its_source_was_last() {
    let herdr = FakeHerdr::new(focused(
        snapshot(
            json!([
                layout("tab-first", vec![pane("pane-first", 0, 0, 100, 100)]),
                layout("tab-source", vec![pane("pane-moving", 0, 0, 100, 100)]),
                layout("tab-stays", vec![pane("pane-stays", 0, 0, 100, 100)]),
            ]),
            json!([
                tab("workspace-first", "tab-first", 1),
                tab("workspace-source", "tab-source", 1),
                tab("workspace-source", "tab-stays", 2),
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
        &["tab-to-new-workspace"],
    );

    run.assert_success();
    assert_eq!(
        run.requests,
        [
            snapshot_call(),
            move_to_new_workspace_call("pane-moving"),
            focus_call("pane-moved"),
        ]
    );
}

#[test]
fn ignores_moving_the_only_tab_to_a_new_workspace() {
    let herdr = FakeHerdr::new(tab_workspace_before_snapshot(Some("workspace-last")));
    let run = herdr.run(
        "workspace-source",
        "tab-source",
        "pane-live",
        &["tab-to-new-workspace"],
    );
    run.assert_success();
    assert_eq!(run.requests, [snapshot_call()]);
}

#[test]
fn moving_a_focused_single_pane_tab_between_workspaces_focuses_its_new_id_without_balancing() {
    for (target, arguments) in tab_workspace_commands("workspace-next") {
        let mut before = single_pane_tab_snapshot();
        if target.is_some() {
            remove_tab(&mut before, "tab-stays");
        }
        let herdr = FakeHerdr::new(before).with_replies("pane.move", [tab_workspace_reply(target)]);

        let run = herdr.run("workspace-source", "tab-source", "pane-moving", arguments);

        run.assert_success();
        let expected = vec![
            snapshot_call(),
            move_to_new_workspace_call_with("pane-moving", tab_workspace_destination(target, None)),
            focus_call("pane-moved"),
            workspace_move_call("workspace-created", 1),
        ];
        assert_eq!(run.requests, tab_workspace_calls(target, expected));
    }
}

#[test]
fn keeps_a_custom_tab_label_on_the_new_workspace() {
    let mut snapshot = single_pane_tab_snapshot();
    snapshot["tabs"][0]["label"] = "build".into();
    let herdr = FakeHerdr::new(snapshot)
        .with_replies("pane.move", [new_workspace_reply("workspace-created")]);

    let run = herdr.run(
        "workspace-source",
        "tab-source",
        "pane-moving",
        &["tab-to-new-workspace"],
    );

    run.assert_success();
    assert_eq!(
        run.requests,
        [
            snapshot_call(),
            move_to_labeled_new_workspace_call("pane-moving", "build"),
            focus_call("pane-moved"),
            workspace_move_call("workspace-created", 1),
        ]
    );
}

#[test]
fn stops_when_herdr_declines_or_fails_the_first_tab_workspace_move() {
    for (target, arguments) in tab_workspace_commands("workspace-last") {
        for reply in [
            no_op_move_reply("source_changed"),
            Err("pane not found".to_owned()),
        ] {
            let failed = reply.is_err();
            let herdr = FakeHerdr::new(tab_workspace_before_snapshot(target))
                .with_replies("pane.move", [reply]);
            let run = herdr.run("workspace-source", "tab-source", "pane-live", arguments);
            if failed {
                assert_eq!(run.assert_failure(), "pane.move failed: pane not found\n");
            } else {
                run.assert_success();
            }
            assert_eq!(
                run.requests,
                [
                    snapshot_call(),
                    move_to_new_workspace_call_with(
                        "pane-live",
                        tab_workspace_destination(target, None)
                    )
                ]
            );
        }
    }
}

#[test]
fn reports_when_a_moved_pane_response_omits_the_created_workspace_id() {
    let herdr =
        FakeHerdr::new(single_pane_tab_snapshot()).with_replies("pane.move", [move_reply()]);

    let run = herdr.run(
        "workspace-source",
        "tab-source",
        "pane-moving",
        &["tab-to-new-workspace"],
    );

    assert_eq!(
        run.assert_failure(),
        "tab moved to a new workspace, but herdr api pane.move response is missing the created workspace id\n"
    );
    assert_eq!(
        run.requests,
        [
            snapshot_call(),
            move_to_new_workspace_call("pane-moving"),
            focus_call("pane-moved"),
        ]
    );
}

#[test]
fn reports_when_a_tab_workspace_move_response_omits_the_created_tab_id() {
    for (target, arguments) in tab_workspace_commands("workspace-last") {
        let herdr = FakeHerdr::new(tab_workspace_before_snapshot(target))
            .with_replies("pane.move", [move_reply()]);

        let run = herdr.run("workspace-source", "tab-source", "pane-live", arguments);

        assert_eq!(
            run.assert_failure(),
            "pane-live moved, but herdr api pane.move response is missing the created tab id; pane-stays remains in tab-source\n"
        );
        assert_eq!(
            run.requests,
            [
                snapshot_call(),
                move_to_new_workspace_call_with(
                    "pane-live",
                    tab_workspace_destination(target, None)
                )
            ]
        );
    }
}

#[test]
fn reports_when_a_following_pane_could_not_join_the_destination_workspace() {
    for (target, arguments) in tab_workspace_commands("workspace-last") {
        for (reply, error) in [
            (
                Err("pane not found".to_owned()),
                "pane.move failed: pane not found",
            ),
            (
                no_op_move_reply("source_changed"),
                "herdr refused to move pane-stays: source_changed",
            ),
        ] {
            let mut before = tab_workspace_before_snapshot(target);
            before["layouts"][0]["panes"] = json!([
                pane("pane-live", 0, 0, 30, 40),
                pane("pane-middle", 30, 0, 30, 40),
                pane("pane-stays", 60, 0, 40, 40),
            ]);
            let herdr = FakeHerdr::new(before).with_replies(
                "pane.move",
                [
                    tab_workspace_reply(target),
                    followed_reply("pane-middle", "pane-followed"),
                    reply,
                ],
            );
            let run = herdr.run("workspace-source", "tab-source", "pane-live", arguments);
            assert_eq!(
                run.assert_failure(),
                format!(
                    "pane-live moved, but moving pane-stays after it failed: {error}; pane-stays remains in tab-source\n"
                )
            );
            assert_eq!(
                run.requests,
                [
                    snapshot_call(),
                    move_to_new_workspace_call_with(
                        "pane-live",
                        tab_workspace_destination(target, None)
                    ),
                    move_call("pane-middle", join_destination("tab-created", "pane-moved")),
                    move_call(
                        "pane-stays",
                        join_destination("tab-created", "pane-followed")
                    ),
                ]
            );
        }
    }
}

#[test]
fn reports_when_the_tab_moved_but_the_new_workspace_could_not_be_repositioned() {
    let herdr = FakeHerdr::new(focused(
        standard_snapshot(),
        "workspace-2",
        "tab-2-1",
        "pane-2-1-top-left",
    ))
    .with_replies(
        "pane.move",
        [
            new_workspace_reply("workspace-created"),
            followed_reply("pane-2-1-top-right", "pane-followed-1"),
            followed_reply("pane-2-1-bottom-right", "pane-followed-2"),
        ],
    )
    .with_replies(
        "workspace.move",
        [Err("insert_index 2 is out of bounds".to_owned())],
    );

    let run = herdr.run(
        "workspace-2",
        "tab-2-1",
        "pane-2-1-top-left",
        &["tab-to-new-workspace"],
    );

    assert_eq!(
        run.assert_failure(),
        "tab moved to a new workspace, but positioning workspace-created after workspace-2 failed: workspace.move failed: insert_index 2 is out of bounds\n"
    );
    assert_eq!(
        run.requests,
        [
            snapshot_call(),
            move_to_new_workspace_call("pane-2-1-top-left"),
            move_call(
                "pane-2-1-top-right",
                join_destination("tab-created", "pane-moved")
            ),
            move_call(
                "pane-2-1-bottom-right",
                join_destination("tab-created", "pane-followed-1")
            ),
            focus_call("pane-moved"),
            workspace_move_call("workspace-created", 2),
        ]
    );
}

#[test]
fn reports_when_the_tab_moved_but_automatic_balance_failed() {
    for (target, arguments) in tab_workspace_commands("workspace-last") {
        let before = tab_workspace_before_snapshot(target);
        let after = tab_workspace_after_snapshot(target);
        let herdr = FakeHerdr::new(before.clone())
            .with_replies(
                "session.snapshot",
                [
                    snapshot_reply(before),
                    snapshot_reply(after.clone()),
                    snapshot_reply(after),
                ],
            )
            .with_replies("pane.move", [tab_workspace_reply(target), move_reply()])
            .with_replies("layout.export", [Err("tab is gone".to_owned())]);

        let run = herdr.run("workspace-source", "tab-source", "pane-live", arguments);

        assert_eq!(
            run.assert_failure(),
            "tab moved, but automatic balance failed: could not automatically balance tab-created: layout.export failed: tab is gone\n"
        );
        let expected = vec![
            snapshot_call(),
            move_to_new_workspace_call_with("pane-live", tab_workspace_destination(target, None)),
            move_call("pane-stays", join_destination("tab-created", "pane-moved")),
            focus_call("pane-moved"),
            workspace_move_call("workspace-created", 2),
            snapshot_call(),
            export_call_for("tab-created"),
            snapshot_call(),
        ];
        assert_eq!(run.requests, tab_workspace_calls(target, expected));
    }
}

#[test]
fn to_new_tab_places_the_new_tab_after_its_source_and_focuses_the_detached_pane() {
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
            focus_call("pane-moved"),
            tab_move_call("tab-created", 2),
        ]
    );
}

#[test]
fn creates_a_focused_default_shell_tab_immediately_after_the_current_tab() {
    // A tab in an earlier workspace makes a session-wide index differ from the
    // insertion slot inside workspace-2.
    let herdr = FakeHerdr::new(focused(
        standard_snapshot(),
        "workspace-2",
        "tab-2-2",
        "pane-2-2-top-left",
    ))
    .with_replies(
        "tab.create",
        [tab_created_reply("tab-created", "pane-created")],
    )
    .with_replies("tab.move", [tab_move_reply()]);

    let run = herdr.run("workspace-stale", "tab-stale", "pane-stale", &["new-tab"]);

    run.assert_success();
    assert_eq!(
        run.requests,
        [
            snapshot_call(),
            create_tab_call("workspace-2"),
            tab_move_call("tab-created", 2),
        ]
    );
}

#[test]
fn creates_a_focused_default_shell_workspace_immediately_after_the_current_workspace() {
    // Snapshot array order differs from visible workspace number order, so the
    // source's visible insertion slot is 2 rather than its array slot 1.
    let herdr = FakeHerdr::new(focused(
        snapshot(
            json!([
                layout("tab-source", vec![pane("pane-live", 0, 0, 100, 100)]),
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
    .with_replies(
        "workspace.create",
        [workspace_created_reply("workspace-created")],
    );

    let run = herdr.run(
        "workspace-stale",
        "tab-stale",
        "pane-stale",
        &["new-workspace"],
    );

    run.assert_success();
    assert_eq!(
        run.requests,
        [
            snapshot_call(),
            create_workspace_call(),
            workspace_move_call("workspace-created", 2),
        ]
    );
}

#[test]
fn leaves_a_created_workspace_at_the_end_when_the_current_workspace_was_last() {
    let herdr = FakeHerdr::new(focused(
        standard_snapshot(),
        "workspace-3",
        "tab-3-1",
        "pane-3-1-top-left",
    ))
    .with_replies(
        "workspace.create",
        [workspace_created_reply("workspace-created")],
    );

    let run = herdr.run(
        "workspace-3",
        "tab-3-1",
        "pane-3-1-top-left",
        &["new-workspace"],
    );

    run.assert_success();
    assert_eq!(run.requests, [snapshot_call(), create_workspace_call()]);
}

#[test]
fn reports_when_a_created_workspace_response_omits_the_workspace_id() {
    let herdr = FakeHerdr::new(focused(
        standard_snapshot(),
        "workspace-2",
        "tab-2-1",
        "pane-2-1-top-left",
    ))
    .with_replies(
        "workspace.create",
        [workspace_created_reply_without_workspace_id()],
    );

    let run = herdr.run(
        "workspace-2",
        "tab-2-1",
        "pane-2-1-top-left",
        &["new-workspace"],
    );

    assert_eq!(
        run.assert_failure(),
        "workspace creation completed, but herdr api workspace.create response is missing the created workspace id\n"
    );
    assert_eq!(run.requests, [snapshot_call(), create_workspace_call()]);
}

#[test]
fn reports_when_the_created_workspace_could_not_be_repositioned() {
    let herdr = FakeHerdr::new(focused(
        standard_snapshot(),
        "workspace-2",
        "tab-2-1",
        "pane-2-1-top-left",
    ))
    .with_replies(
        "workspace.create",
        [workspace_created_reply("workspace-created")],
    )
    .with_replies(
        "workspace.move",
        [Err("insert_index 2 is out of bounds".to_owned())],
    );

    let run = herdr.run(
        "workspace-2",
        "tab-2-1",
        "pane-2-1-top-left",
        &["new-workspace"],
    );

    assert_eq!(
        run.assert_failure(),
        "workspace creation completed, but positioning workspace-created after workspace-2 failed: workspace.move failed: insert_index 2 is out of bounds\n"
    );
    assert_eq!(
        run.requests,
        [
            snapshot_call(),
            create_workspace_call(),
            workspace_move_call("workspace-created", 2),
        ]
    );
}

#[test]
fn leaves_a_created_tab_at_the_end_when_the_current_tab_was_last() {
    let herdr = FakeHerdr::new(focused(
        standard_snapshot(),
        "workspace-2",
        "tab-2-3",
        "pane-2-3-top-left",
    ))
    .with_replies(
        "tab.create",
        [tab_created_reply("tab-created", "pane-created")],
    );

    let run = herdr.run("workspace-2", "tab-2-3", "pane-2-3-top-left", &["new-tab"]);

    run.assert_success();
    assert_eq!(
        run.requests,
        [snapshot_call(), create_tab_call("workspace-2")]
    );
}

#[test]
fn reports_when_a_created_tab_response_omits_the_tab_id() {
    let herdr = FakeHerdr::new(focused(
        standard_snapshot(),
        "workspace-2",
        "tab-2-1",
        "pane-2-1-top-left",
    ))
    .with_replies(
        "tab.create",
        [tab_created_reply_without_tab_id("pane-created")],
    );

    let run = herdr.run("workspace-2", "tab-2-1", "pane-2-1-top-left", &["new-tab"]);

    assert_eq!(
        run.assert_failure(),
        "tab creation completed, but herdr api tab.create response is missing the created tab id\n"
    );
    assert_eq!(
        run.requests,
        [snapshot_call(), create_tab_call("workspace-2")]
    );
}

#[test]
fn reports_when_the_created_tab_could_not_be_repositioned() {
    let herdr = FakeHerdr::new(focused(
        standard_snapshot(),
        "workspace-2",
        "tab-2-1",
        "pane-2-1-top-left",
    ))
    .with_replies(
        "tab.create",
        [tab_created_reply("tab-created", "pane-created")],
    )
    .with_replies(
        "tab.move",
        [Err("insert_index 1 is out of bounds".to_owned())],
    );

    let run = herdr.run("workspace-2", "tab-2-1", "pane-2-1-top-left", &["new-tab"]);

    assert_eq!(
        run.assert_failure(),
        "tab creation completed, but positioning tab-created after tab-2-1 failed: tab.move failed: insert_index 1 is out of bounds\n"
    );
    assert_eq!(
        run.requests,
        [
            snapshot_call(),
            create_tab_call("workspace-2"),
            tab_move_call("tab-created", 1),
        ]
    );
}

#[test]
fn to_new_tab_leaves_the_new_tab_at_the_end_and_focuses_the_detached_pane() {
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
            focus_call("pane-moved"),
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
fn to_new_tab_reports_partial_success_when_the_detached_pane_could_not_be_focused() {
    let herdr = FakeHerdr::new(focused(
        standard_snapshot(),
        "workspace-2",
        "tab-2-1",
        "pane-2-1-top-left",
    ))
    .with_replies("pane.move", [new_tab_reply("tab-created")])
    .with_replies("pane.focus", [Err("pane not found".to_owned())])
    .with_replies("tab.move", [tab_move_reply()]);

    let run = herdr.run(
        "workspace-2",
        "tab-2-1",
        "pane-2-1-top-left",
        &["to-new-tab"],
    );

    assert_eq!(
        run.assert_failure(),
        "pane moved to a new tab, but focusing detached pane pane-moved failed: pane.focus failed: pane not found\n"
    );
    assert_eq!(
        run.requests,
        [
            snapshot_call(),
            move_to_new_tab_call("pane-2-1-top-left", "workspace-2"),
            focus_call("pane-moved"),
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
            focus_call("pane-moved"),
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
            focus_call("pane-moved"),
            tab_move_call("tab-created", 1),
        ]
    );
}

#[test]
fn moving_a_pane_across_tab_or_workspace_edges_focuses_it_in_the_wrapped_destination() {
    for (
        scope,
        direction,
        workspace_id,
        tab_id,
        target_workspace_id,
        target_tab_id,
        moved_pane_id,
    ) in [
        (
            "workspace",
            "next",
            "workspace-3",
            "tab-3-1",
            "workspace-1",
            "tab-1-1",
            "pane-moved",
        ),
        (
            "workspace",
            "previous",
            "workspace-1",
            "tab-1-1",
            "workspace-3",
            "tab-3-1",
            "pane-moved",
        ),
        (
            "tab",
            "next",
            "workspace-2",
            "tab-2-3",
            "workspace-2",
            "tab-2-1",
            "pane-current",
        ),
        (
            "tab",
            "previous",
            "workspace-2",
            "tab-2-1",
            "workspace-2",
            "tab-2-3",
            "pane-current",
        ),
    ] {
        let herdr = FakeHerdr::new(standard_snapshot()).with_replies(
            "pane.move",
            [successful_move_reply(
                ("pane-current", workspace_id, tab_id),
                (moved_pane_id, target_workspace_id, target_tab_id),
                (
                    None,
                    layout(target_tab_id, vec![pane(moved_pane_id, 0, 0, 100, 100)]),
                ),
            )],
        );
        let run = herdr.run(workspace_id, tab_id, "pane-current", &[scope, direction]);

        run.assert_success();
        let mut expected = vec![
            call("session.snapshot", json!({})),
            call("pane.move", move_to_tab("pane-current", target_tab_id)),
            focus_call(moved_pane_id),
        ];
        if scope == "tab" {
            expected.push(snapshot_call());
        }
        assert_eq!(run.requests, expected,);
    }
}

#[test]
fn moving_a_pane_between_reordered_tabs_focuses_it_in_the_display_order_destination() {
    let herdr = FakeHerdr::new(reordered_tab_snapshot());

    for (direction, target_tab_id) in [("next", "tab-b"), ("previous", "tab-c")] {
        let run = herdr.run("workspace-1", "tab-a", "pane-a", &["tab", direction]);

        run.assert_success();
        assert_eq!(
            run.requests,
            [
                snapshot_call(),
                call("pane.move", move_to_tab("pane-a", target_tab_id)),
                focus_call("pane-a"),
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
fn directional_moves_each_geometric_neighbor_inside_a_tab_by_swapping_the_current_pane() {
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
        ))
        .with_replies("pane.swap", [swap_reply()]);

        let run = herdr.run(
            "workspace-1",
            "tab-current",
            "pane-current",
            &["move", direction],
        );

        run.assert_success();
        assert_eq!(
            run.requests,
            [snapshot_call(), swap_call("pane-current", target_id)]
        );
    }
}

#[test]
fn directional_move_leaves_the_anchor_unchanged_when_same_tab_swap_is_declined() {
    let herdr = FakeHerdr::new(snapshot(
        json!([layout(
            "tab-current",
            vec![
                pane("pane-current", 0, 0, 50, 80),
                pane("pane-right", 50, 0, 50, 80),
            ],
        )]),
        json!([tab("workspace-1", "tab-current", 1)]),
        json!([workspace("workspace-1", "tab-current", 1)]),
    ))
    .with_replies("pane.swap", [declined_swap_reply("not_found")]);
    let original_state = r#"{"paneId":"pane-current","tabId":"tab-current","workspaceId":"workspace-1","x":0.5,"y":0.5}"#;
    herdr.write_state(original_state);

    let run = herdr.run(
        "workspace-1",
        "tab-current",
        "pane-current",
        &["move", "right"],
    );

    run.assert_success();
    assert_eq!(
        run.requests,
        [snapshot_call(), swap_call("pane-current", "pane-right")]
    );
    assert_eq!(
        fs::read_to_string(herdr.state_path.join("focus-anchor.json")).unwrap(),
        original_state
    );
}

#[test]
fn returning_to_a_tab_focuses_its_balanced_edge_and_the_hook_does_not_balance_again() {
    // F was detached from F E C / A D B. The background tab still has E C / A D B.
    for direction in ["left", "down"] {
        for reason in ["entry", "pane set", "area"] {
            let across_workspaces = direction == "down";
            let mut herdr = detached_grid_herdr(across_workspaces);
            match reason {
                "pane set" => herdr.write_balance_state(
                    "tab-destination",
                    &["pane-a", "pane-b", "pane-c", "pane-d", "pane-e", "pane-f"],
                ),
                "area" => herdr.write_balance_state_with_bounds(
                    "tab-destination",
                    &["pane-a", "pane-b", "pane-c", "pane-d", "pane-e"],
                    bounds(0.0, 0.0, 100.0, 40.0),
                ),
                _ => {}
            }
            let run = herdr.run("workspace-1", "tab-source", "pane-f", &["focus", direction]);
            run.assert_success();
            assert_eq!(
                run.requests
                    .iter()
                    .filter(|call| call.method == "pane.focus")
                    .collect::<Vec<_>>(),
                [&focus_call("pane-a")],
                "{direction}: {reason}"
            );
            let moves: Vec<_> = run
                .requests
                .iter()
                .filter(|call| call.method == "pane.move")
                .collect();
            assert_eq!(moves.len(), 8);
            assert!(moves.iter().all(|call| call.params["focus"] == false));
            assert_eq!(herdr.read_anchor_state()["paneId"], "pane-a");
            assert_eq!(herdr.read_balance_state()["tabId"], "tab-destination");
            let workspace_id = if across_workspaces {
                "workspace-2"
            } else {
                "workspace-1"
            };
            herdr.snapshot = focused(
                detached_grid_snapshot(true, across_workspaces),
                workspace_id,
                "tab-destination",
                "pane-a",
            );
            herdr.replies.clear();
            let hook = run_on_pane_focused(&herdr, "tab-destination", "pane-a");
            hook.assert_success();
            assert_eq!(hook.requests, [snapshot_call()]);
        }
    }
}

#[test]
fn boundary_moves_choose_the_balanced_edge_before_a_later_refusal() {
    for direction in ["left", "down"] {
        let herdr = detached_grid_herdr(direction == "down").with_replies(
            "pane.move",
            [new_tab_reply("tab-scratch")]
                .into_iter()
                .chain((0..7).map(|_| move_reply()))
                .chain([no_op_move_reply("zoomed_tab")]),
        );
        let run = herdr.run("workspace-1", "tab-source", "pane-f", &["move", direction]);
        run.assert_success();
        assert_eq!(
            run.requests.last(),
            Some(&directional_move_call(
                "pane-f",
                "tab-destination",
                "pane-a",
                if direction == "left" { "right" } else { "down" },
                direction == "left"
            ))
        );
        assert_eq!(herdr.read_balance_state()["tabId"], "tab-source");
        assert_eq!(herdr.read_anchor_state()["paneId"], "pane-f");
    }
}

#[test]
fn failed_preparation_or_entry_keeps_the_previous_balance_record() {
    for method in ["layout.export", "pane.focus"] {
        let herdr = detached_grid_herdr(false).with_replies(method, [Err("rejected".into())]);
        let run = herdr.run("workspace-1", "tab-source", "pane-f", &["focus", "left"]);
        assert!(
            run.assert_failure()
                .contains(&format!("{method} failed: rejected"))
        );
        assert_eq!(herdr.read_balance_state()["tabId"], "tab-source");
        assert_eq!(herdr.read_anchor_state()["paneId"], "pane-f");
        if method == "layout.export" {
            assert_eq!(
                run.requests,
                [snapshot_call(), export_call_for("tab-destination")]
            );
        }
    }
}

#[test]
fn manual_arrangement_and_tab_entry_wait_for_automatic_arrangement() {
    for arguments in [&["focus", "left"][..], &["move", "left"], &["balance"]] {
        let herdr = FakeHerdr::new(reordered_tab_snapshot())
            .with_replies("layout.export", [export_reply(leaf("pane-a"))]);
        herdr.warm_up_binary();
        let held = herdr.hold_balance_lock();
        let run = thread::scope(|scope| {
            let run = scope.spawn(|| herdr.run("workspace-1", "tab-a", "pane-a", arguments));
            thread::sleep(Duration::from_millis(300));
            drop(held);
            run.join().unwrap()
        });
        run.assert_success();
        assert_eq!(herdr.premature_requests(), 0);
    }
}

#[test]
fn changing_tabs_during_preparation_cancels_entry_without_stealing_focus() {
    let before = detached_grid_snapshot(false, false);
    let changed = focused(before.clone(), "workspace-1", "tab-user", "pane-user");
    let herdr = detached_grid_herdr(false).with_replies(
        "session.snapshot",
        [snapshot_reply(before), snapshot_reply(changed)],
    );
    let run = herdr.run("workspace-1", "tab-source", "pane-f", &["focus", "left"]);
    assert!(run.assert_failure().contains("focus left tab-source"));
    assert_eq!(
        run.requests,
        [
            snapshot_call(),
            export_call_for("tab-destination"),
            snapshot_call()
        ]
    );
    assert_eq!(herdr.read_balance_state()["tabId"], "tab-source");
}

#[test]
fn a_changed_background_layout_is_not_focused_or_cached_as_balanced() {
    for changed in ["pane order", "area"] {
        let before = detached_grid_snapshot(true, false);
        let mut after = before.clone();
        after["layouts"][0]["panes"][2]["rect"]["width"] = 41.into();
        let tree = split(
            "down",
            split(
                "right",
                split("right", leaf("pane-e"), leaf("pane-c")),
                leaf("pane-a"),
            ),
            split("right", leaf("pane-d"), leaf("pane-b")),
        );
        let tree = if changed == "pane order" {
            split(
                "down",
                split(
                    "right",
                    split("right", leaf("pane-c"), leaf("pane-e")),
                    leaf("pane-a"),
                ),
                split("right", leaf("pane-d"), leaf("pane-b")),
            )
        } else {
            tree
        };
        let herdr = FakeHerdr::new(before.clone())
            .with_replies("layout.export", [export_reply(tree)])
            .with_replies("pane.swap", [swap_reply()])
            .with_replies(
                "session.snapshot",
                [
                    snapshot_reply(before.clone()),
                    snapshot_reply(if changed == "area" { after } else { before }),
                ],
            );
        herdr.write_balance_state("tab-source", &["pane-f"]);
        let run = herdr.run("workspace-1", "tab-source", "pane-f", &["focus", "left"]);
        assert!(run.assert_failure().contains("changed while"), "{changed}");
        assert!(run.requests.iter().all(|call| !matches!(
            call.method.as_str(),
            "pane.focus" | "pane.swap" | "pane.move"
        )));
        assert_eq!(herdr.read_balance_state()["tabId"], "tab-source");
    }
}

fn detached_grid_herdr(across_workspaces: bool) -> FakeHerdr {
    let before = detached_grid_snapshot(false, across_workspaces);
    let balanced = detached_grid_snapshot(true, across_workspaces);
    let herdr = FakeHerdr::new(before.clone())
        .with_replies(
            "session.snapshot",
            (0..9)
                .map(|_| snapshot_reply(before.clone()))
                .chain([snapshot_reply(balanced.clone())]),
        )
        .with_replies(
            "layout.export",
            [
                export_reply(split(
                    "down",
                    split("right", leaf("pane-e"), leaf("pane-c")),
                    split(
                        "right",
                        split("right", leaf("pane-a"), leaf("pane-d")),
                        leaf("pane-b"),
                    ),
                )),
                export_reply(split(
                    "down",
                    split(
                        "right",
                        split("right", leaf("pane-e"), leaf("pane-c")),
                        leaf("pane-a"),
                    ),
                    split("right", leaf("pane-d"), leaf("pane-b")),
                )),
            ],
        )
        .with_replies("pane.move", [new_tab_reply("tab-scratch"), move_reply()]);
    herdr.write_state(
        r#"{"paneId":"pane-f","tabId":"tab-source","workspaceId":"workspace-1","x":0.75,"y":0.25}"#,
    );
    herdr.write_balance_state("tab-source", &["pane-f"]);
    herdr
}

fn detached_grid_snapshot(balanced: bool, across_workspaces: bool) -> Value {
    let panes = if balanced {
        vec![
            pane("pane-e", 0, 0, 40, 20),
            pane("pane-c", 40, 0, 40, 20),
            pane("pane-a", 80, 0, 40, 20),
            pane("pane-d", 0, 20, 60, 20),
            pane("pane-b", 60, 20, 60, 20),
        ]
    } else {
        vec![
            pane("pane-e", 0, 0, 80, 20),
            pane("pane-c", 80, 0, 40, 20),
            pane("pane-a", 0, 20, 40, 20),
            pane("pane-d", 40, 20, 40, 20),
            pane("pane-b", 80, 20, 40, 20),
        ]
    };
    let mut result = focused(
        snapshot(
            json!([
                layout("tab-destination", panes),
                layout("tab-source", vec![pane("pane-f", 0, 0, 120, 40)])
            ]),
            json!([
                tab("workspace-1", "tab-destination", 1),
                tab("workspace-1", "tab-source", 2)
            ]),
            json!([workspace("workspace-1", "tab-source", 1)]),
        ),
        "workspace-1",
        "tab-source",
        "pane-f",
    );
    if across_workspaces {
        result["tabs"][0]["workspace_id"] = "workspace-2".into();
        result["workspaces"].as_array_mut().unwrap().push(workspace(
            "workspace-2",
            "tab-destination",
            2,
        ));
    }
    result
}

#[test]
fn focus_wraps_across_tabs_and_workspaces_in_visual_order() {
    for (direction, workspace_id, tab_id, target_tab_id, expected) in [
        (
            "left",
            "workspace-2",
            "tab-2-1",
            "tab-2-3",
            "pane-2-3-bottom-right",
        ),
        (
            "right",
            "workspace-2",
            "tab-2-3",
            "tab-2-1",
            "pane-2-1-top-left",
        ),
        (
            "up",
            "workspace-1",
            "tab-1-1",
            "tab-3-1",
            "pane-3-bottom-right",
        ),
        (
            "down",
            "workspace-3",
            "tab-3-1",
            "tab-1-1",
            "pane-1-top-left",
        ),
    ] {
        let mut layouts = standard_layouts();
        let current = layouts
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|layout| layout["tab_id"] == tab_id)
            .unwrap();
        *current = layout(tab_id, vec![pane("pane-current", 0, 0, 100, 80)]);
        let herdr = FakeHerdr::new(snapshot(layouts, standard_tabs(), standard_workspaces()))
            .with_balanced_tab(target_tab_id);

        let run = herdr.run(workspace_id, tab_id, "pane-current", &["focus", direction]);

        run.assert_success();
        assert_eq!(run.requests, [snapshot_call(), focus_call(expected)]);
    }
}

#[test]
fn moving_across_containers_places_and_focuses_the_pane_at_its_directional_boundary() {
    for (direction, workspace_id, tab_id, target_tab_id, target_pane_id, split, swap) in [
        (
            "left",
            "workspace-2",
            "tab-2-1",
            "tab-2-3",
            "pane-2-3-bottom-right",
            "right",
            false,
        ),
        (
            "right",
            "workspace-2",
            "tab-2-3",
            "tab-2-1",
            "pane-2-1-top-left",
            "right",
            true,
        ),
        (
            "up",
            "workspace-1",
            "tab-1-1",
            "tab-3-1",
            "pane-3-bottom-right",
            "down",
            true,
        ),
        (
            "down",
            "workspace-3",
            "tab-3-1",
            "tab-1-1",
            "pane-1-top-left",
            "down",
            false,
        ),
    ] {
        let mut layouts = standard_layouts();
        let current = layouts
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|layout| layout["tab_id"] == tab_id)
            .unwrap();
        *current = layout(tab_id, vec![pane("pane-current", 0, 0, 100, 80)]);
        let herdr = FakeHerdr::new(snapshot(layouts, standard_tabs(), standard_workspaces()))
            .with_replies("pane.swap", [swap_reply()])
            .with_balanced_tab(target_tab_id);

        let run = herdr.run(workspace_id, tab_id, "pane-current", &["move", direction]);

        run.assert_success();
        let mut expected = vec![
            snapshot_call(),
            directional_move_call("pane-current", target_tab_id, target_pane_id, split, !swap),
        ];
        if swap {
            expected.push(swap_call("pane-current", target_pane_id));
        }
        expected.push(focus_call("pane-current"));
        expected.push(snapshot_call());
        assert_eq!(run.requests, expected);
    }
}

#[test]
fn vertical_boundary_moves_enter_before_a_target_to_preserve_a_left_anchor() {
    for (
        direction,
        workspace_id,
        tab_id,
        target_workspace_id,
        target_tab_id,
        target_pane_id,
        moved_pane_id,
        anchor_x,
        swap,
    ) in [
        (
            "up",
            "workspace-1",
            "tab-1-1",
            "workspace-3",
            "tab-3-1",
            "pane-up-target",
            "pane-moved-up",
            0.25,
            true,
        ),
        (
            "down",
            "workspace-3",
            "tab-3-1",
            "workspace-1",
            "tab-1-1",
            "pane-down-target",
            "pane-moved-down",
            0.25,
            true,
        ),
        (
            "down",
            "workspace-3",
            "tab-3-1",
            "workspace-1",
            "tab-1-1",
            "pane-equal-target",
            "pane-moved-at-equality",
            0.5,
            false,
        ),
    ] {
        let mut layouts = standard_layouts();
        for (layout_id, pane_id) in [(tab_id, "pane-current"), (target_tab_id, target_pane_id)] {
            let current = layouts
                .as_array_mut()
                .unwrap()
                .iter_mut()
                .find(|layout| layout["tab_id"] == layout_id)
                .unwrap();
            *current = layout(layout_id, vec![pane(pane_id, 0, 0, 100, 80)]);
        }
        let herdr = FakeHerdr::new(snapshot(layouts, standard_tabs(), standard_workspaces()))
            .with_replies(
                "pane.move",
                [successful_move_reply(
                    ("pane-current", workspace_id, tab_id),
                    (moved_pane_id, target_workspace_id, target_tab_id),
                    (
                        None,
                        layout(
                            target_tab_id,
                            vec![
                                pane(target_pane_id, 0, 0, 50, 80),
                                pane(moved_pane_id, 50, 0, 50, 80),
                            ],
                        ),
                    ),
                )],
            );
        let herdr = if swap {
            herdr.with_replies("pane.swap", [swap_reply()])
        } else {
            herdr
        };
        herdr.write_state(
            &json!({
                "paneId": "pane-current",
                "tabId": tab_id,
                "workspaceId": workspace_id,
                "x": anchor_x,
                "y": 0.5,
            })
            .to_string(),
        );

        let run = herdr.run(workspace_id, tab_id, "pane-current", &["move", direction]);

        run.assert_success();
        let mut expected = vec![
            snapshot_call(),
            directional_move_call("pane-current", target_tab_id, target_pane_id, "down", !swap),
        ];
        if swap {
            expected.push(swap_call(moved_pane_id, target_pane_id));
        }
        expected.push(focus_call(moved_pane_id));
        expected.push(snapshot_call());
        assert_eq!(
            run.requests, expected,
            "{direction} with anchor x={anchor_x}"
        );
        let state = herdr.read_anchor_state();
        assert_eq!(state["paneId"], moved_pane_id);
        assert_eq!(state["tabId"], target_tab_id);
        assert_eq!(state["workspaceId"], target_workspace_id);
        assert_eq!(state["x"], anchor_x);
    }
}

#[test]
fn directional_moves_remember_the_cross_axis_anchor_across_swaps() {
    let mut herdr = anchor_herdr().with_replies("pane.swap", [swap_reply()]);

    let first = herdr.run("workspace-1", "tab-anchor", "pane-a", &["move", "right"]);
    first.assert_success();
    assert_eq!(
        first.requests,
        [snapshot_call(), swap_call("pane-a", "pane-b")]
    );

    herdr.snapshot = anchor_positions("pane-b", "pane-a", "pane-c");
    let second = herdr.run("workspace-1", "tab-anchor", "pane-a", &["move", "down"]);
    second.assert_success();
    assert_eq!(
        second.requests,
        [snapshot_call(), swap_call("pane-a", "pane-c")]
    );

    herdr.snapshot = anchor_positions("pane-b", "pane-c", "pane-a");
    let third = herdr.run("workspace-1", "tab-anchor", "pane-a", &["move", "left"]);
    third.assert_success();
    assert_eq!(
        third.requests,
        [snapshot_call(), swap_call("pane-a", "pane-b")]
    );

    herdr.snapshot = anchor_positions("pane-a", "pane-c", "pane-b");
    let fourth = herdr.run("workspace-1", "tab-anchor", "pane-a", &["move", "right"]);
    fourth.assert_success();
    assert_eq!(
        fourth.requests,
        [snapshot_call(), swap_call("pane-a", "pane-b")]
    );
}

#[test]
fn moving_down_uses_the_live_snapshot_and_focuses_the_new_pane_id_across_workspaces() {
    let mut layouts = standard_layouts();
    let current = layouts
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|layout| layout["tab_id"] == "tab-3-1")
        .unwrap();
    *current = layout("tab-3-1", vec![pane("pane-live", 0, 0, 100, 80)]);
    let before = focused(
        snapshot(layouts, standard_tabs(), standard_workspaces()),
        "workspace-3",
        "tab-3-1",
        "pane-live",
    );
    let herdr = FakeHerdr::new(before).with_replies(
        "pane.move",
        [successful_move_reply(
            ("pane-live", "workspace-3", "tab-3-1"),
            ("pane-new", "workspace-1", "tab-1-1"),
            (
                None,
                layout(
                    "tab-1-1",
                    vec![
                        pane("pane-1-top-left", 0, 0, 50, 80),
                        pane("pane-new", 50, 0, 50, 80),
                    ],
                ),
            ),
        )],
    );

    let herdr = herdr.with_balanced_tab("tab-1-1");
    let run = herdr.run(
        "workspace-stale",
        "tab-stale",
        "pane-stale",
        &["move", "down"],
    );

    run.assert_success();
    assert_eq!(
        run.requests,
        [
            snapshot_call(),
            directional_move_call("pane-live", "tab-1-1", "pane-1-top-left", "down", true,),
            focus_call("pane-new"),
            snapshot_call(),
            snapshot_call(),
        ]
    );
    let state = herdr.read_anchor_state();
    assert_eq!(state["paneId"], "pane-new");
    assert_eq!(state["tabId"], "tab-1-1");
    assert_eq!(state["workspaceId"], "workspace-1");
}

#[test]
fn directional_move_stops_without_changes_when_herdr_declines_the_move() {
    let mut layouts = standard_layouts();
    let current = layouts
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|layout| layout["tab_id"] == "tab-2-1")
        .unwrap();
    *current = layout("tab-2-1", vec![pane("pane-current", 0, 0, 100, 80)]);
    let herdr = FakeHerdr::new(snapshot(layouts, standard_tabs(), standard_workspaces()))
        .with_replies("pane.move", [no_op_move_reply("zoomed_tab")])
        .with_balanced_tab("tab-2-3");
    let original_state = r#"{"paneId":"pane-current","tabId":"tab-2-1","workspaceId":"workspace-2","x":0.5,"y":0.5}"#;
    herdr.write_state(original_state);

    let run = herdr.run("workspace-2", "tab-2-1", "pane-current", &["move", "left"]);

    run.assert_success();
    assert_eq!(
        run.requests,
        [
            snapshot_call(),
            directional_move_call(
                "pane-current",
                "tab-2-3",
                "pane-2-3-bottom-right",
                "right",
                true,
            ),
        ]
    );
    assert_eq!(
        fs::read_to_string(herdr.state_path.join("focus-anchor.json")).unwrap(),
        original_state
    );
}

#[test]
fn directional_move_reports_partial_success_when_the_followup_swap_fails() {
    for (direction, workspace_id, tab_id, target_tab_id, target_pane_id, split, herdr) in [
        (
            "right",
            "workspace-2",
            "tab-2-3",
            "tab-2-1",
            "pane-2-1-top-left",
            "right",
            right_boundary_herdr(Err("target disappeared".to_owned())),
        ),
        (
            "down",
            "workspace-3",
            "tab-3-1",
            "tab-1-1",
            "pane-down-target",
            "down",
            down_boundary_herdr(Err("target disappeared".to_owned())),
        ),
    ] {
        let original_state = fs::read_to_string(herdr.state_path.join("focus-anchor.json")).ok();
        let run = herdr.run(workspace_id, tab_id, "pane-current", &["move", direction]);

        assert_eq!(
            run.assert_failure(),
            "pane moved, but directional swap failed: pane.swap failed: target disappeared\n",
            "{direction} boundary move should report partial success"
        );
        assert_eq!(
            run.requests,
            [
                snapshot_call(),
                directional_move_call("pane-current", target_tab_id, target_pane_id, split, false),
                swap_call("pane-new", target_pane_id),
            ],
            "{direction} boundary move should stop before anchor, focus, or balance"
        );
        assert_eq!(
            fs::read_to_string(herdr.state_path.join("focus-anchor.json")).ok(),
            original_state,
            "{direction} boundary move should not record a new anchor"
        );
    }
}

#[test]
fn directional_move_reports_partial_success_when_the_followup_swap_is_declined() {
    let herdr = right_boundary_herdr(declined_swap_reply("cross_tab"));

    let run = herdr.run("workspace-2", "tab-2-3", "pane-current", &["move", "right"]);

    assert_eq!(
        run.assert_failure(),
        "pane moved, but directional swap failed: cross_tab\n"
    );
    assert_eq!(
        run.requests,
        [
            snapshot_call(),
            directional_move_call(
                "pane-current",
                "tab-2-1",
                "pane-2-1-top-left",
                "right",
                false,
            ),
            swap_call("pane-new", "pane-2-1-top-left"),
        ]
    );
    assert!(!herdr.state_path.join("focus-anchor.json").exists());
}

#[test]
fn directional_move_preserves_partial_success_context_for_an_invalid_swap_response() {
    let herdr = right_boundary_herdr(Ok(json!({
        "type": "pane_swap",
        "swap": {"reason": "cross_tab"},
    })));

    let run = herdr.run("workspace-2", "tab-2-3", "pane-current", &["move", "right"]);

    assert_eq!(
        run.assert_failure(),
        "pane moved, but directional swap failed: herdr api pane.swap returned an invalid response\n"
    );
    assert_eq!(run.requests.len(), 3);
    assert_eq!(
        run.requests.last(),
        Some(&swap_call("pane-new", "pane-2-1-top-left"))
    );
    assert!(!herdr.state_path.join("focus-anchor.json").exists());
}

#[test]
fn moving_right_to_another_tab_records_the_new_anchor_then_stops_before_balance_when_focus_fails() {
    let herdr = right_boundary_herdr(swap_reply())
        .with_replies("pane.focus", [Err("pane not found".to_owned())]);

    let run = herdr.run("workspace-2", "tab-2-3", "pane-current", &["move", "right"]);

    assert_eq!(
        run.assert_failure(),
        "pane moved, but focusing pane-new failed: pane.focus failed: pane not found\n"
    );
    assert_eq!(
        run.requests,
        [
            snapshot_call(),
            directional_move_call(
                "pane-current",
                "tab-2-1",
                "pane-2-1-top-left",
                "right",
                false,
            ),
            swap_call("pane-new", "pane-2-1-top-left"),
            focus_call("pane-new"),
        ]
    );
    let state = herdr.read_anchor_state();
    assert_eq!(state["paneId"], "pane-new");
    assert_eq!(state["tabId"], "tab-2-1");
    assert_eq!(state["workspaceId"], "workspace-2");
}

#[test]
fn directional_move_leaves_its_own_target_unchanged() {
    let herdr = FakeHerdr::new(snapshot(
        json!([layout("tab-only", vec![pane("pane-only", 0, 0, 100, 80)])]),
        json!([tab("workspace-only", "tab-only", 1)]),
        json!([workspace("workspace-only", "tab-only", 1)]),
    ));
    let original_state = r#"{"paneId":"pane-only","tabId":"tab-only","workspaceId":"workspace-only","x":0.5,"y":0.5}"#;
    herdr.write_state(original_state);

    let run = herdr.run("workspace-only", "tab-only", "pane-only", &["move", "left"]);

    run.assert_success();
    assert_eq!(run.requests, [snapshot_call()]);
    assert_eq!(
        fs::read_to_string(herdr.state_path.join("focus-anchor.json")).unwrap(),
        original_state
    );
}

#[test]
fn moving_left_focuses_the_destination_pane_before_reporting_automatic_balance_failure() {
    let before = focused(
        snapshot(
            json!([
                layout("tab-destination", vec![pane("pane-target", 0, 0, 100, 80)],),
                layout("tab-source", vec![pane("pane-moving", 0, 0, 100, 80)]),
            ]),
            json!([
                tab("workspace-1", "tab-destination", 1),
                tab("workspace-1", "tab-source", 2),
            ]),
            json!([workspace("workspace-1", "tab-source", 1)]),
        ),
        "workspace-1",
        "tab-source",
        "pane-moving",
    );
    let after = focused(
        snapshot(
            json!([
                layout(
                    "tab-destination",
                    vec![
                        pane("pane-target", 0, 0, 100, 80),
                        pane("pane-moving", 100, 0, 100, 80),
                    ],
                ),
                layout("tab-source", Vec::new()),
            ]),
            json!([tab("workspace-1", "tab-destination", 1)]),
            json!([workspace("workspace-1", "tab-destination", 1)]),
        ),
        "workspace-1",
        "tab-destination",
        "pane-moving",
    );
    let herdr = FakeHerdr::new(before.clone())
        .with_replies(
            "session.snapshot",
            [
                snapshot_reply(before),
                snapshot_reply(after.clone()),
                snapshot_reply(after),
            ],
        )
        .with_replies("layout.export", [Err("layout unavailable".to_owned())]);

    let run = herdr.run(
        "workspace-1",
        "tab-source",
        "pane-moving",
        &["move", "left"],
    );

    assert_eq!(
        run.assert_failure(),
        "pane moved, but automatic balance failed: could not automatically balance \
         tab-destination: layout.export failed: layout unavailable\n"
    );
    assert_eq!(
        run.requests,
        [
            snapshot_call(),
            directional_move_call(
                "pane-moving",
                "tab-destination",
                "pane-target",
                "right",
                true,
            ),
            focus_call("pane-moving"),
            snapshot_call(),
            export_call_for("tab-destination"),
            snapshot_call(),
        ]
    );
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

    let mut state_files: Vec<_> = fs::read_dir(&herdr.state_path)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    state_files.sort();
    assert_eq!(state_files, ["balance-focus.lock", "focus-anchor.json"]);
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
            focus_call("pane-live"),
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
            focus_call("pane-moving"),
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
        json!({
            "bounds": bounds(0.0, 0.0, 100.0, 40.0),
            "paneIds": ["pane-destination"],
            "tabId": "tab-destination",
        })
    );
}

#[test]
fn moving_a_pane_to_another_tab_stops_before_balance_when_destination_focus_fails() {
    let herdr = FakeHerdr::new(tab_move_before_snapshot())
        .with_replies("pane.focus", [Err("pane not found".to_owned())]);

    let run = herdr.run(
        "workspace-1",
        "tab-source",
        "pane-moving",
        &["tab", "previous"],
    );

    assert_eq!(
        run.assert_failure(),
        "pane moved, but focusing pane-moving failed: pane.focus failed: pane not found\n"
    );
    assert_eq!(
        run.requests,
        [
            snapshot_call(),
            call("pane.move", move_to_tab("pane-moving", "tab-destination")),
            focus_call("pane-moving"),
        ]
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
            focus_call("pane-moving"),
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
    let herdr = herdr.with_balanced_tab("tab-target");
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
    let herdr = herdr.with_balanced_tab("tab-target");
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
fn rejects_the_removed_pane_creation_alias() {
    let output = ProcessCommand::new(env!("CARGO_BIN_EXE_herdr-grid-slide"))
        .arg("split-pane")
        .env_clear()
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.starts_with("usage:"), "{stderr}");
    assert!(!stderr.contains("split-pane"));
}

#[test]
fn new_pane_uses_the_live_focus_instead_of_completing_the_grid_elsewhere() {
    let herdr = FakeHerdr::new(focused(
        tab_snapshot(vec![
            pane("pane-a", 0, 0, 40, 20),
            pane("pane-b", 40, 0, 40, 20),
            pane("pane-c", 80, 0, 40, 20),
            pane("pane-d", 0, 20, 60, 20),
            pane("pane-e", 60, 20, 60, 20),
        ]),
        "workspace-1",
        "tab-main",
        "pane-e",
    ))
    .with_replies(
        "layout.export",
        [export_reply(split(
            "down",
            split(
                "right",
                split("right", leaf("pane-a"), leaf("pane-b")),
                leaf("pane-c"),
            ),
            split("right", leaf("pane-d"), leaf("pane-e")),
        ))],
    )
    .with_replies("pane.split", [split_reply("pane-new")]);

    let run = run_new_pane(&herdr, "pane-a");
    run.assert_success();
    assert_eq!(
        run.requests,
        [
            snapshot_call(),
            export_call(),
            split_call("pane-e", "right")
        ]
    );
}

#[test]
fn new_pane_only_splits_in_the_existing_direction() {
    // Preserve one-way layouts even when the focused pane's aspect ratio favors
    // the other direction. Both insert immediately after the focus.
    for (direction, panes) in [
        (
            "right",
            vec![
                pane("pane-a", 0, 0, 20, 100),
                pane("pane-b", 20, 0, 80, 100),
            ],
        ),
        (
            "down",
            vec![
                pane("pane-a", 0, 0, 100, 20),
                pane("pane-b", 0, 20, 100, 80),
            ],
        ),
    ] {
        let herdr = FakeHerdr::new(tab_snapshot(panes))
            .with_replies(
                "layout.export",
                [export_reply(split(
                    direction,
                    leaf("pane-a"),
                    leaf("pane-b"),
                ))],
            )
            .with_replies("pane.split", [split_reply("pane-new")]);
        let run = run_new_pane(&herdr, "pane-a");

        run.assert_success();
        assert_eq!(
            run.requests,
            [
                snapshot_call(),
                export_call(),
                split_call("pane-a", direction),
            ]
        );
    }
}

#[test]
fn new_pane_follows_a_tall_focused_pane_before_its_row_neighbor() {
    let herdr = FakeHerdr::new(tab_snapshot(vec![
        pane("pane-a", 0, 0, 60, 40),
        pane("pane-b", 60, 0, 60, 40),
        pane("pane-c", 0, 40, 120, 40),
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

    let run = run_new_pane(&herdr, "pane-a");

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
fn new_pane_splits_downward_on_the_two_to_one_boundary() {
    // Cells are about twice as tall as wide, so a pane only counts as wide once
    // its width passes twice its height. The boundary itself splits downwards.
    let herdr = FakeHerdr::new(tab_snapshot(vec![pane("pane-a", 0, 0, 80, 40)]))
        .with_replies("layout.export", [export_reply(leaf("pane-a"))])
        .with_replies("pane.split", [split_reply("pane-new")]);

    let run = run_new_pane(&herdr, "pane-a");

    run.assert_success();
    assert_eq!(
        run.requests,
        [snapshot_call(), export_call(), split_call("pane-a", "down"),]
    );
}

#[test]
fn new_pane_stops_before_splitting_a_zoomed_tab() {
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

    let run = run_new_pane(&herdr, "pane-a");

    assert_eq!(
        run.assert_failure(),
        "tab-main is zoomed; unzoom it before adding a pane\n"
    );
    assert_eq!(run.requests, [snapshot_call(), export_call()]);
}

#[test]
fn new_pane_stops_before_splitting_when_snapshot_and_layout_disagree() {
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

    let run = run_new_pane(&herdr, "pane-a");

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
            json!({
                "bounds": bounds(0.0, 0.0, 100.0, 40.0),
                "paneIds": ["pane-a", "pane-b"],
                "tabId": "tab-main",
            }),
        );
    }
}

#[test]
fn pane_closed_balances_the_focused_tab_without_a_tab_environment_variable() {
    let herdr = FakeHerdr::new(focused(
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
        [export_reply(split("right", leaf("pane-a"), leaf("pane-b")))],
    );
    herdr.write_balance_state("tab-main", &["pane-a", "pane-b", "pane-closed"]);

    let run = herdr.run_without_tab("workspace-1", "pane-closed", &["on-pane-focused"]);

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
        json!({
            "bounds": bounds(0.0, 0.0, 100.0, 40.0),
            "paneIds": ["pane-a", "pane-b"],
            "tabId": "tab-main",
        }),
    );
}

#[test]
fn balance_still_requires_a_tab_environment_variable() {
    let herdr = FakeHerdr::new(tab_snapshot(vec![
        pane("pane-a", 0, 0, 50, 40),
        pane("pane-b", 50, 0, 50, 40),
    ]));

    let run = herdr.run_without_tab("workspace-1", "pane-a", &["balance"]);

    assert_eq!(run.assert_failure(), "missing HERDR_TAB_ID\n");
    assert!(run.requests.is_empty());
}

#[test]
fn pane_focused_balances_the_same_tab_when_its_area_changed() {
    // Herdr sends no event when a client of another size attaches or the window
    // is resized, so the first focus afterwards has to notice that the grid was
    // laid out for a different shape. The tab was a row of two on the desktop; on
    // a phone-shaped area that grid is the wrong shape and becomes a column.
    let before = focused(
        tab_snapshot(vec![
            pane("pane-a", 0, 0, 50, 100),
            pane("pane-b", 50, 0, 50, 100),
        ]),
        "workspace-1",
        "tab-main",
        "pane-b",
    );
    let remaining = focused(
        tab_snapshot(vec![pane("pane-a", 0, 0, 100, 100)]),
        "workspace-1",
        "tab-main",
        "pane-a",
    );
    let herdr = FakeHerdr::new(before.clone())
        .with_replies(
            "layout.export",
            [
                export_reply(split("right", leaf("pane-a"), leaf("pane-b"))),
                export_reply(split("down", leaf("pane-a"), leaf("pane-b"))),
            ],
        )
        .with_replies(
            "pane.move",
            [
                new_tab_reply_with_source(
                    "tab-scratch",
                    focused_layout("tab-main", "pane-a", vec![pane("pane-a", 0, 0, 100, 100)]),
                ),
                move_reply(),
            ],
        )
        .with_replies(
            "session.snapshot",
            [
                snapshot_reply(before.clone()),
                snapshot_reply(before.clone()),
                snapshot_reply(remaining.clone()),
                snapshot_reply(remaining),
                snapshot_reply(before),
            ],
        );
    herdr.write_balance_state_with_bounds(
        "tab-main",
        &["pane-a", "pane-b"],
        bounds(0.0, 0.0, 200.0, 40.0),
    );

    let run = run_on_pane_focused(&herdr, "tab-main", "pane-b");

    run.assert_success();
    // Restore the selection from Herdr's chosen return target after rebuilding.
    assert_eq!(
        run.requests,
        [
            snapshot_call(),
            export_call(),
            snapshot_call(),
            move_call("pane-b", new_tab_destination()),
            snapshot_call(),
            move_call("pane-b", attach_destination("pane-a", "down")),
            export_call(),
            ratio_call(&[], 0.5),
            snapshot_call(),
            focus_call("pane-b"),
            snapshot_call(),
        ]
    );
    assert_eq!(
        herdr.read_balance_state(),
        json!({
            "bounds": bounds(0.0, 0.0, 100.0, 100.0),
            "paneIds": ["pane-a", "pane-b"],
            "tabId": "tab-main",
        }),
    );
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
fn automatic_arrangement_keeps_the_users_selection_and_ignores_queued_internal_events() {
    let entered = focused(tangled_snapshot(), "workspace-1", "tab-main", "pane-c");
    let mut herdr = tangled_herdr()
        .with_snapshot(entered.clone())
        .with_replies(
            "session.snapshot",
            std::iter::repeat_n(snapshot_reply(entered.clone()), 2)
                .chain(std::iter::repeat_n(
                    snapshot_reply(focused(
                        tangled_snapshot(),
                        "workspace-1",
                        "tab-main",
                        "pane-a",
                    )),
                    6,
                ))
                .chain([snapshot_reply(entered)]),
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
                        "pane-a",
                        vec![
                            pane("pane-b", 0, 0, 100, 100),
                            pane("pane-a", 100, 0, 100, 50),
                            pane("pane-d", 100, 50, 100, 50),
                        ],
                    ),
                ),
                move_reply(),
            ],
        );

    let first = run_on_pane_focused(&herdr, "tab-main", "pane-a");
    first.assert_success();
    assert!(first.requests.iter().any(|call| call.method == "pane.move"));
    assert!(first.requests.contains(&focus_call("pane-c")));
    assert!(
        !first
            .requests
            .iter()
            .any(|call| { call.method == "pane.move" && call.params["pane_id"] == "pane-a" })
    );
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

    herdr.write_balance_state("tab-before", &[]);
    herdr.replies.insert(
        "pane.focus".to_owned(),
        [Err("return target vanished".to_owned())]
            .into_iter()
            .collect(),
    );
    let restore_failed = run_on_pane_focused(&herdr, "tab-main", "pane-c");
    assert!(
        restore_failed
            .assert_failure()
            .contains("return target vanished")
    );
    assert!(restore_failed.requests.contains(&focus_call("pane-c")));

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

    let before = focused(tangled_snapshot(), "workspace-1", "tab-main", "pane-c");
    let selected = focused(tangled_snapshot(), "workspace-1", "tab-main", "pane-b");
    // Change selection before another move, then before the final position swap.
    for checks_before_selection in [2, 7] {
        let interrupted = tangled_herdr()
            .with_snapshot(before.clone())
            .with_replies(
                "session.snapshot",
                std::iter::repeat_n(snapshot_reply(before.clone()), 2)
                    .chain(std::iter::repeat_n(
                        snapshot_reply(focused(
                            tangled_snapshot(),
                            "workspace-1",
                            "tab-main",
                            "pane-d",
                        )),
                        checks_before_selection - 2,
                    ))
                    .chain([snapshot_reply(selected.clone())]),
            )
            .with_replies(
                "pane.move",
                [
                    new_tab_reply_with_source(
                        "tab-scratch",
                        focused_layout(
                            "tab-main",
                            "pane-d",
                            vec![
                                pane("pane-a", 0, 0, 100, 100),
                                pane("pane-b", 100, 0, 100, 50),
                                pane("pane-d", 100, 50, 100, 50),
                            ],
                        ),
                    ),
                    move_reply(),
                ],
            );

        let run = run_on_pane_focused(&interrupted, "tab-main", "pane-c");

        assert!(run.assert_failure().contains("focus left pane-d"));
        assert!(
            !run.requests
                .iter()
                .any(|call| matches!(call.method.as_str(), "pane.focus" | "pane.swap"))
        );
        assert_eq!(run.requests.last(), Some(&snapshot_call()));
        if checks_before_selection == 2 {
            assert!(
                run.requests
                    .contains(&move_call("pane-c", attach_destination("pane-d", "right")))
            );
        } else {
            assert_eq!(
                run.requests
                    .iter()
                    .filter(|call| call.method == "pane.move")
                    .count(),
                6
            );
        }
    }
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
fn balance_aligns_columns_without_rebuilding_differently_nested_rows() {
    for width in [119, 120, 121, 122, 127, 128] {
        for automatic in [false, true] {
            let herdr = FakeHerdr::new(focused(
                tab_snapshot(vec![
                    pane("pane-a", 0, 0, width / 3, 20),
                    pane("pane-b", width / 3, 0, width / 3, 20),
                    pane("pane-c", 2 * (width / 3), 0, width - 2 * (width / 3), 20),
                    pane("pane-d", 0, 20, width / 3, 20),
                    pane("pane-e", width / 3, 20, width / 3, 20),
                    pane("pane-f", 2 * (width / 3), 20, width - 2 * (width / 3), 20),
                ]),
                "workspace-1",
                "tab-main",
                "pane-a",
            ))
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
            let run = if automatic {
                herdr.run("workspace-1", "tab-main", "pane-a", &["on-pane-focused"])
            } else {
                run_balance(&herdr, "pane-a")
            };
            run.assert_success();
            let updates: Vec<_> = run
                .requests
                .iter()
                .filter(|call| call.method == "layout.set_split_ratio")
                .collect();
            assert_eq!(updates.len(), 5, "width {width}, automatic {automatic}");
            // Match Herdr's f32 multiplication and rounding, including each nested split.
            let cells = |size: u16, index: usize| {
                (size as f32 * updates[index].params["ratio"].as_f64().unwrap() as f32).round()
                    as u16
            };
            let top_first = cells(width, 1);
            let top_second = top_first + cells(width - top_first, 2);
            let bottom_second = cells(width, 3);
            let bottom_first = cells(bottom_second, 4);
            assert_eq!(
                (top_first, top_second),
                (bottom_first, bottom_second),
                "width {width}, automatic {automatic}"
            );
            assert_eq!(top_first, (width as f64 / 3.0).round() as u16);
            assert_eq!(top_second, (width as f64 * 2.0 / 3.0).round() as u16);
            assert!(run.requests.iter().all(|call| matches!(
                call.method.as_str(),
                "session.snapshot" | "layout.export" | "layout.set_split_ratio"
            )));
        }
    }
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
            ratio_call(&[], 67.0 / 100.0),
            ratio_call(&[false], 33.0 / 67.0),
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
    // Explicit balance keeps its existing best-effort focus restoration on failure.
    let mut herdr = tangled_herdr()
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
                            pane("pane-b", 100, 0, 100, 50),
                            pane("pane-d", 100, 50, 100, 50),
                        ],
                    ),
                ),
                move_reply(),
            ],
        )
        .with_replies("pane.swap", [swap_reply()])
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

    herdr.replies.insert(
        "pane.move".to_owned(),
        [new_tab_reply("tab-scratch"), move_reply()]
            .into_iter()
            .collect(),
    );
    let missing_focus = run_balance(&herdr, "pane-c");
    assert!(
        missing_focus
            .assert_failure()
            .contains("without reporting its remaining focus")
    );
    assert!(
        missing_focus
            .requests
            .contains(&move_call("pane-c", attach_destination("pane-a", "right")))
    );
    assert_eq!(missing_focus.requests.last(), Some(&focus_call("pane-c")));

    for (reply, expected) in [
        (Err("pane disappeared".to_owned()), "pane disappeared"),
        (declined_swap_reply("not_found"), "not_found"),
    ] {
        herdr.replies.insert(
            "pane.move".to_owned(),
            [
                new_tab_reply_with_source(
                    "tab-scratch",
                    focused_layout(
                        "tab-main",
                        "pane-d",
                        vec![
                            pane("pane-a", 0, 0, 100, 100),
                            pane("pane-b", 100, 0, 100, 50),
                            pane("pane-d", 100, 50, 100, 50),
                        ],
                    ),
                ),
                move_reply(),
            ]
            .into_iter()
            .collect(),
        );
        herdr
            .replies
            .insert("pane.swap".to_owned(), [reply].into_iter().collect());

        let run = run_balance(&herdr, "pane-c");

        assert!(run.assert_failure().contains(expected));
        assert_eq!(run.requests.last(), Some(&focus_call("pane-c")));
        assert!(!run.requests.iter().any(|call| {
            call.method == "layout.set_split_ratio"
                || (call.method == "pane.move" && call.params["pane_id"] == "pane-d")
        }));
    }
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
fn manifest_exposes_the_create_tab_to_the_right_action() {
    let manifest = fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/herdr-plugin.toml"))
        .expect("the manifest sits next to Cargo.toml");

    assert!(
        manifest
            .lines()
            .any(|line| line.trim() == r#"id = "new-tab""#)
    );
}

#[test]
fn manifest_exposes_the_create_workspace_after_current_action() {
    let manifest = fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/herdr-plugin.toml"))
        .expect("the manifest sits next to Cargo.toml")
        .replace("\r\n", "\n");

    assert!(manifest.contains(
        r#"[[actions]]
id = "new-workspace"
title = "Create workspace after current"
contexts = ["workspace"]
command = ["./bin/herdr-grid-slide", "new-workspace"]"#
    ));
}

#[test]
fn manifest_exposes_all_directional_pane_move_actions() {
    let manifest = fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/herdr-plugin.toml"))
        .expect("the manifest sits next to Cargo.toml")
        .replace("\r\n", "\n");

    for direction in ["left", "down", "up", "right"] {
        assert!(
            manifest.contains(&format!(
                "id = \"move-{direction}\"\n\
                 title = \"Move pane {direction}\"\n\
                 contexts = [\"pane\"]\n\
                 command = [\"./bin/herdr-grid-slide\", \"move\", \"{direction}\"]"
            )),
            "missing move-{direction} action"
        );
    }
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

    assert_eq!(
        events,
        ["\"pane.focused\"", "\"pane.closed\"", "\"pane.exited\""]
    );

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
        let output = ProcessCommand::new(env!("CARGO_BIN_EXE_herdr-grid-slide"))
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
    program: PathBuf,
    /// Requests that arrived while a held balance lock was still in place.
    premature_requests: Arc<AtomicUsize>,
    lock_released: Arc<AtomicBool>,
    replies: HashMap<String, VecDeque<Reply>>,
    snapshot: Value,
    state_path: PathBuf,
}

/// The balance lock taken the way a running hook holds it. Dropping it lets the
/// plugin proceed and stops counting its requests as premature.
struct HeldBalanceLock {
    file: File,
    released: Arc<AtomicBool>,
}

impl Drop for HeldBalanceLock {
    fn drop(&mut self) {
        self.released.store(true, Ordering::SeqCst);
        self.file.unlock().unwrap();
    }
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
            program: PathBuf::from(env!("CARGO_BIN_EXE_herdr-grid-slide")),
            lock_released: Arc::new(AtomicBool::new(true)),
            premature_requests: Arc::new(AtomicUsize::new(0)),
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

    /// Navigation-only cases start with a destination already recorded by the hook.
    fn with_balanced_tab(self, tab_id: &str) -> Self {
        let layout = self.snapshot["layouts"]
            .as_array()
            .unwrap()
            .iter()
            .find(|layout| layout["tab_id"] == tab_id)
            .unwrap();
        let mut pane_ids: Vec<_> = layout["panes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|pane| pane["pane_id"].as_str().unwrap())
            .collect();
        pane_ids.sort_unstable();
        self.write_balance_state(tab_id, &pane_ids);
        self
    }

    fn run(&self, workspace_id: &str, tab_id: &str, pane_id: &str, args: &[&str]) -> Run {
        self.run_with_context(
            workspace_id,
            Some(tab_id),
            pane_id,
            Some(&self.state_path),
            args,
        )
    }

    fn run_without_state(
        &self,
        workspace_id: &str,
        tab_id: &str,
        pane_id: &str,
        args: &[&str],
    ) -> Run {
        self.run_with_context(workspace_id, Some(tab_id), pane_id, None, args)
    }

    fn run_without_tab(&self, workspace_id: &str, pane_id: &str, args: &[&str]) -> Run {
        self.run_with_context(workspace_id, None, pane_id, Some(&self.state_path), args)
    }

    fn run_with_context(
        &self,
        workspace_id: &str,
        tab_id: Option<&str>,
        pane_id: &str,
        state_path: Option<&Path>,
        args: &[&str],
    ) -> Run {
        let socket_path = self.directory.join("herdr.sock");
        let stop = Arc::new(AtomicBool::new(false));
        let server = self.serve(&socket_path, Arc::clone(&stop));
        let mut command = ProcessCommand::new(&self.program);
        command
            .args(args)
            .env("HERDR_PANE_ID", pane_id)
            .env("HERDR_SOCKET_PATH", &socket_path)
            .env("HERDR_WORKSPACE_ID", workspace_id);
        if let Some(state_path) = state_path {
            command.env("HERDR_PLUGIN_STATE_DIR", state_path);
        } else {
            command.env_remove("HERDR_PLUGIN_STATE_DIR");
        }
        if let Some(tab_id) = tab_id {
            command.env("HERDR_TAB_ID", tab_id);
        } else {
            command.env_remove("HERDR_TAB_ID");
        }
        let output = command.output().unwrap();
        stop.store(true, Ordering::Relaxed);
        let requests = server.join().unwrap();
        Run { output, requests }
    }

    /// Serves requests until `stop` is set, then hands back everything it saw.
    fn serve(&self, socket_path: &Path, stop: Arc<AtomicBool>) -> thread::JoinHandle<Vec<Call>> {
        #[cfg(unix)]
        let listener = {
            let _ = fs::remove_file(socket_path);
            let listener = UnixListener::bind(socket_path).unwrap();
            listener.set_nonblocking(true).unwrap();
            listener
        };
        #[cfg(windows)]
        let listener = ListenerOptions::new()
            .name(
                socket_path
                    .to_string_lossy()
                    .to_ns_name::<GenericNamespaced>()
                    .unwrap(),
            )
            .nonblocking(ListenerNonblockingMode::Accept)
            .create_sync()
            .unwrap();
        let mut replies = self.replies.clone();
        let snapshot = self.snapshot.clone();
        let lock_released = Arc::clone(&self.lock_released);
        let premature_requests = Arc::clone(&self.premature_requests);
        thread::spawn(move || {
            let mut requests = Vec::new();
            loop {
                #[cfg(unix)]
                let accepted = listener.accept().map(|(connection, _)| connection);
                #[cfg(windows)]
                let accepted = listener.accept();
                match accepted {
                    Ok(connection) => {
                        if !lock_released.load(Ordering::SeqCst) {
                            premature_requests.fetch_add(1, Ordering::SeqCst);
                        }
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

    fn hold_balance_lock(&self) -> HeldBalanceLock {
        fs::create_dir_all(&self.state_path).unwrap();
        let file = File::options()
            .create(true)
            .read(true)
            .truncate(false)
            .write(true)
            .open(self.state_path.join("balance-focus.lock"))
            .unwrap();
        file.lock().unwrap();
        self.lock_released.store(false, Ordering::SeqCst);
        HeldBalanceLock {
            file,
            released: Arc::clone(&self.lock_released),
        }
    }

    /// Runs the plugin once without arguments, which fails before any socket use.
    fn warm_up_binary(&self) {
        ProcessCommand::new(env!("CARGO_BIN_EXE_herdr-grid-slide"))
            .output()
            .unwrap();
    }

    fn premature_requests(&self) -> usize {
        self.premature_requests.load(Ordering::SeqCst)
    }

    fn write_state(&self, state: &str) {
        fs::create_dir_all(&self.state_path).unwrap();
        fs::write(self.state_path.join("focus-anchor.json"), state).unwrap();
    }

    /// Records the tab as entered the way it is laid out in the current snapshot.
    fn write_balance_state(&self, tab_id: &str, pane_ids: &[&str]) {
        self.write_balance_state_with_bounds(
            tab_id,
            pane_ids,
            snapshot_bounds(&self.snapshot, tab_id),
        );
    }

    fn write_balance_state_with_bounds(&self, tab_id: &str, pane_ids: &[&str], bounds: Value) {
        fs::create_dir_all(&self.state_path).unwrap();
        fs::write(
            self.state_path.join("balance-focus.json"),
            json!({"bounds": bounds, "paneIds": pane_ids, "tabId": tab_id}).to_string(),
        )
        .unwrap();
    }

    fn read_balance_state(&self) -> Value {
        let state = fs::read_to_string(self.state_path.join("balance-focus.json")).unwrap();
        serde_json::from_str(&state).unwrap()
    }

    fn read_anchor_state(&self) -> Value {
        let state = fs::read_to_string(self.state_path.join("focus-anchor.json")).unwrap();
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
    connection: LocalStream,
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
    anchor_positions("pane-a", "pane-b", "pane-c")
}

fn anchor_positions(left: &str, top_right: &str, bottom_right: &str) -> Value {
    snapshot(
        json!([layout(
            "tab-anchor",
            vec![
                pane(left, 0, 0, 60, 100),
                pane(top_right, 60, 0, 40, 50),
                pane(bottom_right, 60, 50, 40, 50),
            ],
        )]),
        json!([tab("workspace-1", "tab-anchor", 1)]),
        json!([workspace("workspace-1", "tab-anchor", 1)]),
    )
}

fn standard_snapshot() -> Value {
    snapshot(standard_layouts(), standard_tabs(), standard_workspaces())
}

fn tab_workspace_commands(target: &str) -> [(Option<&str>, &'static [&'static str]); 2] {
    [
        (None, &["tab-to-new-workspace"]),
        (Some(target), &["move-tab-workspace", "next"]),
    ]
}

fn tab_workspace_destination(target: Option<&str>, label: Option<&str>) -> Value {
    let mut destination = match target {
        Some(workspace_id) => json!({"type": "new_tab", "workspace_id": workspace_id}),
        None => json!({"type": "new_workspace"}),
    };
    if let Some(label) = label {
        destination[if target.is_some() {
            "label"
        } else {
            "tab_label"
        }] = label.into();
    }
    destination
}

fn tab_workspace_reply(target: Option<&str>) -> Reply {
    match target {
        Some(_) => new_tab_reply("tab-created"),
        None => new_workspace_reply("workspace-created"),
    }
}

// Only a newly created workspace is repositioned after the source.
fn tab_workspace_calls(target: Option<&str>, calls: Vec<Call>) -> Vec<Call> {
    calls
        .into_iter()
        .filter(|call| target.is_none() || call.method != "workspace.move")
        .collect()
}

fn remove_tab(snapshot: &mut Value, tab_id: &str) {
    for field in ["tabs", "layouts"] {
        snapshot[field]
            .as_array_mut()
            .unwrap()
            .retain(|entry| entry["tab_id"] != tab_id);
    }
}

/// An existing destination uses a source workspace with only the moving tab.
fn tab_workspace_before_snapshot(target: Option<&str>) -> Value {
    let mut before = tab_to_new_workspace_before_snapshot();
    if target.is_some() {
        remove_tab(&mut before, "tab-other");
    }
    before
}

/// A two-pane tab beside another tab, so moving the whole tab leaves its workspace
/// open. Workspace array order differs from visible `number` order, so the source's
/// visible insertion slot is 2 rather than its array slot 1.
fn tab_to_new_workspace_before_snapshot() -> Value {
    focused(
        snapshot(
            json!([
                layout(
                    "tab-source",
                    vec![
                        pane("pane-live", 0, 0, 50, 40),
                        pane("pane-stays", 50, 0, 50, 40),
                    ],
                ),
                layout("tab-other", vec![pane("pane-other", 0, 0, 100, 40)]),
                layout("tab-first", vec![pane("pane-first", 0, 0, 100, 40)]),
                layout("tab-last", vec![pane("pane-last", 0, 0, 100, 40)]),
            ]),
            json!([
                tab("workspace-source", "tab-source", 1),
                tab("workspace-source", "tab-other", 2),
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
    )
}

/// Both panes reached the destination, before balancing its uneven split.
/// An existing destination leaves the source workspace removed.
fn tab_workspace_after_snapshot(existing: Option<&str>) -> Value {
    let mut after = focused(
        snapshot(
            json!([
                layout(
                    "tab-created",
                    vec![
                        pane("pane-live", 0, 0, 70, 40),
                        pane("pane-stays", 70, 0, 30, 40),
                    ],
                ),
                layout("tab-other", vec![pane("pane-other", 0, 0, 100, 40)]),
                layout("tab-first", vec![pane("pane-first", 0, 0, 100, 40)]),
                layout("tab-last", vec![pane("pane-last", 0, 0, 100, 40)]),
            ]),
            json!([
                tab("workspace-source", "tab-other", 2),
                tab("workspace-first", "tab-first", 1),
                tab("workspace-last", "tab-last", 1),
                tab("workspace-created", "tab-created", 1),
            ]),
            json!([
                workspace("workspace-source", "tab-other", 2),
                workspace("workspace-first", "tab-first", 1),
                workspace("workspace-created", "tab-created", 3),
                workspace("workspace-last", "tab-last", 4),
            ]),
        ),
        "workspace-created",
        "tab-created",
        "pane-live",
    );
    if let Some(workspace_id) = existing {
        after["focused_workspace_id"] = workspace_id.into();
        after["tabs"][3]["workspace_id"] = workspace_id.into();
        after["workspaces"].as_array_mut().unwrap().remove(2);
        after["workspaces"][2]["number"] = 3.into();
        for workspace in after["workspaces"].as_array_mut().unwrap() {
            if workspace["workspace_id"] == workspace_id {
                workspace["active_tab_id"] = "tab-created".into();
            }
        }
        remove_tab(&mut after, "tab-other");
        after["workspaces"]
            .as_array_mut()
            .unwrap()
            .retain(|workspace| workspace["workspace_id"] != "workspace-source");
        for workspace in after["workspaces"].as_array_mut().unwrap() {
            if workspace["number"].as_u64().unwrap() > 2 {
                workspace["number"] = (workspace["number"].as_u64().unwrap() - 1).into();
            }
        }
    }
    after
}

/// A single-pane tab with a sibling tab, which moves as a whole with one request.
fn single_pane_tab_snapshot() -> Value {
    focused(
        snapshot(
            json!([
                layout("tab-source", vec![pane("pane-moving", 0, 0, 100, 100)]),
                layout("tab-stays", vec![pane("pane-stays", 0, 0, 100, 100)]),
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
    )
}

fn right_boundary_herdr(swap: Reply) -> FakeHerdr {
    let mut layouts = standard_layouts();
    let current = layouts
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|layout| layout["tab_id"] == "tab-2-3")
        .unwrap();
    *current = layout("tab-2-3", vec![pane("pane-current", 0, 0, 100, 80)]);
    FakeHerdr::new(snapshot(layouts, standard_tabs(), standard_workspaces()))
        .with_replies(
            "pane.move",
            [successful_move_reply(
                ("pane-current", "workspace-2", "tab-2-3"),
                ("pane-new", "workspace-2", "tab-2-1"),
                (
                    None,
                    layout(
                        "tab-2-1",
                        vec![
                            pane("pane-2-1-top-left", 0, 0, 50, 80),
                            pane("pane-new", 50, 0, 50, 80),
                        ],
                    ),
                ),
            )],
        )
        .with_replies("pane.swap", [swap])
        .with_balanced_tab("tab-2-1")
}

fn down_boundary_herdr(swap: Reply) -> FakeHerdr {
    let mut layouts = standard_layouts();
    for (tab_id, pane_id) in [("tab-3-1", "pane-current"), ("tab-1-1", "pane-down-target")] {
        let current = layouts
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|layout| layout["tab_id"] == tab_id)
            .unwrap();
        *current = layout(tab_id, vec![pane(pane_id, 0, 0, 100, 80)]);
    }
    let herdr = FakeHerdr::new(snapshot(layouts, standard_tabs(), standard_workspaces()))
        .with_replies(
            "pane.move",
            [successful_move_reply(
                ("pane-current", "workspace-3", "tab-3-1"),
                ("pane-new", "workspace-1", "tab-1-1"),
                (
                    None,
                    layout(
                        "tab-1-1",
                        vec![
                            pane("pane-down-target", 0, 0, 50, 80),
                            pane("pane-new", 50, 0, 50, 80),
                        ],
                    ),
                ),
            )],
        )
        .with_replies("pane.swap", [swap]);
    herdr.write_state(
        r#"{"paneId":"pane-current","tabId":"tab-3-1","workspaceId":"workspace-3","x":0.25,"y":0.5}"#,
    );
    herdr
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

/// Every `new-pane` scenario runs the same command against the same tab, so only
/// the tree and the focused pane change between them.
fn run_new_pane(herdr: &FakeHerdr, pane_id: &str) -> Run {
    herdr.run("workspace-1", "tab-main", pane_id, &["new-pane"])
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
    Ok(json!({
        "type": "pane_swap",
        "swap": {"changed": true, "reason": null},
    }))
}

fn declined_swap_reply(reason: &str) -> Reply {
    Ok(json!({
        "type": "pane_swap",
        "swap": {"changed": false, "reason": reason},
    }))
}

fn tab_move_reply() -> Reply {
    Ok(json!({"type": "tab_list", "tabs": []}))
}

fn tab_created_reply(tab_id: &str, pane_id: &str) -> Reply {
    Ok(json!({
        "type": "tab_created",
        "tab": {"tab_id": tab_id},
        "root_pane": {"pane_id": pane_id},
    }))
}

fn tab_created_reply_without_tab_id(pane_id: &str) -> Reply {
    Ok(json!({
        "type": "tab_created",
        "tab": {},
        "root_pane": {"pane_id": pane_id},
    }))
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

/// A pane that followed its tab into the created workspace, where herdr gave it a
/// new ID.
fn followed_reply(pane_id: &str, moved_pane_id: &str) -> Reply {
    successful_move_reply(
        (pane_id, "workspace-source", "tab-source"),
        (moved_pane_id, "workspace-created", "tab-created"),
        (
            None,
            layout("tab-created", vec![pane(moved_pane_id, 0, 0, 100, 100)]),
        ),
    )
}

fn new_tab_reply(tab_id: &str) -> Reply {
    new_tab_reply_with_source(tab_id, Value::Null)
}

fn new_workspace_reply(workspace_id: &str) -> Reply {
    Ok(json!({
        "type": "pane_move",
        "move_result": {
            "changed": true,
            "created_tab": {"tab_id": "tab-created", "workspace_id": workspace_id},
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

fn workspace_created_reply(workspace_id: &str) -> Reply {
    Ok(json!({
        "type": "workspace_created",
        "workspace": {"workspace_id": workspace_id},
        "tab": {"tab_id": "tab-created"},
        "root_pane": {"pane_id": "pane-created"},
    }))
}

fn workspace_created_reply_without_workspace_id() -> Reply {
    Ok(json!({
        "type": "workspace_created",
        "workspace": {},
        "tab": {"tab_id": "tab-created"},
        "root_pane": {"pane_id": "pane-created"},
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
    tab_destination("tab-scratch")
}

fn tab_destination(tab_id: &str) -> Value {
    json!({"type": "tab", "tab_id": tab_id, "split": "right", "ratio": 0.5})
}

/// Where a following pane lands: right of the pane herdr reported for the move
/// before it, so a tab crossing workspaces keeps its reading order.
fn join_destination(tab_id: &str, target_pane_id: &str) -> Value {
    json!({
        "type": "tab",
        "tab_id": tab_id,
        "target_pane_id": target_pane_id,
        "split": "right",
        "ratio": 0.5,
    })
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

/// Tabs without a label get the one herdr shows by default: their one-based
/// position among the tabs of their workspace, in listing order, which is not the
/// stable `number`.
fn snapshot(layouts: Value, mut tabs: Value, workspaces: Value) -> Value {
    let mut positions: HashMap<String, usize> = HashMap::new();
    for tab in tabs.as_array_mut().expect("tab list") {
        let workspace_id = tab["workspace_id"]
            .as_str()
            .expect("tab workspace")
            .to_owned();
        let position = positions.entry(workspace_id).or_default();
        *position += 1;
        if tab.get("label").is_none() {
            tab["label"] = position.to_string().into();
        }
    }
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

/// The bounds the plugin records for a tab: the extent of its pane rects, or
/// null for a tab the snapshot does not lay out.
fn snapshot_bounds(snapshot: &Value, tab_id: &str) -> Value {
    let rects: Vec<&Value> = snapshot["layouts"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|layout| layout["tab_id"] == tab_id)
        .flat_map(|layout| layout["panes"].as_array().into_iter().flatten())
        .map(|pane| &pane["rect"])
        .collect();
    if rects.is_empty() {
        return Value::Null;
    }
    let field = |rect: &Value, name: &str| rect[name].as_f64().unwrap();
    let min_x = rects
        .iter()
        .map(|rect| field(rect, "x"))
        .fold(f64::MAX, f64::min);
    let min_y = rects
        .iter()
        .map(|rect| field(rect, "y"))
        .fold(f64::MAX, f64::min);
    let max_x = rects
        .iter()
        .map(|rect| field(rect, "x") + field(rect, "width"))
        .fold(f64::MIN, f64::max);
    let max_y = rects
        .iter()
        .map(|rect| field(rect, "y") + field(rect, "height"))
        .fold(f64::MIN, f64::max);
    bounds(min_x, min_y, max_x - min_x, max_y - min_y)
}

fn bounds(x: f64, y: f64, width: f64, height: f64) -> Value {
    json!({"height": height, "width": width, "x": x, "y": y})
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

fn create_workspace_call() -> Call {
    call("workspace.create", json!({"focus": true}))
}

fn tab_move_call(tab_id: &str, insert_index: usize) -> Call {
    call(
        "tab.move",
        json!({"tab_id": tab_id, "insert_index": insert_index}),
    )
}

fn create_tab_call(workspace_id: &str) -> Call {
    call(
        "tab.create",
        json!({"workspace_id": workspace_id, "focus": true}),
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
    move_to_new_workspace_call_with(pane_id, json!({"type": "new_workspace"}))
}

fn move_to_labeled_new_workspace_call(pane_id: &str, tab_label: &str) -> Call {
    move_to_new_workspace_call_with(
        pane_id,
        json!({"type": "new_workspace", "tab_label": tab_label}),
    )
}

fn move_to_new_workspace_call_with(pane_id: &str, destination: Value) -> Call {
    call(
        "pane.move",
        json!({"pane_id": pane_id, "destination": destination, "focus": true}),
    )
}

fn move_to_tab(pane_id: &str, tab_id: &str) -> Value {
    json!({
        "pane_id": pane_id,
        "destination": {"type": "tab", "tab_id": tab_id, "split": "right", "ratio": 0.5},
        "focus": true,
    })
}

fn directional_move_call(
    pane_id: &str,
    tab_id: &str,
    target_pane_id: &str,
    split: &str,
    focus: bool,
) -> Call {
    call(
        "pane.move",
        json!({
            "pane_id": pane_id,
            "destination": {
                "type": "tab",
                "tab_id": tab_id,
                "target_pane_id": target_pane_id,
                "split": split,
                "ratio": 0.5,
            },
            "focus": focus,
        }),
    )
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
                .is_some_and(|program| program.ends_with("/herdr-grid-slide"))
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
