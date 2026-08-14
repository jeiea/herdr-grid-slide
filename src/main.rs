use std::cmp::Ordering;
use std::env;
use std::fs;
use std::io::{BufRead, BufReader, Write};
#[cfg(unix)]
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

const FOCUS_ANCHOR_STATE_FILE: &str = "focus-anchor.json";
type Result<T> = std::result::Result<T, String>;

#[derive(Clone, Copy)]
enum Scope {
    Workspace,
    Tab,
}

#[derive(Clone, Copy)]
enum MoveDirection {
    Next,
    Previous,
}

#[derive(Clone, Copy)]
enum PaneDirection {
    Left,
    Right,
    Up,
    Down,
}

#[derive(Clone, Copy, PartialEq, Deserialize)]
#[serde(rename_all = "snake_case")]
enum SplitDirection {
    Right,
    Down,
}

enum Command {
    Move {
        direction: MoveDirection,
        scope: Scope,
    },
    Focus(PaneDirection),
    NewPane,
}

struct Context {
    pane_id: String,
    socket_path: String,
    state_dir: Option<PathBuf>,
    tab_id: String,
    workspace_id: String,
}

struct NavigationContext<'a> {
    pane_id: &'a str,
    tab_id: &'a str,
    workspace_id: &'a str,
}

#[derive(Deserialize)]
struct SnapshotResult {
    snapshot: Snapshot,
}

#[derive(Deserialize)]
struct Snapshot {
    focused_pane_id: Option<String>,
    focused_tab_id: Option<String>,
    focused_workspace_id: Option<String>,
    layouts: Vec<TabLayout>,
    tabs: Vec<Tab>,
    workspaces: Vec<Workspace>,
}

#[derive(Deserialize)]
struct Workspace {
    active_tab_id: String,
    workspace_id: String,
    number: f64,
}

#[derive(Deserialize)]
struct Tab {
    tab_id: String,
    number: f64,
    workspace_id: String,
}

#[derive(Deserialize)]
struct TabLayout {
    panes: Vec<LayoutPane>,
    tab_id: String,
}

#[derive(Clone, Deserialize)]
struct LayoutPane {
    pane_id: String,
    rect: Rect,
}

#[derive(Deserialize)]
struct LayoutExportResult {
    layout: ExportedLayout,
}

#[derive(Deserialize)]
struct ExportedLayout {
    root: LayoutNode,
    zoomed: bool,
}

/// The split tree herdr keeps for a tab. A split always has two children, so a
/// path down the tree is a sequence of `false` (first) and `true` (second).
#[derive(Deserialize)]
#[serde(rename_all = "snake_case", tag = "type")]
enum LayoutNode {
    Pane {
        pane_id: String,
    },
    Split {
        direction: SplitDirection,
        first: Box<LayoutNode>,
        second: Box<LayoutNode>,
    },
}

#[derive(Deserialize)]
struct PaneInfoResult {
    pane: PaneRef,
}

#[derive(Deserialize)]
struct PaneRef {
    pane_id: String,
}

impl LayoutNode {
    fn direction(&self) -> Option<SplitDirection> {
        match self {
            LayoutNode::Split { direction, .. } => Some(*direction),
            LayoutNode::Pane { .. } => None,
        }
    }
}

impl SplitDirection {
    fn as_str(self) -> &'static str {
        match self {
            SplitDirection::Right => "right",
            SplitDirection::Down => "down",
        }
    }
}

#[derive(Clone, Copy, Deserialize)]
struct Rect {
    height: f64,
    width: f64,
    x: f64,
    y: f64,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FocusAnchorState {
    pane_id: String,
    tab_id: String,
    workspace_id: String,
    x: f64,
    y: f64,
}

#[derive(Clone, Copy)]
struct Point {
    x: f64,
    y: f64,
}

#[derive(Clone, Copy)]
struct Bounds {
    height: f64,
    width: f64,
    x: f64,
    y: f64,
}

struct FocusTarget<'a> {
    layout: &'a TabLayout,
    pane: &'a LayoutPane,
}

fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let command = parse_arguments(env::args().skip(1))?;
    let context = read_context(matches!(command, Command::Focus(_)))?;
    let mut client = SocketClient::new(&context.socket_path);
    match command {
        Command::Focus(direction) => focus_with_anchor(&context, direction, &mut client),
        Command::Move { direction, scope } => move_pane(&context, direction, scope, &mut client),
        Command::NewPane => new_pane(&context, &mut client),
    }
}

fn parse_arguments(args: impl Iterator<Item = String>) -> Result<Command> {
    let args: Vec<_> = args.collect();
    match args.as_slice() {
        [operation] if operation == "new-pane" => Ok(Command::NewPane),
        [operation, direction] if operation == "focus" => {
            Ok(Command::Focus(match direction.as_str() {
                "left" => PaneDirection::Left,
                "right" => PaneDirection::Right,
                "up" => PaneDirection::Up,
                "down" => PaneDirection::Down,
                _ => return Err(usage()),
            }))
        }
        [scope, direction] => {
            let scope = match scope.as_str() {
                "workspace" => Scope::Workspace,
                "tab" => Scope::Tab,
                _ => return Err(usage()),
            };
            let direction = match direction.as_str() {
                "next" => MoveDirection::Next,
                "previous" => MoveDirection::Previous,
                _ => return Err(usage()),
            };
            Ok(Command::Move { direction, scope })
        }
        _ => Err(usage()),
    }
}

fn usage() -> String {
    "usage: herdr-move-pane <workspace|tab> <next|previous> | focus <direction> | new-pane".into()
}

fn read_context(require_focus: bool) -> Result<Context> {
    Ok(Context {
        pane_id: required_env("HERDR_PANE_ID")?,
        socket_path: required_env("HERDR_SOCKET_PATH")?,
        state_dir: require_focus
            .then(|| required_env("HERDR_PLUGIN_STATE_DIR").map(PathBuf::from))
            .transpose()?,
        tab_id: required_env("HERDR_TAB_ID")?,
        workspace_id: required_env("HERDR_WORKSPACE_ID")?,
    })
}

fn move_pane(
    context: &Context,
    direction: MoveDirection,
    scope: Scope,
    client: &mut SocketClient,
) -> Result<()> {
    let snapshot = read_snapshot(client)?;
    let navigation = navigation_context(context, &snapshot);
    let target_tab_id = resolve_move_target_tab(&navigation, direction, scope, &snapshot)?;
    if target_tab_id == navigation.tab_id {
        return Ok(());
    }
    client.request(
        "pane.move",
        json!({
            "pane_id": navigation.pane_id,
            "destination": {
                "type": "tab",
                "tab_id": target_tab_id,
                "split": "right",
                "ratio": 0.5,
            },
            "focus": true,
        }),
    )?;
    Ok(())
}

fn focus_with_anchor(
    context: &Context,
    direction: PaneDirection,
    client: &mut SocketClient,
) -> Result<()> {
    let state_dir = context
        .state_dir
        .as_ref()
        .ok_or("missing HERDR_PLUGIN_STATE_DIR")?;
    fs::create_dir_all(state_dir).map_err(|error| error.to_string())?;
    let snapshot = read_snapshot(client)?;
    let navigation = navigation_context(context, &snapshot);
    let state = read_focus_anchor_state(state_dir)?;
    let (target, next_state) =
        resolve_focus_target(&navigation, direction, &snapshot, state.as_ref())?;
    client.request("pane.focus", json!({"pane_id": target.pane_id}))?;
    write_focus_anchor_state(state_dir, &next_state)
}

