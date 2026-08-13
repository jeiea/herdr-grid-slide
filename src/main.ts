import { join } from "@std/path";

type Scope = "workspace" | "tab";
type MoveDirection = "next" | "previous";
type PaneDirection = "left" | "right" | "up" | "down";

const focusAnchorStateFileName = "focus-anchor.json";
const focusAnchorLockFileName = "focus-anchor.lock";

type Command =
  | { readonly direction: MoveDirection; readonly operation: "move"; readonly scope: Scope }
  | { readonly direction: PaneDirection; readonly operation: "focus" };

interface Context {
  readonly herdrBin: string;
  readonly paneId: string;
  readonly socketPath?: string;
  readonly stateDir?: string;
  readonly tabId: string;
  readonly workspaceId: string;
}

interface Workspace {
  readonly activeTabId: string;
  readonly id: string;
  readonly number: number;
}

interface Tab {
  readonly id: string;
  readonly number: number;
  readonly workspaceId: string;
}

interface Snapshot {
  readonly layouts: readonly TabLayout[];
  readonly tabs: readonly Tab[];
  readonly workspaces: readonly Workspace[];
}

interface TabLayout {
  readonly panes: readonly LayoutPane[];
  readonly tabId: string;
}

interface LayoutPane {
  readonly height: number;
  readonly id: string;
  readonly width: number;
  readonly x: number;
  readonly y: number;
}

interface FocusAnchorState {
  readonly paneId: string;
  readonly tabId: string;
  readonly workspaceId: string;
  readonly x: number;
  readonly y: number;
}

interface Point {
  readonly x: number;
  readonly y: number;
}

interface Bounds {
  readonly height: number;
  readonly width: number;
  readonly x: number;
  readonly y: number;
}

if (import.meta.main) {
  try {
    await main(Deno.args);
  } catch (error) {
    console.error(error instanceof Error ? error.message : String(error));
    Deno.exit(1);
  }
}

async function main(args: readonly string[]): Promise<void> {
  const command = parseArguments(args);
  const context = readContext({ requireFocus: command.operation === "focus" });

  if (command.operation === "focus") {
    await focusWithAnchor({ context, direction: command.direction });
    return;
  }

  const snapshot = await readSnapshot(context.herdrBin);
  const targetTabId = resolveMoveTargetTab({
    context,
    direction: command.direction,
    scope: command.scope,
    snapshot,
  });
  if (targetTabId === context.tabId) return;
  await runHerdr(context.herdrBin, [
    "pane",
    "move",
    context.paneId,
    "--tab",
    targetTabId,
    "--split",
    "right",
    "--ratio",
    "0.5",
    "--focus",
  ]);
}

function parseArguments(args: readonly string[]): Command {
  const [first, second, ...rest] = args;
  if (
    first === "focus" &&
    rest.length === 0 &&
    (second === "left" || second === "right" || second === "up" || second === "down")
  ) {
    return { direction: second, operation: "focus" };
  }

  if (
    rest.length > 0 ||
    (first !== "workspace" && first !== "tab") ||
    (second !== "next" && second !== "previous")
  ) {
    throw new Error("usage: herdr-move-pane <workspace|tab> <next|previous> | focus <direction>");
  }
  return { direction: second, operation: "move", scope: first };
}

function readContext(options: { readonly requireFocus: boolean }): Context {
  return {
    herdrBin: requiredEnv("HERDR_BIN_PATH"),
    paneId: requiredEnv("HERDR_PANE_ID"),
    socketPath: options.requireFocus ? requiredEnv("HERDR_SOCKET_PATH") : undefined,
    stateDir: options.requireFocus ? requiredEnv("HERDR_PLUGIN_STATE_DIR") : undefined,
    tabId: requiredEnv("HERDR_TAB_ID"),
    workspaceId: requiredEnv("HERDR_WORKSPACE_ID"),
  };
}

