type Scope = "workspace" | "tab";
type Direction = "next" | "previous";

interface Context {
  readonly herdrBin: string;
  readonly paneId: string;
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
  readonly tabs: readonly Tab[];
  readonly workspaces: readonly Workspace[];
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
  const { direction, scope } = parseArguments(args);
  const context = readContext();
  const snapshot = await readSnapshot(context.herdrBin);
  const targetTabId = resolveTargetTab(snapshot, context, scope, direction);

  if (targetTabId === context.tabId) {
    return;
  }

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

function parseArguments(args: readonly string[]): { direction: Direction; scope: Scope } {
  const [scope, direction, ...rest] = args;
  if (
    rest.length > 0 ||
    (scope !== "workspace" && scope !== "tab") ||
    (direction !== "next" && direction !== "previous")
  ) {
    throw new Error("usage: herdr-move-pane <workspace|tab> <next|previous>");
  }
  return { direction, scope };
}

function readContext(): Context {
  return {
    herdrBin: requiredEnv("HERDR_BIN_PATH"),
    paneId: requiredEnv("HERDR_PANE_ID"),
    tabId: requiredEnv("HERDR_TAB_ID"),
    workspaceId: requiredEnv("HERDR_WORKSPACE_ID"),
  };
}

async function readSnapshot(herdrBin: string): Promise<Snapshot> {
  const output = await runHerdr(herdrBin, ["api", "snapshot"]);
  let response: unknown;
  try {
    response = JSON.parse(output);
  } catch {
    throw new Error("herdr api snapshot returned invalid JSON");
  }

  if (!isRecord(response) || !isRecord(response.result) || !isRecord(response.result.snapshot)) {
    throw new Error("herdr api snapshot response is missing result.snapshot");
  }

  const { tabs, workspaces } = response.result.snapshot;
  if (!Array.isArray(tabs) || !Array.isArray(workspaces)) {
    throw new Error("herdr api snapshot response is missing tabs or workspaces");
  }

  return {
    tabs: tabs.map(parseTab),
    workspaces: workspaces.map(parseWorkspace),
  };
}

function resolveTargetTab(
  snapshot: Snapshot,
  context: Context,
  scope: Scope,
  direction: Direction,
): string {
  const delta = direction === "next" ? 1 : -1;
  if (scope === "workspace") {
    const workspaces = snapshot.workspaces.toSorted(compareNumber);
    return adjacent(workspaces, context.workspaceId, delta, (workspace) => workspace.id)
      .activeTabId;
  }

  const tabs = snapshot.tabs
    .filter((tab) => tab.workspaceId === context.workspaceId)
    .toSorted(compareNumber);
  return adjacent(tabs, context.tabId, delta, (tab) => tab.id).id;
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