/// Splits the focused pane along the direction the tab already grows in, then gives
/// every pane of that row (or column) an equal share, which herdr's own 50:50 split
/// does not do once a row holds more than two panes.
fn new_pane(context: &Context, client: &mut SocketClient) -> Result<()> {
    let snapshot = read_snapshot(client)?;
    let navigation = navigation_context(context, &snapshot);
    let tab_id = navigation.tab_id;
    let snapshot_layout = find_layout(&snapshot, tab_id)?;
    let pane = find_pane(snapshot_layout, navigation.pane_id)?;
    let layout = read_layout(client, tab_id)?;
    if !holds_the_same_panes(snapshot_layout, &layout.root) {
        return Err(format!(
            "{tab_id} changed while it was being read; try again"
        ));
    }
    if layout.zoomed {
        // Unzooming here would leave the tab zoomed out if a later step failed.
        return Err(format!(
            "{tab_id} is zoomed; unzoom it before adding a pane"
        ));
    }
    let created = split_pane(
        client,
        navigation.pane_id,
        split_direction(&layout.root, pane),
    )?;
    // Every failure past this point has to say the pane already exists, so that a
    // reported failure is never mistaken for "nothing happened".
    let layout = read_layout(client, tab_id)
        .map_err(|error| format!("created {created} but could not read {tab_id} back: {error}"))?;
    let descent = descent_to_pane(&layout.root, &created).ok_or_else(|| {
        format!("created {created} but it is missing from {tab_id}; left the sizes alone")
    })?;
    for (path, ratio) in even_ratios(&descent) {
        client
            .request(
                "layout.set_split_ratio",
                json!({"tab_id": tab_id, "path": path, "ratio": ratio}),
            )
            .map_err(|error| {
                format!("created {created} but could not even out the sizes: {error}")
            })?;
    }
    Ok(())
}

fn read_snapshot(client: &mut SocketClient) -> Result<Snapshot> {
    let result = client.request("session.snapshot", json!({}))?;
    serde_json::from_value::<SnapshotResult>(result)
        .map(|response| response.snapshot)
        .map_err(|_| "herdr api snapshot returned an invalid response".into())
}

fn read_layout(client: &mut SocketClient, tab_id: &str) -> Result<ExportedLayout> {
    let result = client.request("layout.export", json!({"tab_id": tab_id}))?;
    serde_json::from_value::<LayoutExportResult>(result)
        .map(|response| response.layout)
        .map_err(|_| "herdr api layout returned an invalid response".into())
}

fn split_pane(
    client: &mut SocketClient,
    pane_id: &str,
    direction: SplitDirection,
) -> Result<String> {
    let result = client.request(
        "pane.split",
        json!({
            "target_pane_id": pane_id,
            "direction": direction.as_str(),
            "ratio": 0.5,
            "focus": true,
        }),
    )?;
    serde_json::from_value::<PaneInfoResult>(result)
        .map(|response| response.pane.pane_id)
        // herdr answered the split, so it likely went through even though the reply
        // is unreadable and the new pane cannot be named.
        .map_err(|_| {
            "herdr api pane.split returned an invalid response; a pane may have been created".into()
        })
}

fn navigation_context<'a>(context: &'a Context, snapshot: &'a Snapshot) -> NavigationContext<'a> {
    match (
        &snapshot.focused_workspace_id,
        &snapshot.focused_tab_id,
        &snapshot.focused_pane_id,
    ) {
        (Some(workspace_id), Some(tab_id), Some(pane_id)) => NavigationContext {
            pane_id,
            tab_id,
            workspace_id,
        },
        _ => NavigationContext {
            pane_id: &context.pane_id,
            tab_id: &context.tab_id,
            workspace_id: &context.workspace_id,
        },
    }
}

fn resolve_move_target_tab<'a>(
    context: &NavigationContext,
    direction: MoveDirection,
    scope: Scope,
    snapshot: &'a Snapshot,
) -> Result<&'a str> {
    match scope {
        Scope::Workspace => {
            let mut items: Vec<_> = snapshot.workspaces.iter().collect();
            items.sort_by(|left, right| left.number.total_cmp(&right.number));
            adjacent(&items, context.workspace_id, direction, |item| {
                item.workspace_id.as_str()
            })
            .map(|item| item.active_tab_id.as_str())
        }
        Scope::Tab => {
            let mut items: Vec<_> = snapshot
                .tabs
                .iter()
                .filter(|tab| tab.workspace_id == context.workspace_id)
                .collect();
            items.sort_by(|left, right| left.number.total_cmp(&right.number));
            adjacent(&items, context.tab_id, direction, |item| {
                item.tab_id.as_str()
            })
            .map(|item| item.tab_id.as_str())
        }
    }
}

fn adjacent<'a, T>(
    items: &[&'a T],
    current_id: &str,
    direction: MoveDirection,
    get_id: impl Fn(&T) -> &str,
) -> Result<&'a T> {
    let current = items
        .iter()
        .position(|item| get_id(item) == current_id)
        .ok_or_else(|| format!("current item not found in herdr snapshot: {current_id}"))?;
    let target = match direction {
        MoveDirection::Next => (current + 1) % items.len(),
        MoveDirection::Previous => (current + items.len() - 1) % items.len(),
    };
    Ok(items[target])
}

