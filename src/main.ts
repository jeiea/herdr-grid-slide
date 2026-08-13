type Scope = "workspace" | "tab";
type MoveDirection = "next" | "previous";
type PaneDirection = "left" | "right" | "up" | "down";

type Command =
  | { readonly direction: MoveDirection; readonly operation: "move"; readonly scope: Scope }
  | { readonly direction: PaneDirection; readonly operation: "focus" };

interface Context {
  readonly herdrBin: string;
  readonly paneId: string;
  readonly socketPath?: string;
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
  readonly id: string;
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
  const context = readContext({ requireSocket: command.operation === "focus" });

  if (command.operation === "focus") {
    if (await focusDirection(context, command.direction)) return;
    const snapshot = await readSnapshot(context.herdrBin);
    await focusBoundary({ context, direction: command.direction, snapshot });
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

function readContext(options: { readonly requireSocket: boolean }): Context {
  return {
    herdrBin: requiredEnv("HERDR_BIN_PATH"),
    paneId: requiredEnv("HERDR_PANE_ID"),
    socketPath: options.requireSocket ? requiredEnv("HERDR_SOCKET_PATH") : undefined,
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

async function focusDirection(context: Context, direction: PaneDirection): Promise<boolean> {
  const response = parseJson(
    await runHerdr(context.herdrBin, [
      "pane",
      "focus",
      "--direction",
      direction,
      "--pane",
      context.paneId,
    ]),
    "herdr pane focus",
  );
  const focus = isRecord(response) && isRecord(response.result) ? response.result.focus : undefined;
  if (!isRecord(focus) || typeof focus.changed !== "boolean") {
    throw new Error("herdr pane focus response is missing result.focus.changed");
  }
  if (!focus.changed && focus.reason !== "no_neighbor") {
    throw new Error("herdr pane focus did not change focus for an unexpected reason");
  }
  return focus.changed;
}

async function focusBoundary(options: {
  readonly context: Context;
  readonly direction: PaneDirection;
  readonly snapshot: Snapshot;
}): Promise<void> {
  const { context, direction, snapshot } = options;
  const targetTabId = resolveBoundaryTab({ context, direction, snapshot });
  const layout = snapshot.layouts.find((candidate) => candidate.tabId === targetTabId);
  if (layout === undefined) throw new Error(`target tab has no layout: ${targetTabId}`);
  const panes = layout.panes.toSorted(compareReadingOrder);
  const targetPane = direction === "left" || direction === "up" ? panes.at(-1) : panes[0];
  if (targetPane === undefined) {
    throw new Error(`target tab contains no pane in herdr layout: ${targetTabId}`);
  }
  await focusPane(context, targetPane.id);
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
    !isNumber(value.rect.x) ||
    !isNumber(value.rect.y)
  ) {
    throw new Error("herdr layout contains an invalid pane");
  }
  return { id: value.pane_id, x: value.rect.x, y: value.rect.y };
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