async function readSnapshot(herdrBin: string): Promise<Snapshot> {
  const output = await runHerdr(herdrBin, ["api", "snapshot"]);
  const response = parseJson(output, "herdr api snapshot");

  if (!isRecord(response) || !isRecord(response.result) || !isRecord(response.result.snapshot)) {
    throw new Error("herdr api snapshot response is missing result.snapshot");
  }

  const { layouts, tabs, workspaces } = response.result.snapshot;
  if (!Array.isArray(layouts) || !Array.isArray(tabs) || !Array.isArray(workspaces)) {
    throw new Error("herdr api snapshot response is missing layouts, tabs, or workspaces");
  }

  return {
    layouts: layouts.map(parseTabLayout),
    tabs: tabs.map(parseTab),
    workspaces: workspaces.map(parseWorkspace),
  };
}

async function focusWithAnchor(options: {
  readonly context: Context;
  readonly direction: PaneDirection;
}): Promise<void> {
  const { context, direction } = options;
  if (context.stateDir === undefined) throw new Error("missing HERDR_PLUGIN_STATE_DIR");

  await Deno.mkdir(context.stateDir, { recursive: true });
  const lockFile = await Deno.open(join(context.stateDir, focusAnchorLockFileName), {
    create: true,
    read: true,
    write: true,
  });
  try {
    await lockFile.lock(true);
    try {
      const snapshot = await readSnapshot(context.herdrBin);
      const state = await readFocusAnchorState(context.stateDir);
      const result = resolveFocusTarget({ context, direction, snapshot, state });
      await focusPane(context, result.pane.id);
      await writeFocusAnchorState(context.stateDir, result.state);
    } finally {
      await lockFile.unlock();
    }
  } finally {
    lockFile.close();
  }
}

function resolveMoveTargetTab(options: {
  readonly context: Context;
  readonly direction: MoveDirection;
  readonly scope: Scope;
  readonly snapshot: Snapshot;
}): string {
  const { context, direction, scope, snapshot } = options;
  const delta = direction === "next" ? 1 : -1;
  if (scope === "workspace") {
    const workspaces = snapshot.workspaces.toSorted(compareNumber);
    const workspace = adjacent(workspaces, context.workspaceId, delta, (item) => item.id);
    return workspace.activeTabId;
  }

  const tabs = snapshot.tabs
    .filter((tab) => tab.workspaceId === context.workspaceId)
    .toSorted(compareNumber);
  const tab = adjacent(tabs, context.tabId, delta, (item) => item.id);
  return tab.id;
}

function resolveFocusTarget(options: {
  readonly context: Context;
  readonly direction: PaneDirection;
  readonly snapshot: Snapshot;
  readonly state?: FocusAnchorState;
}): { readonly pane: LayoutPane; readonly state: FocusAnchorState } {
  const { context, direction, snapshot, state } = options;
  const currentLayout = findLayout(snapshot, context.tabId);
  const currentPane = findPane(currentLayout, context.paneId);
  const currentBounds = layoutBounds(currentLayout);
  const sourcePoint = resolveAnchorPoint({
    bounds: currentBounds,
    context,
    pane: currentPane,
    state,
  });
  const sourceRatio = normalizePoint(currentBounds, sourcePoint);
  const withinTarget = resolveWithinLayout({
    direction,
    layout: currentLayout,
    pane: currentPane,
    point: sourcePoint,
  });
  const target = withinTarget ?? resolveBoundaryTarget({
    context,
    direction,
    pointRatio: sourceRatio,
    snapshot,
  });
  const targetBounds = layoutBounds(target.layout);
  const targetPoint = withinTarget === undefined
    ? denormalizePoint(targetBounds, sourceRatio)
    : sourcePoint;
  const nextPoint = moveAnchorPoint({
    direction,
    point: targetPoint,
    target: target.pane,
  });

  return {
    pane: target.pane,
    state: {
      paneId: target.pane.id,
      tabId: target.layout.tabId,
      workspaceId: workspaceIdForTab(snapshot, target.layout.tabId),
      ...normalizePoint(targetBounds, nextPoint),
    },
  };
}

function resolveAnchorPoint(options: {
  readonly bounds: Bounds;
  readonly context: Context;
  readonly pane: LayoutPane;
  readonly state?: FocusAnchorState;
}): Point {
  const { bounds, context, pane, state } = options;
  if (
    state !== undefined &&
    state.paneId === context.paneId &&
    state.tabId === context.tabId &&
    state.workspaceId === context.workspaceId
  ) {
    return clampPointToPane(denormalizePoint(bounds, state), pane);
  }
  return paneCenter(pane);
}