fn resolve_focus_target<'a>(
    context: &NavigationContext,
    direction: PaneDirection,
    snapshot: &'a Snapshot,
    state: Option<&FocusAnchorState>,
) -> Result<(&'a LayoutPane, FocusAnchorState)> {
    let current_layout = find_layout(snapshot, context.tab_id)?;
    let current_pane = find_pane(current_layout, context.pane_id)?;
    let current_bounds = layout_bounds(current_layout)?;
    let source_point = resolve_anchor_point(current_bounds, context, current_pane, state);
    let source_ratio = normalize_point(current_bounds, source_point);
    let within_target =
        resolve_within_layout(current_layout, current_pane, direction, source_point);
    let target = match within_target {
        Some(pane) => FocusTarget {
            layout: current_layout,
            pane,
        },
        None => resolve_boundary_target(context, direction, source_ratio, snapshot)?,
    };
    let target_bounds = layout_bounds(target.layout)?;
    let target_point = if within_target.is_none() {
        denormalize_point(target_bounds, source_ratio)
    } else {
        source_point
    };
    let normalized = normalize_point(
        target_bounds,
        move_anchor_point(direction, target_point, target.pane),
    );
    Ok((
        target.pane,
        FocusAnchorState {
            pane_id: target.pane.pane_id.clone(),
            tab_id: target.layout.tab_id.clone(),
            workspace_id: workspace_id_for_tab(snapshot, &target.layout.tab_id)?.into(),
            x: normalized.x,
            y: normalized.y,
        },
    ))
}

fn resolve_anchor_point(
    bounds: Bounds,
    context: &NavigationContext,
    pane: &LayoutPane,
    state: Option<&FocusAnchorState>,
) -> Point {
    if let Some(state) = state
        && state.pane_id == context.pane_id
        && state.tab_id == context.tab_id
        && state.workspace_id == context.workspace_id
    {
        return clamp_point_to_pane(
            denormalize_point(
                bounds,
                Point {
                    x: state.x,
                    y: state.y,
                },
            ),
            pane,
        );
    }
    pane_center(pane)
}

fn resolve_within_layout<'a>(
    layout: &'a TabLayout,
    pane: &LayoutPane,
    direction: PaneDirection,
    point: Point,
) -> Option<&'a LayoutPane> {
    layout
        .panes
        .iter()
        .filter(|candidate| candidate.pane_id != pane.pane_id)
        .filter(|candidate| is_pane_in_direction(candidate, pane, direction))
        .filter(|candidate| cross_axis_overlaps(candidate, pane, direction))
        .min_by(|left, right| compare_candidates(left, right, pane, direction, point))
}

fn compare_candidates(
    left: &LayoutPane,
    right: &LayoutPane,
    pane: &LayoutPane,
    direction: PaneDirection,
    point: Point,
) -> Ordering {
    primary_distance(left, pane, direction)
        .total_cmp(&primary_distance(right, pane, direction))
        .then_with(|| {
            axis_distance(left, direction, point).total_cmp(&axis_distance(right, direction, point))
        })
        .then_with(|| {
            axis_center_distance(left, direction, point)
                .total_cmp(&axis_center_distance(right, direction, point))
        })
        .then_with(|| compare_reading_order(left, right))
}

fn resolve_boundary_target<'a>(
    context: &NavigationContext,
    direction: PaneDirection,
    point_ratio: Point,
    snapshot: &'a Snapshot,
) -> Result<FocusTarget<'a>> {
    let target_tab_id = resolve_boundary_tab(context, direction, snapshot)?;
    let layout = find_layout(snapshot, target_tab_id)?;
    let point = denormalize_point(layout_bounds(layout)?, point_ratio);
    let pane = entry_edge_panes(layout, direction)
        .into_iter()
        .min_by(|left, right| {
            axis_distance(left, direction, point)
                .total_cmp(&axis_distance(right, direction, point))
                .then_with(|| {
                    axis_center_distance(left, direction, point)
                        .total_cmp(&axis_center_distance(right, direction, point))
                })
                .then_with(|| compare_boundary_reading_order(left, right, direction))
        })
        .ok_or_else(|| format!("target tab contains no pane in herdr layout: {target_tab_id}"))?;
    Ok(FocusTarget { layout, pane })
}

