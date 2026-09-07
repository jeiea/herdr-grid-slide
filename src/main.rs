use std::cmp::Ordering;
use std::env;
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
#[cfg(unix)]
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

const FOCUS_ANCHOR_STATE_FILE: &str = "focus-anchor.json";
const BALANCE_FOCUS_STATE_FILE: &str = "balance-focus.json";
const BALANCE_FOCUS_LOCK_FILE: &str = "balance-focus.lock";
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
    CreateTab,
    CreateWorkspace,
    MoveToNewTab,
    MoveTabWorkspace(Option<MoveDirection>),
    MoveWorkspace(MoveDirection),
    Focus(PaneDirection),
    MoveDirectionally(PaneDirection),
    CreatePane,
    OnPaneFocused,
    Balance,
}

#[derive(Clone, Copy, PartialEq)]
enum BalanceFocusPolicy {
    Automatic,
    Explicit,
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

struct PaneAction {
    pane_id: String,
    split_direction: SplitDirection,
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
    label: String,
    tab_id: String,
    workspace_id: String,
}

#[derive(Deserialize)]
struct TabLayout {
    #[serde(default)]
    focused_pane_id: Option<String>,
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
struct TabCreateResult {
    tab: Option<TabRef>,
}

#[derive(Deserialize)]
struct WorkspaceCreateResult {
    workspace: Option<WorkspaceRef>,
}

#[derive(Deserialize)]
struct PaneRef {
    pane_id: String,
}

#[derive(Deserialize)]
struct PaneMoveResult {
    move_result: MoveOutcome,
}

#[derive(Deserialize)]
struct PaneSwapResult {
    swap: SwapOutcome,
}

/// What herdr made of a move. A move it declined comes back with `changed: false`
/// and a reason instead of an error.
#[derive(Deserialize)]
struct MoveOutcome {
    changed: bool,
    created_tab: Option<TabRef>,
    created_workspace: Option<WorkspaceRef>,
    pane: PaneRef,
    reason: Option<String>,
    #[serde(default)]
    source_layout: Option<TabLayout>,
}

/// What herdr made of a swap. A missing pane or a pane that changed tabs is a
/// successful API response with `changed: false` and a reason.
#[derive(Deserialize)]
struct SwapOutcome {
    changed: bool,
    reason: Option<String>,
}

impl SwapOutcome {
    fn refusal_reason(self) -> Option<String> {
        (!self.changed).then(|| self.reason.unwrap_or_else(|| "no reason given".to_owned()))
    }
}

#[derive(Deserialize)]
struct TabRef {
    tab_id: String,
}

#[derive(Deserialize)]
struct WorkspaceRef {
    workspace_id: String,
}

impl SplitDirection {
    fn as_str(self) -> &'static str {
        match self {
            SplitDirection::Right => "right",
            SplitDirection::Down => "down",
        }
    }
}

impl PaneDirection {
    fn boundary_split(self) -> SplitDirection {
        match self {
            PaneDirection::Left | PaneDirection::Right => SplitDirection::Right,
            PaneDirection::Up | PaneDirection::Down => SplitDirection::Down,
        }
    }

    fn requires_boundary_swap(self) -> bool {
        matches!(self, PaneDirection::Right | PaneDirection::Down)
    }

    fn is_horizontal(self) -> bool {
        matches!(self, PaneDirection::Left | PaneDirection::Right)
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

/// The tab the automatic balance last entered, as it was laid out then. The
/// bounds are part of it because Herdr sends no event when the window is resized
/// or a client of another size attaches, and the grid that suits a phone differs
/// from the one that suited the desktop. Comparing them exactly is sound because
/// Herdr reports whole terminal cells, so no rounding can creep in.
#[derive(PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BalanceFocusState {
    bounds: Option<Bounds>,
    pane_ids: Vec<String>,
    tab_id: String,
}

#[derive(Clone, Copy)]
struct Point {
    x: f64,
    y: f64,
}

#[derive(Clone, Copy, PartialEq, Serialize, Deserialize)]
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

/// One row of a tab read as a grid, with the path of the subtree holding it.
struct GridRow<'a> {
    node: &'a LayoutNode,
    panes: Vec<&'a str>,
    path: Vec<bool>,
}

/// The grid a tab is being balanced into: its panes in reading order and how many of
/// them belong on each row.
struct GridPlan<'a> {
    focus_policy: BalanceFocusPolicy,
    focus_pane_id: &'a str,
    order: Vec<&'a str>,
    row_sizes: Vec<usize>,
    tab_id: &'a str,
    workspace_id: &'a str,
}

/// The tab panes are parked in while their own tab is rebuilt around them.
#[derive(Default)]
struct ScratchTab<'a> {
    expected_focus_pane_id: Option<String>,
    panes: Vec<&'a str>,
    tab_id: Option<String>,
}

impl<'a> GridPlan<'a> {
    /// The panes of each row, in reading order.
    fn rows(&self) -> Vec<&[&'a str]> {
        let mut rows = Vec::new();
        let mut rest = self.order.as_slice();
        for size in &self.row_sizes {
            let (row, tail) = rest.split_at(*size);
            rows.push(row);
            rest = tail;
        }
        rows
    }

    /// Whether the tab is already shaped like the target grid, however its splits are
    /// nested and whichever pane sits in which cell.
    fn matches(&self, grid: &[GridRow]) -> bool {
        grid.iter()
            .map(|row| row.panes.len())
            .eq(self.row_sizes.iter().copied())
    }

    /// Whether the tab still holds exactly the panes the plan was made for.
    fn holds(&self, grid: &[GridRow]) -> bool {
        let mut placed: Vec<_> = grid
            .iter()
            .flat_map(|row| row.panes.iter().copied())
            .collect();
        let mut expected = self.order.clone();
        placed.sort_unstable();
        expected.sort_unstable();
        placed == expected
    }
}

fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let command = parse_arguments(env::args().skip(1))?;
    let context = read_context(matches!(
        command,
        Command::Focus(_)
            | Command::MoveDirectionally(_)
            | Command::MoveTabWorkspace(_)
            | Command::Move {
                scope: Scope::Tab,
                ..
            }
            | Command::OnPaneFocused
    ))?;
    let mut client = SocketClient::new(&context.socket_path);
    match command {
        Command::Focus(direction) => focus_with_anchor(&context, direction, &mut client),
        Command::MoveDirectionally(direction) => {
            move_directionally(&context, direction, &mut client)
        }
        Command::Move { direction, scope } => move_pane(&context, direction, scope, &mut client),
        Command::CreateTab => create_tab_to_right(&context, &mut client),
        Command::CreateWorkspace => create_workspace_after_current(&context, &mut client),
        Command::MoveToNewTab => move_pane_to_new_tab(&context, &mut client),
        Command::MoveTabWorkspace(direction) => {
            move_tab_workspace(&context, direction, &mut client)
        }
        Command::MoveWorkspace(direction) => move_workspace(&context, direction, &mut client),
        Command::CreatePane => create_pane(&context, &mut client),
        Command::OnPaneFocused => balance_focused_pane(&context, &mut client, None),
        Command::Balance => balance(&context, &mut client),
    }
}

fn parse_arguments(args: impl Iterator<Item = String>) -> Result<Command> {
    let args: Vec<_> = args.collect();
    match args.as_slice() {
        [operation] => match operation.as_str() {
            "new-tab" => Ok(Command::CreateTab),
            "new-workspace" => Ok(Command::CreateWorkspace),
            "split-pane" | "new-pane" => Ok(Command::CreatePane),
            "to-new-tab" => Ok(Command::MoveToNewTab),
            "to-new-workspace" => Ok(Command::MoveTabWorkspace(None)),
            "on-pane-focused" => Ok(Command::OnPaneFocused),
            "balance" => Ok(Command::Balance),
            _ => Err(usage()),
        },
        [operation, direction] if operation == "focus" => {
            Ok(Command::Focus(parse_pane_direction(direction)?))
        }
        [operation, direction] if operation == "move" => {
            Ok(Command::MoveDirectionally(parse_pane_direction(direction)?))
        }
        [operation, direction] if operation == "move-tab-workspace" => Ok(
            Command::MoveTabWorkspace(Some(parse_move_direction(direction)?)),
        ),
        [operation, direction] if operation == "move-workspace" => {
            Ok(Command::MoveWorkspace(parse_move_direction(direction)?))
        }
        [scope, direction] => {
            let scope = match scope.as_str() {
                "workspace" => Scope::Workspace,
                "tab" => Scope::Tab,
                _ => return Err(usage()),
            };
            Ok(Command::Move {
                direction: parse_move_direction(direction)?,
                scope,
            })
        }
        _ => Err(usage()),
    }
}

fn parse_move_direction(direction: &str) -> Result<MoveDirection> {
    match direction {
        "next" => Ok(MoveDirection::Next),
        "previous" => Ok(MoveDirection::Previous),
        _ => Err(usage()),
    }
}

fn parse_pane_direction(direction: &str) -> Result<PaneDirection> {
    match direction {
        "left" => Ok(PaneDirection::Left),
        "right" => Ok(PaneDirection::Right),
        "up" => Ok(PaneDirection::Up),
        "down" => Ok(PaneDirection::Down),
        _ => Err(usage()),
    }
}

fn usage() -> String {
    "usage: herdr-move-pane <workspace|tab> <next|previous> | move-workspace <next|previous> | move-tab-workspace <next|previous> | <focus|move> <direction> | new-tab | new-workspace | to-new-tab | to-new-workspace | split-pane | new-pane | balance | on-pane-focused".into()
}