function resolveWithinLayout(options: {
  readonly direction: PaneDirection;
  readonly layout: TabLayout;
  readonly pane: LayoutPane;
  readonly point: Point;
}): { readonly layout: TabLayout; readonly pane: LayoutPane } | undefined {
  const { direction, layout, pane, point } = options;
  const target = layout.panes
    .filter((candidate) => candidate.id !== pane.id)
    .filter((candidate) => isPaneInDirection({ candidate, direction, pane }))
    .filter((candidate) => crossAxisOverlaps(candidate, pane, direction))
    .toSorted((left, right) => {
      const leftScore = candidateScore({ candidate: left, direction, pane, point });
      const rightScore = candidateScore({ candidate: right, direction, pane, point });
      return leftScore.primaryDistance - rightScore.primaryDistance ||
        leftScore.anchorDistance - rightScore.anchorDistance ||
        leftScore.centerDistance - rightScore.centerDistance ||
        compareReadingOrder(left, right);
    })[0];
  return target === undefined ? undefined : { layout, pane: target };
}

function resolveBoundaryTarget(options: {
  readonly context: Context;
  readonly direction: PaneDirection;
  readonly pointRatio: Point;
  readonly snapshot: Snapshot;
}): { readonly layout: TabLayout; readonly pane: LayoutPane } {
  const { context, direction, pointRatio, snapshot } = options;
  const targetTabId = resolveBoundaryTab({ context, direction, snapshot });
  const layout = findLayout(snapshot, targetTabId);
  const bounds = layoutBounds(layout);
  const point = denormalizePoint(bounds, pointRatio);
  const target = entryEdgePanes(layout, direction)
    .toSorted((left, right) => {
      return axisDistance(left, direction, point) - axisDistance(right, direction, point) ||
        axisCenterDistance(left, direction, point) -
          axisCenterDistance(right, direction, point) ||
        compareBoundaryReadingOrder(left, right, direction);
    })[0];
  if (target === undefined) {
    throw new Error(`target tab contains no pane in herdr layout: ${targetTabId}`);
  }
  return { layout, pane: target };
}

function resolveBoundaryTab(options: {
  readonly context: Context;
  readonly direction: PaneDirection;
  readonly snapshot: Snapshot;
}): string {
  const { context, direction, snapshot } = options;
  if (direction === "left" || direction === "right") {
    const tabs = snapshot.tabs
      .filter((tab) => tab.workspaceId === context.workspaceId)
      .toSorted(compareNumber);
    const tab = adjacent(
      tabs,
      context.tabId,
      direction === "right" ? 1 : -1,
      (item) => item.id,
    );
    return tab.id;
  }

  const workspaces = snapshot.workspaces.toSorted(compareNumber);
  const workspace = adjacent(
    workspaces,
    context.workspaceId,
    direction === "down" ? 1 : -1,
    (item) => item.id,
  );
  return workspace.activeTabId;
}

async function readFocusAnchorState(stateDir: string): Promise<FocusAnchorState | undefined> {
  let text: string;
  try {
    text = await Deno.readTextFile(join(stateDir, focusAnchorStateFileName));
  } catch (error) {
    if (error instanceof Deno.errors.NotFound) return undefined;
    throw error;
  }
  let value: unknown;
  try {
    value = JSON.parse(text);
  } catch {
    return undefined;
  }
  if (
    !isRecord(value) ||
    typeof value.paneId !== "string" ||
    typeof value.tabId !== "string" ||
    typeof value.workspaceId !== "string" ||
    !isUnitInterval(value.x) ||
    !isUnitInterval(value.y)
  ) {
    return undefined;
  }
  return value as unknown as FocusAnchorState;
}

async function writeFocusAnchorState(stateDir: string, state: FocusAnchorState): Promise<void> {
  const path = join(stateDir, focusAnchorStateFileName);
  const temporaryPath = join(stateDir, `${focusAnchorStateFileName}.${crypto.randomUUID()}.tmp`);
  try {
    await Deno.writeTextFile(temporaryPath, JSON.stringify(state));
    await Deno.rename(temporaryPath, path);
  } finally {
    await removeIfPresent(temporaryPath);
  }
}