fn resolve_boundary_tab<'a>(
    context: &NavigationContext,
    direction: PaneDirection,
    snapshot: &'a Snapshot,
) -> Result<&'a str> {
    if matches!(direction, PaneDirection::Left | PaneDirection::Right) {
        let mut items: Vec<_> = snapshot
            .tabs
            .iter()
            .filter(|tab| tab.workspace_id == context.workspace_id)
            .collect();
        items.sort_by(|left, right| left.number.total_cmp(&right.number));
        let movement = if matches!(direction, PaneDirection::Right) {
            MoveDirection::Next
        } else {
            MoveDirection::Previous
        };
        return adjacent(&items, context.tab_id, movement, |item| {
            item.tab_id.as_str()
        })
        .map(|item| item.tab_id.as_str());
    }
    let mut items: Vec<_> = snapshot.workspaces.iter().collect();
    items.sort_by(|left, right| left.number.total_cmp(&right.number));
    let movement = if matches!(direction, PaneDirection::Down) {
        MoveDirection::Next
    } else {
        MoveDirection::Previous
    };
    adjacent(&items, context.workspace_id, movement, |item| {
        item.workspace_id.as_str()
    })
    .map(|item| item.active_tab_id.as_str())
}

fn read_focus_anchor_state(state_dir: &Path) -> Result<Option<FocusAnchorState>> {
    let bytes = match fs::read(state_dir.join(FOCUS_ANCHOR_STATE_FILE)) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.to_string()),
    };
    match serde_json::from_slice::<FocusAnchorState>(&bytes) {
        Ok(state) if is_unit_interval(state.x) && is_unit_interval(state.y) => Ok(Some(state)),
        _ => Ok(None),
    }
}

fn write_focus_anchor_state(state_dir: &Path, state: &FocusAnchorState) -> Result<()> {
    let path = state_dir.join(FOCUS_ANCHOR_STATE_FILE);
    let temporary = state_dir.join(format!(
        "{FOCUS_ANCHOR_STATE_FILE}.{}-{}.tmp",
        std::process::id(),
        timestamp()?
    ));
    let bytes = serde_json::to_vec(state).map_err(|error| error.to_string())?;
    let result = fs::write(&temporary, bytes).and_then(|()| fs::rename(&temporary, &path));
    let _ = fs::remove_file(&temporary);
    result.map_err(|error| error.to_string())
}

struct SocketClient {
    next_id: u64,
    socket_path: String,
}

impl SocketClient {
    fn new(socket_path: &str) -> Self {
        Self {
            next_id: 1,
            socket_path: socket_path.to_owned(),
        }
    }

    fn request(&mut self, method: &str, params: Value) -> Result<Value> {
        let id = format!("jeiea.move-pane:{}-{}", std::process::id(), self.next_id);
        self.next_id += 1;
        let mut writer =
            UnixStream::connect(&self.socket_path).map_err(|error| error.to_string())?;
        let mut reader = BufReader::new(writer.try_clone().map_err(|error| error.to_string())?);
        writeln!(
            writer,
            "{}",
            json!({"id": id, "method": method, "params": params})
        )
        .map_err(|error| error.to_string())?;

        let mut response = String::new();
        reader
            .read_line(&mut response)
            .map_err(|error| error.to_string())?;
        let response: Value = serde_json::from_str(&response)
            .map_err(|_| format!("{method} socket response returned invalid JSON"))?;
        if response.get("id") != Some(&Value::String(id)) {
            return Err(format!("{method} socket response has an unexpected id"));
        }
        if let Some(error) = response.get("error") {
            let detail = error
                .get("message")
                .and_then(Value::as_str)
                .map(|message| format!(": {message}"))
                .unwrap_or_default();
            return Err(format!("{method} failed{detail}"));
        }
        response
            .get("result")
            .cloned()
            .ok_or_else(|| format!("{method} socket response is missing result"))
    }
}