fn read_context(needs_state_dir: bool) -> Result<Context> {
    Ok(Context {
        pane_id: required_env("HERDR_PANE_ID")?,
        socket_path: required_env("HERDR_SOCKET_PATH")?,
        state_dir: needs_state_dir
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
    let pane_id = navigation.pane_id.to_owned();
    let target_tab_id =
        resolve_move_target_tab(&navigation, direction, scope, &snapshot)?.to_owned();
    if target_tab_id == navigation.tab_id {
        return Ok(());
    }
    let moved = request_pane_move(client, &pane_id, tab_edge(&target_tab_id), true)?;
    if !moved.changed || !matches!(scope, Scope::Tab) {
        return Ok(());
    }
    balance_focused_pane(context, client, Some(&target_tab_id))
        .map_err(|error| format!("pane moved, but automatic balance failed: {error}"))
}

fn create_tab_to_right(context: &Context, client: &mut SocketClient) -> Result<()> {
    let snapshot = read_snapshot(client)?;
    let navigation = navigation_context(context, &snapshot);
    let tabs = tabs_in_display_order(&snapshot, navigation.workspace_id);
    let source = current_position(&tabs, navigation.tab_id, |tab| tab.tab_id.as_str())?;
    let result = client.request(
        "tab.create",
        json!({"workspace_id": navigation.workspace_id, "focus": true}),
    )?;
    if source + 1 == tabs.len() {
        return Ok(());
    }
    let created_tab_id = serde_json::from_value::<TabCreateResult>(result)
        .ok()
        .and_then(|created| created.tab)
        .map(|tab| tab.tab_id)
        .ok_or(
            "tab creation completed, but herdr api tab.create response is missing the created tab id",
        )?;
    client
        .request(
            "tab.move",
            json!({"tab_id": &created_tab_id, "insert_index": source + 1}),
        )
        .map_err(|error| {
            format!(
                "tab creation completed, but positioning {created_tab_id} after {} failed: {error}",
                navigation.tab_id
            )
        })?;
    Ok(())
}

fn create_workspace_after_current(context: &Context, client: &mut SocketClient) -> Result<()> {
    let snapshot = read_snapshot(client)?;
    let navigation = navigation_context(context, &snapshot);
    let workspaces = workspaces_in_visible_order(&snapshot);
    let source = current_position(&workspaces, navigation.workspace_id, |workspace| {
        workspace.workspace_id.as_str()
    })?;
    let result = client.request("workspace.create", json!({"focus": true}))?;
    if source + 1 == workspaces.len() {
        return Ok(());
    }
    let created_workspace_id = serde_json::from_value::<WorkspaceCreateResult>(result)
        .ok()
        .and_then(|created| created.workspace)
        .map(|workspace| workspace.workspace_id)
        .ok_or(
            "workspace creation completed, but herdr api workspace.create response is missing the created workspace id",
        )?;
    client
        .request(
            "workspace.move",
            json!({"workspace_id": &created_workspace_id, "insert_index": source + 1}),
        )
        .map_err(|error| {
            format!(
                "workspace creation completed, but positioning {created_workspace_id} after {} failed: {error}",
                navigation.workspace_id
            )
        })?;
    Ok(())
}

fn move_pane_to_new_tab(context: &Context, client: &mut SocketClient) -> Result<()> {
    let snapshot = read_snapshot(client)?;
    let navigation = navigation_context(context, &snapshot);
    if find_layout(&snapshot, navigation.tab_id)?.panes.len() <= 1 {
        return Ok(());
    }
    let tabs = tabs_in_display_order(&snapshot, navigation.workspace_id);
    let source = current_position(&tabs, navigation.tab_id, |tab| tab.tab_id.as_str())?;
    let moved = request_pane_move(
        client,
        navigation.pane_id,
        json!({"type": "new_tab", "workspace_id": navigation.workspace_id}),
        true,
    )?;
    if !moved.changed {
        return Ok(());
    }
    if source + 1 == tabs.len() {
        return Ok(());
    }
    let created_tab = moved.created_tab.ok_or(
        "pane moved to a new tab, but herdr api pane.move response is missing the created tab id",
    )?;
    client
        .request(
            "tab.move",
            json!({"tab_id": created_tab.tab_id, "insert_index": source + 1}),
        )
        .map_err(|error| {
            format!(
                "pane moved to a new tab, but positioning {} after {} failed: {error}",
                created_tab.tab_id, navigation.tab_id
            )
        })?;
    Ok(())
}

/// Moves the focused tab into a new tab in another workspace. herdr has no request that
/// carries a tab across workspaces -- `tab.move` only reorders within one -- so the
/// first pane in reading order opens the destination tab and the rest of the tab follows it
/// one pane at a time, in that order, so the balanced tab keeps its pane order. Focus
/// then returns to the pane that had it. The whole move runs under the balance lock:
/// the `pane.focused` hooks it fires along the way wait for it, then find the tab
/// settled, instead of balancing a half-moved tab and handing focus back to the
/// leader. Plugin policy keeps the only tab of a workspace in place so the source
/// workspace stays open.
fn move_tab_workspace(
    context: &Context,
    direction: Option<MoveDirection>,
    client: &mut SocketClient,
) -> Result<()> {
    let lock = lock_balance(context)?;
    let snapshot = read_snapshot(client)?;
    let navigation = navigation_context(context, &snapshot);
    let tabs = tabs_in_display_order(&snapshot, navigation.workspace_id);
    if tabs.len() <= 1 {
        return Ok(());
    }
    let tab_position = current_position(&tabs, navigation.tab_id, |tab| tab.tab_id.as_str())?;
    let workspaces = workspaces_in_visible_order(&snapshot);
    let source = current_position(&workspaces, navigation.workspace_id, |workspace| {
        workspace.workspace_id.as_str()
    })?;
    let label = custom_label(&tabs[tab_position].label, tab_position);
    let destination = if let Some(direction) = direction {
        if workspaces.len() <= 1 {
            return Ok(());
        }
        let target = adjacent(
            &workspaces,
            navigation.workspace_id,
            direction,
            |workspace| workspace.workspace_id.as_str(),
        )?;
        let mut destination = json!({"type": "new_tab", "workspace_id": target.workspace_id});
        if let Some(label) = label {
            destination["label"] = label.into();
        }
        destination
    } else {
        new_workspace_destination(label)
    };
    let order = reading_order(find_layout(&snapshot, navigation.tab_id)?);
    let (leader, following) = order
        .split_first()
        .ok_or_else(|| format!("{} has no panes", navigation.tab_id))?;
    let moved = request_pane_move(client, leader, destination, true)?;
    if !moved.changed {
        return Ok(());
    }
    let created_tab_id = follow_leader(client, &moved, leader, following, navigation.tab_id)?;
    if *leader != navigation.pane_id {
        client
            .request("pane.focus", json!({"pane_id": navigation.pane_id}))
            .map_err(|error| {
                format!(
                    "tab moved, but refocusing {} failed: {error}",
                    navigation.pane_id
                )
            })?;
    }
    if direction.is_none() && source + 1 < workspaces.len() {
        let created_workspace = moved.created_workspace.as_ref().ok_or(
            "tab moved to a new workspace, but herdr api pane.move response is missing the created workspace id",
        )?;
        client
            .request(
                "workspace.move",
                json!({
                    "workspace_id": created_workspace.workspace_id,
                    "insert_index": source + 1,
                }),
            )
            .map_err(|error| {
                format!(
                    "tab moved to a new workspace, but positioning {} after {} failed: {error}",
                    created_workspace.workspace_id, navigation.workspace_id
                )
            })?;
    }
    match created_tab_id {
        Some(tab_id) => balance_focused_pane_locked(&lock, client, Some(tab_id))
            .map_err(|error| format!("tab moved, but automatic balance failed: {error}")),
        None => Ok(()),
    }
}

/// Brings the panes that were left behind the leading pane into the tab its move
/// created, each to the right of the one before it so the tab keeps its reading
/// order; without a target herdr would split the still-focused leader every time
/// and reverse everything after the second pane. Panes take new IDs when they cross
/// workspaces, so each target is the ID herdr reported for the previous move. herdr
/// only reports the created tab on the leading move, so without its ID the rest of
/// the tab stays where it was; likewise a pane that cannot follow leaves itself and
/// the ones after it behind, and the error says which.
fn follow_leader<'a>(
    client: &mut SocketClient,
    moved: &'a MoveOutcome,
    leader: &str,
    following: &[&str],
    source_tab_id: &str,
) -> Result<Option<&'a str>> {
    if following.is_empty() {
        return Ok(None);
    }
    let left_behind = |problem: String, remaining: &[&str]| {
        let verb = if remaining.len() == 1 {
            "remains"
        } else {
            "remain"
        };
        format!(
            "{leader} moved, but {problem}; {} {verb} in {source_tab_id}",
            remaining.join(", ")
        )
    };
    let created_tab_id = moved
        .created_tab
        .as_ref()
        .map(|tab| tab.tab_id.as_str())
        .ok_or_else(|| {
            left_behind(
                "herdr api pane.move response is missing the created tab id".to_owned(),
                following,
            )
        })?;
    let mut target_pane_id = moved.pane.pane_id.clone();
    for (index, pane_id) in following.iter().enumerate() {
        let destination = attachment(created_tab_id, &target_pane_id, SplitDirection::Right);
        let outcome = move_pane_to(client, pane_id, destination).map_err(|error| {
            left_behind(
                format!("moving {pane_id} after it failed: {error}"),
                &following[index..],
            )
        })?;
        target_pane_id = outcome.pane.pane_id;
    }
    Ok(Some(created_tab_id))
}