async function focusPane(context: Context, paneId: string): Promise<void> {
  if (context.socketPath === undefined) {
    throw new Error("missing HERDR_SOCKET_PATH");
  }
  const connection = await Deno.connect({ path: context.socketPath, transport: "unix" });
  try {
    const id = `jeiea.move-pane:${crypto.randomUUID()}`;
    await writeAll(
      connection,
      new TextEncoder().encode(`${
        JSON.stringify({
          id,
          method: "pane.focus",
          params: { pane_id: paneId },
        })
      }\n`),
    );
    const response = parseJson(await readLine(connection), "pane.focus socket response");
    if (!isRecord(response) || response.id !== id) {
      throw new Error("pane.focus socket response has an unexpected id");
    }
    if (isRecord(response.error)) {
      const detail = typeof response.error.message === "string"
        ? `: ${response.error.message}`
        : "";
      throw new Error(`pane.focus failed${detail}`);
    }
  } finally {
    connection.close();
  }
}

function adjacent<T>(
  items: readonly T[],
  currentId: string,
  delta: 1 | -1,
  getId: (item: T) => string,
): T {
  const currentIndex = items.findIndex((item) => getId(item) === currentId);
  if (currentIndex === -1) {
    throw new Error(`current item not found in herdr snapshot: ${currentId}`);
  }
  const targetIndex = (currentIndex + delta + items.length) % items.length;
  const target = items[targetIndex];
  if (target === undefined) {
    throw new Error("herdr snapshot contains no navigation target");
  }
  return target;
}

async function runHerdr(herdrBin: string, args: readonly string[]): Promise<string> {
  const output = await new Deno.Command(herdrBin, {
    args: [...args],
    stderr: "piped",
    stdout: "piped",
  }).output();
  if (!output.success) {
    const detail = new TextDecoder().decode(output.stderr).trim();
    throw new Error(`herdr ${args.join(" ")} failed${detail === "" ? "" : `: ${detail}`}`);
  }
  return new TextDecoder().decode(output.stdout);
}

function parseJson(text: string, source: string): unknown {
  try {
    return JSON.parse(text);
  } catch {
    throw new Error(`${source} returned invalid JSON`);
  }
}

function parseLayoutPane(value: unknown): LayoutPane {
  if (
    !isRecord(value) ||
    typeof value.pane_id !== "string" ||
    !isRecord(value.rect) ||
    !isNumber(value.rect.height) ||
    !isNumber(value.rect.width) ||
    !isNumber(value.rect.x) ||
    !isNumber(value.rect.y)
  ) {
    throw new Error("herdr layout contains an invalid pane");
  }
  return {
    height: value.rect.height,
    id: value.pane_id,
    width: value.rect.width,
    x: value.rect.x,
    y: value.rect.y,
  };
}

function parseTabLayout(value: unknown): TabLayout {
  if (!isRecord(value) || typeof value.tab_id !== "string" || !Array.isArray(value.panes)) {
    throw new Error("herdr snapshot contains an invalid tab layout");
  }
  return { panes: value.panes.map(parseLayoutPane), tabId: value.tab_id };
}

function parseWorkspace(value: unknown): Workspace {
  if (
    !isRecord(value) ||
    typeof value.workspace_id !== "string" ||
    typeof value.active_tab_id !== "string" ||
    !isNumber(value.number)
  ) {
    throw new Error("herdr snapshot contains an invalid workspace");
  }
  return {
    activeTabId: value.active_tab_id,
    id: value.workspace_id,
    number: value.number,
  };
}

function parseTab(value: unknown): Tab {
  if (
    !isRecord(value) ||
    typeof value.tab_id !== "string" ||
    typeof value.workspace_id !== "string" ||
    !isNumber(value.number)
  ) {
    throw new Error("herdr snapshot contains an invalid tab");
  }
  return {
    id: value.tab_id,
    number: value.number,
    workspaceId: value.workspace_id,
  };
}

function compareNumber(
  left: { readonly number: number },
  right: { readonly number: number },
): number {
  return left.number - right.number;
}