fn find_layout<'a>(snapshot: &'a Snapshot, tab_id: &str) -> Result<&'a TabLayout> {
    snapshot
        .layouts
        .iter()
        .find(|layout| layout.tab_id == tab_id)
        .ok_or_else(|| format!("target tab has no layout: {tab_id}"))
}

fn find_pane<'a>(layout: &'a TabLayout, pane_id: &str) -> Result<&'a LayoutPane> {
    layout
        .panes
        .iter()
        .find(|pane| pane.pane_id == pane_id)
        .ok_or_else(|| format!("current pane not found in herdr layout: {pane_id}"))
}

/// A snapshot and a layout read one after another can straddle a concurrent change,
/// which would make every path computed from the tree point at the wrong pane.
fn holds_the_same_panes(layout: &TabLayout, root: &LayoutNode) -> bool {
    let mut snapshot_ids: Vec<_> = layout
        .panes
        .iter()
        .map(|pane| pane.pane_id.as_str())
        .collect();
    let mut layout_ids = leaf_pane_ids(root);
    snapshot_ids.sort_unstable();
    layout_ids.sort_unstable();
    snapshot_ids == layout_ids
}

fn split_direction(root: &LayoutNode, pane: &LayoutPane) -> SplitDirection {
    if let Some(direction) = uniform_direction(root) {
        return direction;
    }
    // Terminal cells are roughly twice as tall as they are wide, so a pane only
    // looks wide once its width passes twice its height.
    if pane.rect.width > pane.rect.height * 2.0 {
        SplitDirection::Right
    } else {
        SplitDirection::Down
    }
}

/// The direction every split of the tab shares, if the tab grows only one way.
fn uniform_direction(root: &LayoutNode) -> Option<SplitDirection> {
    let directions = split_directions(root);
    let first = *directions.first()?;
    directions
        .iter()
        .all(|direction| *direction == first)
        .then_some(first)
}

fn split_directions(node: &LayoutNode) -> Vec<SplitDirection> {
    match node {
        LayoutNode::Pane { .. } => Vec::new(),
        LayoutNode::Split {
            direction,
            first,
            second,
        } => [*direction]
            .into_iter()
            .chain(split_directions(first))
            .chain(split_directions(second))
            .collect(),
    }
}

/// Ratios that give every slot of the new pane's run an equal share, closest to the
/// root first. The run is the largest group of same-direction splits joining the new
/// pane's parent; a slot is a child that leaves the run, so a crosswise subtree
/// counts as one slot however many panes it holds.
fn even_ratios(descent: &[(&LayoutNode, bool)]) -> Vec<(Vec<bool>, f64)> {
    let Some(direction) = descent.last().and_then(|(parent, _)| parent.direction()) else {
        return Vec::new();
    };
    let joined = descent
        .iter()
        .rev()
        .take_while(|(node, _)| node.direction() == Some(direction))
        .count();
    let run_root = descent.len() - joined;
    let path = descent[..run_root].iter().map(|(_, branch)| *branch);
    ratio_updates(descent[run_root].0, direction, path.collect())
}

fn ratio_updates(
    node: &LayoutNode,
    direction: SplitDirection,
    path: Vec<bool>,
) -> Vec<(Vec<bool>, f64)> {
    let LayoutNode::Split {
        direction: node_direction,
        first,
        second,
    } = node
    else {
        return Vec::new();
    };
    if *node_direction != direction {
        return Vec::new();
    }
    let ratio = slot_count(first, direction) as f64 / slot_count(node, direction) as f64;
    [(path.clone(), ratio)]
        .into_iter()
        .chain(ratio_updates(first, direction, child_path(&path, false)))
        .chain(ratio_updates(second, direction, child_path(&path, true)))
        .collect()
}

fn slot_count(node: &LayoutNode, direction: SplitDirection) -> usize {
    match node {
        LayoutNode::Split {
            direction: node_direction,
            first,
            second,
        } if *node_direction == direction => {
            slot_count(first, direction) + slot_count(second, direction)
        }
        _ => 1,
    }
}

/// The splits passed on the way down to a pane, each with the branch taken there.
fn descent_to_pane<'a>(node: &'a LayoutNode, pane_id: &str) -> Option<Vec<(&'a LayoutNode, bool)>> {
    match node {
        LayoutNode::Pane { pane_id: found } => (found == pane_id).then(Vec::new),
        LayoutNode::Split { first, second, .. } => [(first, false), (second, true)]
            .into_iter()
            .find_map(|(child, branch)| {
                let descent = descent_to_pane(child, pane_id)?;
                Some(std::iter::once((node, branch)).chain(descent).collect())
            }),
    }
}