fn new_workspace_destination(tab_label: Option<&str>) -> Value {
    let mut destination = json!({"type": "new_workspace"});
    if let Some(label) = tab_label {
        destination["tab_label"] = label.into();
    }
    destination
}

/// herdr reports a tab's display label, which is its custom name or, failing that,
/// its one-based position in the workspace, and the snapshot does not say which. A
/// label that spells the tab's own position is taken as the default and left
/// behind, because carrying it over would pin the new tab to a position it no
/// longer has. A custom name that happens to equal that position is lost with it;
/// herdr 0.8.2 offers nothing to tell the two apart. Once a herdr snapshot exposes
/// whether a label is custom, use that flag here instead of this comparison.
fn custom_label(label: &str, position: usize) -> Option<&str> {
    (label != (position + 1).to_string()).then_some(label)
}

/// Moves the active workspace itself one step through the visible number order,
/// wrapping at either end. herdr's `workspace.move` takes the insertion slot counted
/// before the workspace is removed: stepping forward inserts two slots ahead, and
/// the ends wrap to slot 0 or to one past the last.
fn move_workspace(
    context: &Context,
    direction: MoveDirection,
    client: &mut SocketClient,
) -> Result<()> {
    let snapshot = read_snapshot(client)?;
    let navigation = navigation_context(context, &snapshot);
    let items = workspaces_in_visible_order(&snapshot);
    if items.len() < 2 {
        return Ok(());
    }
    let source = current_position(&items, navigation.workspace_id, |workspace| {
        workspace.workspace_id.as_str()
    })?;
    let insert_index = match direction {
        MoveDirection::Next if source == items.len() - 1 => 0,
        MoveDirection::Next => source + 2,
        MoveDirection::Previous if source == 0 => items.len(),
        MoveDirection::Previous => source - 1,
    };
    client.request(
        "workspace.move",
        json!({
            "workspace_id": navigation.workspace_id,
            "insert_index": insert_index,
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

fn move_directionally(
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
    let (target, mut next_state) =
        resolve_focus_target(&navigation, direction, &snapshot, state.as_ref())?;
    if target.pane_id == navigation.pane_id {
        return Ok(());
    }
    let source_pane_id = navigation.pane_id.to_owned();
    let source_tab_id = navigation.tab_id.to_owned();
    let target_pane_id = target.pane_id.clone();
    if next_state.tab_id == source_tab_id {
        let swapped = request_pane_swap(client, &source_pane_id, &target_pane_id)?;
        if swapped.refusal_reason().is_some() {
            return Ok(());
        }
        next_state.pane_id = source_pane_id;
        return write_focus_anchor_state(state_dir, &next_state);
    }

    let split_direction = direction.boundary_split();
    let needs_swap = direction.requires_boundary_swap();
    let target_tab_id = next_state.tab_id.clone();
    let moved = request_pane_move(
        client,
        &source_pane_id,
        attachment(&target_tab_id, &target_pane_id, split_direction),
        !needs_swap,
    )?;
    if !moved.changed {
        return Ok(());
    }
    let moved_pane_id = moved.pane.pane_id;
    if needs_swap {
        let swapped = request_pane_swap(client, &moved_pane_id, &target_pane_id)
            .map_err(|error| format!("pane moved, but directional swap failed: {error}"))?;
        if let Some(reason) = swapped.refusal_reason() {
            return Err(format!("pane moved, but directional swap failed: {reason}"));
        }
    }
    next_state.pane_id = moved_pane_id;
    write_focus_anchor_state(state_dir, &next_state)?;
    if direction.is_horizontal() {
        balance_focused_pane(context, client, Some(&target_tab_id))
            .map_err(|error| format!("pane moved, but automatic balance failed: {error}"))?;
    }
    Ok(())
}

/// Splits the focused pane along the direction the tab already grows in.
fn create_pane(context: &Context, client: &mut SocketClient) -> Result<()> {
    let pane = prepare_pane_action(context, client)?;
    request_pane_split(client, &pane.pane_id, pane.split_direction)?;
    Ok(())
}

/// Reads and validates everything needed before a pane change can have side effects.
fn prepare_pane_action(context: &Context, client: &mut SocketClient) -> Result<PaneAction> {
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
    Ok(PaneAction {
        pane_id: navigation.pane_id.to_owned(),
        split_direction: split_direction(&layout.root, pane),
    })
}

/// Spreads the panes of a tab over an even grid, in reading order. A tab that is
/// already a grid of the right shape only needs resizing, and panes sitting in the
/// wrong cell are swapped into place, so running this twice changes nothing the
/// second time. Anything else has to be taken apart and rebuilt.
fn balance(context: &Context, client: &mut SocketClient) -> Result<()> {
    let snapshot = read_snapshot(client)?;
    let navigation = navigation_context(context, &snapshot);
    let snapshot_layout = find_layout(&snapshot, navigation.tab_id)?;
    balance_layout(
        client,
        snapshot_layout,
        navigation.tab_id,
        navigation.workspace_id,
        navigation.pane_id,
        BalanceFocusPolicy::Explicit,
    )
}

fn balance_focused_pane(
    context: &Context,
    client: &mut SocketClient,
    expected_destination_tab_id: Option<&str>,
) -> Result<()> {
    let lock = lock_balance(context)?;
    balance_focused_pane_locked(&lock, client, expected_destination_tab_id)
}

/// The lock every hook and tab-scoped move takes before reading the session, held
/// for as long as the value lives.
struct BalanceLock<'a> {
    state_dir: &'a Path,
    _file: File,
}

fn lock_balance(context: &Context) -> Result<BalanceLock<'_>> {
    let state_dir = context
        .state_dir
        .as_deref()
        .ok_or("missing HERDR_PLUGIN_STATE_DIR")?;
    fs::create_dir_all(state_dir).map_err(|error| error.to_string())?;
    let file = OpenOptions::new()
        .create(true)
        .read(true)
        .truncate(false)
        .write(true)
        .open(state_dir.join(BALANCE_FOCUS_LOCK_FILE))
        .map_err(|error| error.to_string())?;
    file.lock().map_err(|error| error.to_string())?;
    Ok(BalanceLock {
        state_dir,
        _file: file,
    })
}

fn balance_focused_pane_locked(
    lock: &BalanceLock,
    client: &mut SocketClient,
    mut expected_destination_tab_id: Option<&str>,
) -> Result<()> {
    let state_dir = lock.state_dir;
    let mut previous = read_balance_focus_state(state_dir)?;
    let mut failures = Vec::new();
    loop {
        let snapshot = read_snapshot(client)?;
        let Some(tab_id) = snapshot.focused_tab_id.as_deref() else {
            break;
        };
        let snapshot_layout = snapshot
            .layouts
            .iter()
            .find(|layout| layout.tab_id == tab_id);
        // A focus inside the settled tab is not a tab entry, but a new or closed
        // pane changes the pane set, and a resized window changes the bounds; both
        // warrant a balance.
        let observed = observed_balance_focus(tab_id, snapshot_layout);
        let force_balance = expected_destination_tab_id.take() == Some(tab_id);
        if !force_balance && previous.as_ref() == Some(&observed) {
            break;
        }
        write_state_file(state_dir, BALANCE_FOCUS_STATE_FILE, &observed)?;
        previous = Some(observed);

        let Some(snapshot_layout) = snapshot_layout else {
            continue;
        };
        if snapshot_layout.panes.len() <= 1 {
            continue;
        }
        let Some(workspace_id) = snapshot.focused_workspace_id.as_deref() else {
            continue;
        };
        let Some(focus_pane_id) = snapshot.focused_pane_id.as_deref() else {
            continue;
        };
        if let Err(error) =
            balance_automatic_layout(client, snapshot_layout, tab_id, workspace_id, focus_pane_id)
        {
            failures.push(format!("could not automatically balance {tab_id}: {error}"));
        }
    }

    if failures.is_empty() {
        Ok(())
    } else {
        Err(failures.join("; "))
    }
}

fn balance_layout(
    client: &mut SocketClient,
    snapshot_layout: &TabLayout,
    tab_id: &str,
    workspace_id: &str,
    focus_pane_id: &str,
    focus_policy: BalanceFocusPolicy,
) -> Result<()> {
    let layout = read_layout(client, tab_id)?;
    if !holds_the_same_panes(snapshot_layout, &layout.root) {
        return Err(format!(
            "{tab_id} changed while it was being read; try again"
        ));
    }
    if layout.zoomed {
        // Unzooming here would leave the tab zoomed out if a later step failed.
        return Err(format!("{tab_id} is zoomed; unzoom it before balancing"));
    }
    balance_exported_layout(
        client,
        snapshot_layout,
        &layout,
        tab_id,
        workspace_id,
        focus_pane_id,
        focus_policy,
    )
}

fn balance_automatic_layout(
    client: &mut SocketClient,
    snapshot_layout: &TabLayout,
    tab_id: &str,
    workspace_id: &str,
    focus_pane_id: &str,
) -> Result<()> {
    let layout = read_layout(client, tab_id)?;
    if layout.zoomed {
        return Ok(());
    }
    if !holds_the_same_panes(snapshot_layout, &layout.root) {
        return Err(format!(
            "{tab_id} changed while it was being read; try again"
        ));
    }
    balance_exported_layout(
        client,
        snapshot_layout,
        &layout,
        tab_id,
        workspace_id,
        focus_pane_id,
        BalanceFocusPolicy::Automatic,
    )
}

fn balance_exported_layout(
    client: &mut SocketClient,
    snapshot_layout: &TabLayout,
    layout: &ExportedLayout,
    tab_id: &str,
    workspace_id: &str,
    focus_pane_id: &str,
    focus_policy: BalanceFocusPolicy,
) -> Result<()> {
    let order = reading_order(snapshot_layout);
    if order.len() < 2 {
        return Ok(());
    }
    let plan = GridPlan {
        focus_policy,
        focus_pane_id,
        row_sizes: grid_row_sizes(order.len(), layout_bounds(snapshot_layout)?)?,
        order,
        tab_id,
        workspace_id,
    };
    match read_grid(&layout.root).filter(|grid| plan.matches(grid)) {
        Some(grid) => sort_grid(client, &plan, &layout.root, &grid),
        None => rebuild_grid(client, &plan),
    }
}

/// Puts a tab that is already the right grid in order: panes sitting in the wrong
/// cell trade places, then every split is resized.
fn sort_grid(
    client: &mut SocketClient,
    plan: &GridPlan,
    root: &LayoutNode,
    grid: &[GridRow],
) -> Result<()> {
    let placed = grid.iter().flat_map(|row| row.panes.iter().copied());
    let swaps = swaps_towards(placed.collect(), &plan.order);
    for (index, (source, target)) in swaps.iter().enumerate() {
        ensure_automatic_target_is_focused(client, plan)?;
        let swap = client.request(
            "pane.swap",
            json!({"source_pane_id": source, "target_pane_id": target}),
        );
        if let Err(error) = swap {
            return Err(with_focus_restored(client, plan, index > 0, error));
        }
    }
    for (path, ratio) in grid_ratios(root, grid) {
        let resize = client.request(
            "layout.set_split_ratio",
            json!({"tab_id": plan.tab_id, "path": path, "ratio": ratio}),
        );
        if let Err(error) = resize {
            return Err(with_focus_restored(client, plan, !swaps.is_empty(), error));
        }
    }
    if let Some((source, _)) = swaps.last() {
        restore_focus_after_success(client, plan, source)?;
    }
    Ok(())
}

/// Takes a tab apart and lays it out again as the target grid. herdr refuses to move
/// a pane inside its own tab, so every pane but the first goes out to a scratch tab
/// and comes back one at a time; the last one to leave takes the empty scratch tab
/// with it. A rebuild interrupted halfway is not undone -- the panes are brought
/// home, but the shape they land in is whatever the moves left behind.
fn rebuild_grid(client: &mut SocketClient, plan: &GridPlan) -> Result<()> {
    let (anchor, staged) = plan
        .order
        .split_first()
        .ok_or("nothing to balance in an empty tab")?;
    let mut scratch = ScratchTab::default();
    for pane in staged {
        ensure_automatic_target_is_focused(client, plan)
            .map_err(|error| recovered(client, plan, anchor, &scratch, error))?;
        let destination = match &scratch.tab_id {
            Some(tab_id) => tab_edge(tab_id),
            None => json!({"type": "new_tab", "workspace_id": plan.workspace_id}),
        };
        match move_pane_to(client, pane, destination) {
            Ok(outcome) => {
                if let Some(source_layout) = outcome
                    .source_layout
                    .as_ref()
                    .filter(|layout| layout.tab_id == plan.tab_id)
                {
                    scratch.expected_focus_pane_id = source_layout.focused_pane_id.clone();
                }
                scratch.tab_id = scratch.tab_id.or(outcome.created_tab.map(|tab| tab.tab_id));
                scratch.panes.push(pane);
            }
            Err(error) => return Err(recovered(client, plan, anchor, &scratch, error)),
        }
        if scratch.tab_id.is_none() {
            // Without the tab id there is nowhere to fetch the pane back from, so
            // name it rather than leave the user hunting for it.
            let error = format!("herdr moved {pane} to a new tab without saying which one");
            return Err(with_focus_back(client, plan, error));
        }
    }
    for (pane, target, direction) in grid_moves(&plan.rows()) {
        ensure_automatic_target_is_focused(client, plan)
            .map_err(|error| recovered(client, plan, anchor, &scratch, error))?;
        let destination = attachment(plan.tab_id, target, direction);
        if let Err(error) = move_pane_to(client, pane, destination) {
            return Err(recovered(client, plan, anchor, &scratch, error));
        }
        scratch.panes.retain(|staged| *staged != pane);
    }
    match settle_rebuilt_grid(client, plan) {
        Ok(()) if plan.focus_policy == BalanceFocusPolicy::Explicit => {
            client.request("pane.focus", json!({"pane_id": plan.focus_pane_id}))?;
            Ok(())
        }
        Ok(()) => match scratch.expected_focus_pane_id.as_deref() {
            Some(expected_focus_pane_id) => {
                restore_focus_after_success(client, plan, expected_focus_pane_id)
            }
            None => Ok(()),
        },
        Err(error) => Err(with_focus_back(client, plan, error)),
    }
}

fn restore_focus_after_success(
    client: &mut SocketClient,
    plan: &GridPlan,
    expected_focus_pane_id: &str,
) -> Result<()> {
    if plan.focus_policy == BalanceFocusPolicy::Explicit {
        client.request("pane.focus", json!({"pane_id": plan.focus_pane_id}))?;
        return Ok(());
    }
    if expected_focus_pane_id == plan.focus_pane_id {
        return Ok(());
    }
    let snapshot = read_snapshot(client)?;
    if snapshot.focused_tab_id.as_deref() == Some(plan.tab_id)
        && snapshot.focused_pane_id.as_deref() == Some(expected_focus_pane_id)
    {
        let _ = client.request("pane.focus", json!({"pane_id": plan.focus_pane_id}));
    }
    Ok(())
}

/// Checks the rebuilt tab came out as planned and evens out its splits.
fn settle_rebuilt_grid(client: &mut SocketClient, plan: &GridPlan) -> Result<()> {
    let layout = read_layout(client, plan.tab_id).map_err(|error| {
        format!(
            "rebuilt {} but could not read it back: {error}",
            plan.tab_id
        )
    })?;
    let grid = read_grid(&layout.root)
        .filter(|grid| plan.matches(grid) && plan.holds(grid))
        .ok_or_else(|| {
            format!(
                "{} did not come out as the expected grid; left the sizes alone",
                plan.tab_id
            )
        })?;
    for (path, ratio) in grid_ratios(&layout.root, &grid) {
        client
            .request(
                "layout.set_split_ratio",
                json!({"tab_id": plan.tab_id, "path": path, "ratio": ratio}),
            )
            .map_err(|error| {
                format!(
                    "rebuilt {} but could not even out the sizes: {error}",
                    plan.tab_id
                )
            })?;
    }
    Ok(())
}

fn ensure_automatic_target_is_focused(client: &mut SocketClient, plan: &GridPlan) -> Result<()> {
    if plan.focus_policy == BalanceFocusPolicy::Explicit {
        return Ok(());
    }
    let snapshot = read_snapshot(client)?;
    if snapshot.focused_tab_id.as_deref() == Some(plan.tab_id) {
        Ok(())
    } else {
        Err(format!(
            "focus left {} while it was being balanced",
            plan.tab_id
        ))
    }
}

/// Where a pane goes when it joins a tab at its right edge.
fn tab_edge(tab_id: &str) -> Value {
    json!({"type": "tab", "tab_id": tab_id, "split": "right", "ratio": 0.5})
}

/// Where a pane goes when it is hung off one that is already in the tab.
fn attachment(tab_id: &str, target_pane_id: &str, direction: SplitDirection) -> Value {
    json!({
        "type": "tab",
        "tab_id": tab_id,
        "target_pane_id": target_pane_id,
        "split": direction.as_str(),
        "ratio": 0.5,
    })
}

/// Hands back a failure with the focus put where it started, best effort.
fn with_focus_back(client: &mut SocketClient, plan: &GridPlan, error: String) -> String {
    if plan.focus_policy == BalanceFocusPolicy::Explicit {
        let _ = client.request("pane.focus", json!({"pane_id": plan.focus_pane_id}));
    }
    error
}

/// Moves one pane and insists that herdr actually moved it: a refused move comes back
/// as `changed: false` with a reason rather than as an error, and reading that as
/// success would leave the rebuild working from a layout that never happened.
fn move_pane_to(
    client: &mut SocketClient,
    pane_id: &str,
    destination: Value,
) -> Result<MoveOutcome> {
    let moved = request_pane_move(client, pane_id, destination, false)?;
    if !moved.changed {
        let reason = moved.reason.unwrap_or_else(|| "no reason given".to_owned());
        return Err(format!("herdr refused to move {pane_id}: {reason}"));
    }
    Ok(moved)
}

fn request_pane_move(
    client: &mut SocketClient,
    pane_id: &str,
    destination: Value,
    focus: bool,
) -> Result<MoveOutcome> {
    let result = client.request(
        "pane.move",
        json!({"pane_id": pane_id, "destination": destination, "focus": focus}),
    )?;
    Ok(serde_json::from_value::<PaneMoveResult>(result)
        .map_err(|_| "herdr api pane.move returned an invalid response".to_owned())?
        .move_result)
}

fn request_pane_swap(
    client: &mut SocketClient,
    source_pane_id: &str,
    target_pane_id: &str,
) -> Result<SwapOutcome> {
    let result = client.request(
        "pane.swap",
        json!({
            "source_pane_id": source_pane_id,
            "target_pane_id": target_pane_id,
        }),
    )?;
    Ok(serde_json::from_value::<PaneSwapResult>(result)
        .map_err(|_| "herdr api pane.swap returned an invalid response".to_owned())?
        .swap)
}

/// Cleans up after a rebuild that stopped halfway. Whatever is still parked in the
/// scratch tab is dropped back beside the anchor, because losing sight of a terminal
/// is worse than a crooked layout, and the failure that started it all is reported
/// together with anything that could not be brought home.
fn recovered(
    client: &mut SocketClient,
    plan: &GridPlan,
    anchor: &str,
    scratch: &ScratchTab,
    error: String,
) -> String {
    let Some(scratch_tab) = &scratch.tab_id else {
        return error;
    };
    let mut stranded = Vec::new();
    for pane in &scratch.panes {
        let destination = attachment(plan.tab_id, anchor, SplitDirection::Right);
        if move_pane_to(client, pane, destination).is_err() {
            stranded.push(*pane);
        }
    }
    if plan.focus_policy == BalanceFocusPolicy::Explicit {
        let _ = client.request("pane.focus", json!({"pane_id": plan.focus_pane_id}));
    }
    if stranded.is_empty() {
        return format!("could not rebuild {}: {error}", plan.tab_id);
    }
    format!(
        "could not rebuild {}: {error}; {} left in {scratch_tab}",
        plan.tab_id,
        stranded.join(", ")
    )
}

/// Reports a failure that struck after panes had already been traded. The layout is
/// left half sorted either way, so the focus is put back where it started, best
/// effort, and the original failure is what the user hears about.
fn with_focus_restored(
    client: &mut SocketClient,
    plan: &GridPlan,
    swapped: bool,
    error: String,
) -> String {
    if !swapped {
        return error;
    }
    if plan.focus_policy == BalanceFocusPolicy::Explicit {
        let _ = client.request("pane.focus", json!({"pane_id": plan.focus_pane_id}));
    }
    format!("swapped some panes but could not finish balancing: {error}")
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

fn request_pane_split(
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
            let items = workspaces_in_visible_order(snapshot);
            adjacent(&items, context.workspace_id, direction, |item| {
                item.workspace_id.as_str()
            })
            .map(|item| item.active_tab_id.as_str())
        }
        Scope::Tab => {
            let items = tabs_in_display_order(snapshot, context.workspace_id);
            adjacent(&items, context.tab_id, direction, |item| {
                item.tab_id.as_str()
            })
            .map(|item| item.tab_id.as_str())
        }
    }
}

/// Herdr tab reorder changes the snapshot array order while keeping public tab
/// numbers stable, so the array is the displayed order rather than the numbers.
fn tabs_in_display_order<'a>(snapshot: &'a Snapshot, workspace_id: &str) -> Vec<&'a Tab> {
    snapshot
        .tabs
        .iter()
        .filter(|tab| tab.workspace_id == workspace_id)
        .collect()
}