function compareReadingOrder(left: LayoutPane, right: LayoutPane): number {
  return left.y - right.y || left.x - right.x || left.id.localeCompare(right.id);
}

function compareBoundaryReadingOrder(
  left: LayoutPane,
  right: LayoutPane,
  direction: PaneDirection,
): number {
  const order = compareReadingOrder(left, right);
  return direction === "left" || direction === "up" ? -order : order;
}

function findLayout(snapshot: Snapshot, tabId: string): TabLayout {
  const layout = snapshot.layouts.find((candidate) => candidate.tabId === tabId);
  if (layout === undefined) throw new Error(`target tab has no layout: ${tabId}`);
  return layout;
}

function findPane(layout: TabLayout, paneId: string): LayoutPane {
  const pane = layout.panes.find((candidate) => candidate.id === paneId);
  if (pane === undefined) {
    throw new Error(`current pane not found in herdr layout: ${paneId}`);
  }
  return pane;
}

function layoutBounds(layout: TabLayout): Bounds {
  const [first, ...rest] = layout.panes;
  if (first === undefined) {
    throw new Error(`target tab contains no pane in herdr layout: ${layout.tabId}`);
  }
  const bounds = rest.reduce(
    (current, pane) => ({
      maxX: Math.max(current.maxX, pane.x + pane.width),
      maxY: Math.max(current.maxY, pane.y + pane.height),
      minX: Math.min(current.minX, pane.x),
      minY: Math.min(current.minY, pane.y),
    }),
    {
      maxX: first.x + first.width,
      maxY: first.y + first.height,
      minX: first.x,
      minY: first.y,
    },
  );
  return {
    height: bounds.maxY - bounds.minY,
    width: bounds.maxX - bounds.minX,
    x: bounds.minX,
    y: bounds.minY,
  };
}

function workspaceIdForTab(snapshot: Snapshot, tabId: string): string {
  const tab = snapshot.tabs.find((candidate) => candidate.id === tabId);
  if (tab === undefined) {
    throw new Error(`target tab not found in herdr snapshot: ${tabId}`);
  }
  return tab.workspaceId;
}

function isPaneInDirection(options: {
  readonly candidate: LayoutPane;
  readonly direction: PaneDirection;
  readonly pane: LayoutPane;
}): boolean {
  const { candidate, direction, pane } = options;
  switch (direction) {
    case "left":
      return candidate.x + candidate.width <= pane.x;
    case "right":
      return candidate.x >= pane.x + pane.width;
    case "up":
      return candidate.y + candidate.height <= pane.y;
    case "down":
      return candidate.y >= pane.y + pane.height;
  }
}

function candidateScore(options: {
  readonly candidate: LayoutPane;
  readonly direction: PaneDirection;
  readonly pane: LayoutPane;
  readonly point: Point;
}): {
  readonly anchorDistance: number;
  readonly centerDistance: number;
  readonly primaryDistance: number;
} {
  const { candidate, direction, pane, point } = options;
  const primaryDistance = (() => {
    switch (direction) {
      case "left":
        return pane.x - (candidate.x + candidate.width);
      case "right":
        return candidate.x - (pane.x + pane.width);
      case "up":
        return pane.y - (candidate.y + candidate.height);
      case "down":
        return candidate.y - (pane.y + pane.height);
    }
  })();
  return {
    anchorDistance: axisDistance(candidate, direction, point),
    centerDistance: axisCenterDistance(candidate, direction, point),
    primaryDistance,
  };
}

function crossAxisOverlaps(
  left: LayoutPane,
  right: LayoutPane,
  direction: PaneDirection,
): boolean {
  if (direction === "left" || direction === "right") {
    return intervalsOverlap(left.y, left.y + left.height, right.y, right.y + right.height);
  }
  return intervalsOverlap(left.x, left.x + left.width, right.x, right.x + right.width);
}

function intervalsOverlap(
  leftStart: number,
  leftEnd: number,
  rightStart: number,
  rightEnd: number,
): boolean {
  return Math.max(leftStart, rightStart) < Math.min(leftEnd, rightEnd);
}