fn leaf_pane_ids(node: &LayoutNode) -> Vec<&str> {
    match node {
        LayoutNode::Pane { pane_id } => vec![pane_id.as_str()],
        LayoutNode::Split { first, second, .. } => leaf_pane_ids(first)
            .into_iter()
            .chain(leaf_pane_ids(second))
            .collect(),
    }
}

fn child_path(path: &[bool], branch: bool) -> Vec<bool> {
    path.iter().copied().chain([branch]).collect()
}

fn workspace_id_for_tab<'a>(snapshot: &'a Snapshot, tab_id: &str) -> Result<&'a str> {
    snapshot
        .tabs
        .iter()
        .find(|tab| tab.tab_id == tab_id)
        .map(|tab| tab.workspace_id.as_str())
        .ok_or_else(|| format!("target tab not found in herdr snapshot: {tab_id}"))
}

fn layout_bounds(layout: &TabLayout) -> Result<Bounds> {
    let first = layout.panes.first().ok_or_else(|| {
        format!(
            "target tab contains no pane in herdr layout: {}",
            layout.tab_id
        )
    })?;
    let (mut min_x, mut min_y) = (first.rect.x, first.rect.y);
    let (mut max_x, mut max_y) = (
        first.rect.x + first.rect.width,
        first.rect.y + first.rect.height,
    );
    for pane in &layout.panes[1..] {
        min_x = min_x.min(pane.rect.x);
        min_y = min_y.min(pane.rect.y);
        max_x = max_x.max(pane.rect.x + pane.rect.width);
        max_y = max_y.max(pane.rect.y + pane.rect.height);
    }
    Ok(Bounds {
        height: max_y - min_y,
        width: max_x - min_x,
        x: min_x,
        y: min_y,
    })
}

fn is_pane_in_direction(
    candidate: &LayoutPane,
    pane: &LayoutPane,
    direction: PaneDirection,
) -> bool {
    match direction {
        PaneDirection::Left => candidate.rect.x + candidate.rect.width <= pane.rect.x,
        PaneDirection::Right => candidate.rect.x >= pane.rect.x + pane.rect.width,
        PaneDirection::Up => candidate.rect.y + candidate.rect.height <= pane.rect.y,
        PaneDirection::Down => candidate.rect.y >= pane.rect.y + pane.rect.height,
    }
}

fn cross_axis_overlaps(left: &LayoutPane, right: &LayoutPane, direction: PaneDirection) -> bool {
    match direction {
        PaneDirection::Left | PaneDirection::Right => intervals_overlap(
            left.rect.y,
            left.rect.y + left.rect.height,
            right.rect.y,
            right.rect.y + right.rect.height,
        ),
        PaneDirection::Up | PaneDirection::Down => intervals_overlap(
            left.rect.x,
            left.rect.x + left.rect.width,
            right.rect.x,
            right.rect.x + right.rect.width,
        ),
    }
}

fn intervals_overlap(left_start: f64, left_end: f64, right_start: f64, right_end: f64) -> bool {
    left_start.max(right_start) < left_end.min(right_end)
}

fn primary_distance(candidate: &LayoutPane, pane: &LayoutPane, direction: PaneDirection) -> f64 {
    match direction {
        PaneDirection::Left => pane.rect.x - (candidate.rect.x + candidate.rect.width),
        PaneDirection::Right => candidate.rect.x - (pane.rect.x + pane.rect.width),
        PaneDirection::Up => pane.rect.y - (candidate.rect.y + candidate.rect.height),
        PaneDirection::Down => candidate.rect.y - (pane.rect.y + pane.rect.height),
    }
}

fn axis_distance(pane: &LayoutPane, direction: PaneDirection, point: Point) -> f64 {
    match direction {
        PaneDirection::Left | PaneDirection::Right => {
            interval_distance(point.y, pane.rect.y, pane.rect.y + pane.rect.height)
        }
        PaneDirection::Up | PaneDirection::Down => {
            interval_distance(point.x, pane.rect.x, pane.rect.x + pane.rect.width)
        }
    }
}