fn workspaces_in_visible_order(snapshot: &Snapshot) -> Vec<&Workspace> {
    let mut items: Vec<_> = snapshot.workspaces.iter().collect();
    items.sort_by(|left, right| left.number.total_cmp(&right.number));
    items
}

fn current_position<T>(
    items: &[&T],
    current_id: &str,
    get_id: impl Fn(&T) -> &str,
) -> Result<usize> {
    items
        .iter()
        .position(|item| get_id(item) == current_id)
        .ok_or_else(|| format!("current item not found in herdr snapshot: {current_id}"))
}

fn adjacent<'a, T>(
    items: &[&'a T],
    current_id: &str,
    direction: MoveDirection,
    get_id: impl Fn(&T) -> &str,
) -> Result<&'a T> {
    let current = current_position(items, current_id, get_id)?;
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
        let items = tabs_in_display_order(snapshot, context.workspace_id);
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
    let items = workspaces_in_visible_order(snapshot);
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

fn read_balance_focus_state(state_dir: &Path) -> Result<Option<BalanceFocusState>> {
    let bytes = match fs::read(state_dir.join(BALANCE_FOCUS_STATE_FILE)) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.to_string()),
    };
    Ok(serde_json::from_slice(&bytes).ok())
}

fn observed_balance_focus(tab_id: &str, layout: Option<&TabLayout>) -> BalanceFocusState {
    BalanceFocusState {
        bounds: layout.and_then(|layout| layout_bounds(layout).ok()),
        pane_ids: sorted_pane_ids(layout),
        tab_id: tab_id.to_owned(),
    }
}