function axisDistance(pane: LayoutPane, direction: PaneDirection, point: Point): number {
  if (direction === "left" || direction === "right") {
    return intervalDistance(point.y, pane.y, pane.y + pane.height);
  }
  return intervalDistance(point.x, pane.x, pane.x + pane.width);
}

function axisCenterDistance(
  pane: LayoutPane,
  direction: PaneDirection,
  point: Point,
): number {
  if (direction === "left" || direction === "right") {
    return Math.abs(point.y - paneCenter(pane).y);
  }
  return Math.abs(point.x - paneCenter(pane).x);
}

function intervalDistance(value: number, start: number, end: number): number {
  if (value < start) return start - value;
  if (value > end) return value - end;
  return 0;
}

function entryEdgePanes(layout: TabLayout, direction: PaneDirection): readonly LayoutPane[] {
  const edge = (() => {
    switch (direction) {
      case "left":
        return Math.max(...layout.panes.map((pane) => pane.x + pane.width));
      case "right":
        return Math.min(...layout.panes.map((pane) => pane.x));
      case "up":
        return Math.max(...layout.panes.map((pane) => pane.y + pane.height));
      case "down":
        return Math.min(...layout.panes.map((pane) => pane.y));
    }
  })();
  return layout.panes.filter((pane) => {
    switch (direction) {
      case "left":
        return pane.x + pane.width === edge;
      case "right":
        return pane.x === edge;
      case "up":
        return pane.y + pane.height === edge;
      case "down":
        return pane.y === edge;
    }
  });
}

function moveAnchorPoint(options: {
  readonly direction: PaneDirection;
  readonly point: Point;
  readonly target: LayoutPane;
}): Point {
  const { direction, point, target } = options;
  if (direction === "left" || direction === "right") {
    return {
      x: paneCenter(target).x,
      y: clamp(point.y, target.y, target.y + target.height),
    };
  }
  return {
    x: clamp(point.x, target.x, target.x + target.width),
    y: paneCenter(target).y,
  };
}

function paneCenter(pane: LayoutPane): Point {
  return {
    x: pane.x + pane.width / 2,
    y: pane.y + pane.height / 2,
  };
}

function clampPointToPane(point: Point, pane: LayoutPane): Point {
  return {
    x: clamp(point.x, pane.x, pane.x + pane.width),
    y: clamp(point.y, pane.y, pane.y + pane.height),
  };
}

function normalizePoint(bounds: Bounds, point: Point): Point {
  return {
    x: bounds.width === 0 ? 0 : clamp((point.x - bounds.x) / bounds.width, 0, 1),
    y: bounds.height === 0 ? 0 : clamp((point.y - bounds.y) / bounds.height, 0, 1),
  };
}

function denormalizePoint(bounds: Bounds, point: Point): Point {
  return {
    x: bounds.x + point.x * bounds.width,
    y: bounds.y + point.y * bounds.height,
  };
}

function clamp(value: number, minimum: number, maximum: number): number {
  return Math.min(Math.max(value, minimum), maximum);
}

async function removeIfPresent(path: string): Promise<void> {
  try {
    await Deno.remove(path);
  } catch (error) {
    if (!(error instanceof Deno.errors.NotFound)) throw error;
  }
}

async function writeAll(connection: Deno.Conn, bytes: Uint8Array): Promise<void> {
  let offset = 0;
  while (offset < bytes.length) {
    offset += await connection.write(bytes.subarray(offset));
  }
}

async function readLine(connection: Deno.Conn): Promise<string> {
  const decoder = new TextDecoder();
  const buffer = new Uint8Array(8_192);
  let text = "";
  while (!text.includes("\n")) {
    const size = await connection.read(buffer);
    if (size === null) throw new Error("herdr socket closed before returning a response");
    text += decoder.decode(buffer.subarray(0, size), { stream: true });
  }
  return text.slice(0, text.indexOf("\n"));
}

function requiredEnv(name: string): string {
  const value = Deno.env.get(name);
  if (value === undefined || value === "") {
    throw new Error(`missing ${name}`);
  }
  return value;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}

function isNumber(value: unknown): value is number {
  return typeof value === "number" && Number.isFinite(value);
}

function isUnitInterval(value: unknown): value is number {
  return isNumber(value) && value >= 0 && value <= 1;
}