fn axis_center_distance(pane: &LayoutPane, direction: PaneDirection, point: Point) -> f64 {
    let center = pane_center(pane);
    match direction {
        PaneDirection::Left | PaneDirection::Right => (point.y - center.y).abs(),
        PaneDirection::Up | PaneDirection::Down => (point.x - center.x).abs(),
    }
}

fn interval_distance(value: f64, start: f64, end: f64) -> f64 {
    if value < start {
        start - value
    } else if value > end {
        value - end
    } else {
        0.0
    }
}

fn entry_edge_panes(layout: &TabLayout, direction: PaneDirection) -> Vec<&LayoutPane> {
    let edge = layout
        .panes
        .iter()
        .map(|pane| match direction {
            PaneDirection::Left => pane.rect.x + pane.rect.width,
            PaneDirection::Right => pane.rect.x,
            PaneDirection::Up => pane.rect.y + pane.rect.height,
            PaneDirection::Down => pane.rect.y,
        })
        .reduce(|left, right| match direction {
            PaneDirection::Left | PaneDirection::Up => left.max(right),
            PaneDirection::Right | PaneDirection::Down => left.min(right),
        });
    layout
        .panes
        .iter()
        .filter(|pane| {
            edge == Some(match direction {
                PaneDirection::Left => pane.rect.x + pane.rect.width,
                PaneDirection::Right => pane.rect.x,
                PaneDirection::Up => pane.rect.y + pane.rect.height,
                PaneDirection::Down => pane.rect.y,
            })
        })
        .collect()
}

fn move_anchor_point(direction: PaneDirection, point: Point, target: &LayoutPane) -> Point {
    let center = pane_center(target);
    match direction {
        PaneDirection::Left | PaneDirection::Right => Point {
            x: center.x,
            y: point
                .y
                .clamp(target.rect.y, target.rect.y + target.rect.height),
        },
        PaneDirection::Up | PaneDirection::Down => Point {
            x: point
                .x
                .clamp(target.rect.x, target.rect.x + target.rect.width),
            y: center.y,
        },
    }
}

fn pane_center(pane: &LayoutPane) -> Point {
    Point {
        x: pane.rect.x + pane.rect.width / 2.0,
        y: pane.rect.y + pane.rect.height / 2.0,
    }
}

fn clamp_point_to_pane(point: Point, pane: &LayoutPane) -> Point {
    Point {
        x: point.x.clamp(pane.rect.x, pane.rect.x + pane.rect.width),
        y: point.y.clamp(pane.rect.y, pane.rect.y + pane.rect.height),
    }
}

fn normalize_point(bounds: Bounds, point: Point) -> Point {
    Point {
        x: if bounds.width == 0.0 {
            0.0
        } else {
            ((point.x - bounds.x) / bounds.width).clamp(0.0, 1.0)
        },
        y: if bounds.height == 0.0 {
            0.0
        } else {
            ((point.y - bounds.y) / bounds.height).clamp(0.0, 1.0)
        },
    }
}

fn denormalize_point(bounds: Bounds, point: Point) -> Point {
    Point {
        x: bounds.x + point.x * bounds.width,
        y: bounds.y + point.y * bounds.height,
    }
}

fn compare_reading_order(left: &LayoutPane, right: &LayoutPane) -> Ordering {
    left.rect
        .y
        .total_cmp(&right.rect.y)
        .then_with(|| left.rect.x.total_cmp(&right.rect.x))
        .then_with(|| left.pane_id.cmp(&right.pane_id))
}

fn compare_boundary_reading_order(
    left: &LayoutPane,
    right: &LayoutPane,
    direction: PaneDirection,
) -> Ordering {
    let order = compare_reading_order(left, right);
    if matches!(direction, PaneDirection::Left | PaneDirection::Up) {
        order.reverse()
    } else {
        order
    }
}

fn is_unit_interval(value: f64) -> bool {
    value.is_finite() && (0.0..=1.0).contains(&value)
}

fn timestamp() -> Result<u128> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .map_err(|error| error.to_string())
}

fn required_env(name: &str) -> Result<String> {
    env::var(name)
        .ok()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("missing {name}"))
}