fn sorted_pane_ids(layout: Option<&TabLayout>) -> Vec<String> {
    let mut pane_ids: Vec<String> = layout
        .map(|layout| {
            layout
                .panes
                .iter()
                .map(|pane| pane.pane_id.clone())
                .collect()
        })
        .unwrap_or_default();
    pane_ids.sort_unstable();
    pane_ids
}

fn write_focus_anchor_state(state_dir: &Path, state: &FocusAnchorState) -> Result<()> {
    write_state_file(state_dir, FOCUS_ANCHOR_STATE_FILE, state)
}

fn write_state_file(state_dir: &Path, file_name: &str, state: &impl Serialize) -> Result<()> {
    let path = state_dir.join(file_name);
    let temporary = state_dir.join(format!(
        "{file_name}.{}-{}.tmp",
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

fn reading_order(layout: &TabLayout) -> Vec<&str> {
    let mut panes: Vec<_> = layout.panes.iter().collect();
    panes.sort_by(|left, right| compare_reading_order(left, right));
    panes
        .into_iter()
        .map(|pane| pane.pane_id.as_str())
        .collect()
}

/// How many panes each row of the target grid holds. Terminal cells are roughly
/// twice as tall as they are wide, which the ideal column count corrects for. A
/// divisor immediately below or above that ideal avoids a short final row; otherwise
/// the last row is allowed to come up short.
fn grid_row_sizes(count: usize, bounds: Bounds) -> Result<Vec<usize>> {
    if bounds.width <= 0.0 || bounds.height <= 0.0 {
        return Err("herdr reported a tab with no area; cannot balance".into());
    }
    let ideal_columns = (count as f64 * bounds.width / (bounds.height * 2.0)).sqrt();
    let lower = (ideal_columns.floor() as usize).clamp(1, count);
    let upper = (ideal_columns.ceil() as usize).clamp(1, count);
    let columns = match (count.is_multiple_of(lower), count.is_multiple_of(upper)) {
        (true, true) => {
            if ideal_columns - (lower as f64) < upper as f64 - ideal_columns {
                lower
            } else {
                upper
            }
        }
        (true, false) => lower,
        (false, true) => upper,
        (false, false) => (ideal_columns.round() as usize).clamp(1, count),
    };
    let rows = count.div_ceil(columns);
    Ok((0..rows)
        .map(|row| columns.min(count - row * columns))
        .collect())
}

/// The tab read as rows of panes: the top-level `down` run gives the rows, and the
/// `right` run inside a row gives its panes. A tab shaped any other way is not a
/// grid, which is what tells the two idempotent paths apart from a rebuild.
fn read_grid(root: &LayoutNode) -> Option<Vec<GridRow<'_>>> {
    run_slots(root, SplitDirection::Down, Vec::new())
        .into_iter()
        .map(|(node, path)| {
            let panes = run_slots(node, SplitDirection::Right, path.clone())
                .into_iter()
                .map(|(cell, _)| match cell {
                    LayoutNode::Pane { pane_id } => Some(pane_id.as_str()),
                    LayoutNode::Split { .. } => None,
                })
                .collect::<Option<Vec<_>>>()?;
            Some(GridRow { node, panes, path })
        })
        .collect()
}

/// Ratios for the whole grid: the rows share the tab evenly and the panes of a row
/// share that row evenly, closest to the root first.
fn grid_ratios(root: &LayoutNode, grid: &[GridRow]) -> Vec<(Vec<bool>, f64)> {
    ratio_updates(root, SplitDirection::Down, Vec::new())
        .into_iter()
        .chain(
            grid.iter()
                .flat_map(|row| ratio_updates(row.node, SplitDirection::Right, row.path.clone())),
        )
        .collect()
}

/// The moves that draw the target grid, each one attaching a pane to a pane already
/// back in the tab. The rows are hung off the anchor first, then each row is filled
/// in, and a parent is always attached before its children, so whatever a move needs
/// to attach to is there by the time it runs. herdr turns the pane a move names into
/// a split holding that pane and the newcomer, so attaching the second half's first
/// pane to the first half's first pane, parents before children, grows exactly the
/// tree [`join_evenly`] describes.
fn grid_moves<'a>(rows: &[&[&'a str]]) -> Vec<(&'a str, &'a str, SplitDirection)> {
    let row_anchors: Vec<_> = rows.iter().filter_map(|row| row.first().copied()).collect();
    join_evenly(&row_anchors, SplitDirection::Down)
        .into_iter()
        .chain(
            rows.iter()
                .flat_map(|row| join_evenly(row, SplitDirection::Right)),
        )
        .collect()
}

/// Joins panes into a balanced tree by hanging the second half's first pane off the
/// first half's first pane and halving from there. Halving keeps every split near an
/// even share, which matters because herdr clamps ratios to [0.1, 0.9] and a tree
/// built one pane at a time would soon ask for shares outside that.
fn join_evenly<'a>(
    panes: &[&'a str],
    direction: SplitDirection,
) -> Vec<(&'a str, &'a str, SplitDirection)> {
    if panes.len() < 2 {
        return Vec::new();
    }
    let middle = panes.len().div_ceil(2);
    [(panes[middle], panes[0], direction)]
        .into_iter()
        .chain(join_evenly(&panes[..middle], direction))
        .chain(join_evenly(&panes[middle..], direction))
        .collect()
}

/// The swaps that put the panes into reading order, keeping a local copy in step so
/// that a pane already in place is never swapped away again.
fn swaps_towards<'a>(mut placed: Vec<&'a str>, order: &[&'a str]) -> Vec<(&'a str, &'a str)> {
    let mut swaps = Vec::new();
    for (index, wanted) in order.iter().enumerate() {
        if placed[index] == *wanted {
            continue;
        }
        let Some(found) = placed.iter().position(|pane| pane == wanted) else {
            continue;
        };
        swaps.push((placed[index], *wanted));
        placed.swap(index, found);
    }
    swaps
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

/// The children that leave a run of same-direction splits, each with its path. These
/// are the slots [`slot_count`] counts.
fn run_slots(
    node: &LayoutNode,
    direction: SplitDirection,
    path: Vec<bool>,
) -> Vec<(&LayoutNode, Vec<bool>)> {
    match node {
        LayoutNode::Split {
            direction: node_direction,
            first,
            second,
        } if *node_direction == direction => run_slots(first, direction, child_path(&path, false))
            .into_iter()
            .chain(run_slots(second, direction, child_path(&path, true)))
            .collect(),
        _ => vec![(node, path)],
    }
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

#[cfg(test)]
mod tests {
    use super::{
        Bounds, LayoutPane, Rect, TabLayout, custom_label, grid_row_sizes, sorted_pane_ids,
    };

    #[test]
    fn custom_label_drops_only_the_positional_number_herdr_reports_by_default() {
        assert_eq!(custom_label("build", 0), Some("build"));
        assert_eq!(custom_label("2", 1), None);
        assert_eq!(custom_label("2", 0), Some("2"));
    }

    #[test]
    fn grid_row_sizes_cover_column_selection_variants() {
        for (count, width, height, expected) in [
            (4, 300.0, 80.0, vec![2, 2]),
            (3, 300.0, 80.0, vec![3]),
            (5, 300.0, 80.0, vec![3, 2]),
            (12, 49.0, 24.0, vec![4, 4, 4]),
            (12, 41.0, 24.0, vec![3, 3, 3, 3]),
            (4, 50.0, 200.0, vec![1, 1, 1, 1]),
            (5, 160.0, 100.0, vec![2, 2, 1]),
        ] {
            assert_eq!(
                grid_row_sizes(
                    count,
                    Bounds {
                        height,
                        width,
                        x: 0.0,
                        y: 0.0,
                    },
                )
                .unwrap(),
                expected,
            );
        }
    }

    #[test]
    fn sorted_pane_ids_order_panes_regardless_of_snapshot_order() {
        let rect = Rect {
            height: 40.0,
            width: 100.0,
            x: 0.0,
            y: 0.0,
        };
        let layout = TabLayout {
            focused_pane_id: None,
            panes: ["pane-b", "pane-a"]
                .map(|pane_id| LayoutPane {
                    pane_id: pane_id.to_owned(),
                    rect,
                })
                .into(),
            tab_id: "tab-main".to_owned(),
        };
        assert_eq!(sorted_pane_ids(Some(&layout)), ["pane-a", "pane-b"]);
        assert_eq!(sorted_pane_ids(None), Vec::<String>::new());
    }
}
