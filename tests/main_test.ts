import { assertEquals } from "@std/assert";
import { dirname, fromFileUrl, join } from "@std/path";

const root = dirname(dirname(fromFileUrl(import.meta.url)));
const expectedMovePrefix = ["pane", "move", "pane-current", "--tab"];
const expectedMoveSuffix = ["--split", "right", "--ratio", "0.5", "--focus"];

const workspaces = [
  { workspace_id: "workspace-3", active_tab_id: "tab-3-1", number: 3 },
  { workspace_id: "workspace-1", active_tab_id: "tab-1-1", number: 1 },
  { workspace_id: "workspace-2", active_tab_id: "tab-2-2", number: 2 },
];
const tabs = [
  { workspace_id: "workspace-2", tab_id: "tab-2-3", number: 3 },
  { workspace_id: "workspace-1", tab_id: "tab-1-1", number: 1 },
  { workspace_id: "workspace-2", tab_id: "tab-2-1", number: 1 },
  { workspace_id: "workspace-3", tab_id: "tab-3-1", number: 1 },
  { workspace_id: "workspace-2", tab_id: "tab-2-2", number: 2 },
];
const layoutsByTab = {
  "tab-1-1": paneLayout("tab-1-1", "workspace-1", "pane-1"),
  "tab-2-1": paneLayout("tab-2-1", "workspace-2", "pane-2-1"),
  "tab-2-2": paneLayout("tab-2-2", "workspace-2", "pane-2-2"),
  "tab-2-3": paneLayout("tab-2-3", "workspace-2", "pane-2-3"),
  "tab-3-1": paneLayout("tab-3-1", "workspace-3", "pane-3"),
};
const layouts = Object.values(layoutsByTab);
const anchorLayouts = [
  {
    panes: [
      {
        pane_id: "pane-a",
        rect: { height: 100, width: 60, x: 0, y: 0 },
      },
      {
        pane_id: "pane-b",
        rect: { height: 50, width: 40, x: 60, y: 0 },
      },
      {
        pane_id: "pane-c",
        rect: { height: 50, width: 40, x: 60, y: 50 },
      },
    ],
    tab_id: "tab-anchor",
    workspace_id: "workspace-1",
  },
];
const anchorTabs = [{ workspace_id: "workspace-1", tab_id: "tab-anchor", number: 1 }];
const anchorWorkspaces = [
  { workspace_id: "workspace-1", active_tab_id: "tab-anchor", number: 1 },
];

for (
  const scenario of [
    {
      name: "next workspace wraps by workspace number",
      scope: "workspace",
      direction: "next",
      workspaceId: "workspace-3",
      tabId: "tab-3-1",
      targetTabId: "tab-1-1",
    },
    {
      name: "previous workspace wraps by workspace number",
      scope: "workspace",
      direction: "previous",
      workspaceId: "workspace-1",
      tabId: "tab-1-1",
      targetTabId: "tab-3-1",
    },
    {
      name: "next tab wraps within the current workspace by tab number",
      scope: "tab",
      direction: "next",
      workspaceId: "workspace-2",
      tabId: "tab-2-3",
      targetTabId: "tab-2-1",
    },
    {
      name: "previous tab wraps within the current workspace by tab number",
      scope: "tab",
      direction: "previous",
      workspaceId: "workspace-2",
      tabId: "tab-2-1",
      targetTabId: "tab-2-3",
    },
  ] as const
) {
  Deno.test(scenario.name, async () => {
    const integration = await setupIntegration({
      workspaceId: scenario.workspaceId,
      tabId: scenario.tabId,
    });

    try {
      const result = await integration.run(scenario.scope, scenario.direction);

      assertEquals(result.code, 0, result.stderr);
      assertEquals(await integration.herdrCall(), [
        ...expectedMovePrefix,
        scenario.targetTabId,
        ...expectedMoveSuffix,
      ]);
    } finally {
      await integration.dispose();
    }
  });
}

for (
  const scenario of [
    {
      name: "focuses the left geometric neighbor within the current tab",
      direction: "left",
      neighborPaneId: "pane-neighbor-left",
      expectedPaneId: "pane-neighbor-left",
    },
    {
      name: "focuses the right geometric neighbor within the current tab",
      direction: "right",
      neighborPaneId: "pane-neighbor-right",
      expectedPaneId: "pane-neighbor-right",
    },
    {
      name: "focuses the upper geometric neighbor within the current tab",
      direction: "up",
      neighborPaneId: "pane-neighbor-up",
      expectedPaneId: "pane-neighbor-up",
    },
    {
      name: "focuses the lower geometric neighbor within the current tab",
      direction: "down",
      neighborPaneId: "pane-neighbor-down",
      expectedPaneId: "pane-neighbor-down",
    },
  ] as const
) {
  Deno.test(scenario.name, async () => {
    const integration = await setupIntegration({
      layouts: [directionLayout(scenario.direction, scenario.neighborPaneId)],
      workspaceId: "workspace-2",
      tabId: "tab-2-2",
    });

    try {
      const result = await integration.run("focus", scenario.direction);

      assertEquals(result.code, 0, result.stderr);
      assertEquals(await integration.herdrCall(), ["pane", "focus", scenario.expectedPaneId]);
    } finally {
      await integration.dispose();
    }
  });
}

for (
  const scenario of [
    {
      name: "left edge wraps to the previous tab's last pane in reading order",
      direction: "left",
      workspaceId: "workspace-2",
      tabId: "tab-2-1",
      expectedPaneId: "pane-2-3-bottom-right",
    },
    {
      name: "right edge wraps to the next tab's first pane in reading order",
      direction: "right",
      workspaceId: "workspace-2",
      tabId: "tab-2-3",
      expectedPaneId: "pane-2-1-top-left",
    },
    {
      name: "upper edge wraps to the previous workspace active tab's last pane",
      direction: "up",
      workspaceId: "workspace-1",
      tabId: "tab-1-1",
      expectedPaneId: "pane-3-bottom-right",
    },
    {
      name: "lower edge wraps to the next workspace active tab's first pane",
      direction: "down",
      workspaceId: "workspace-3",
      tabId: "tab-3-1",
      expectedPaneId: "pane-1-top-left",
    },
  ] as const
) {
  Deno.test(scenario.name, async () => {
    const integration = await setupIntegration({
      layouts: layouts.map((layout) =>
        layout.tab_id === scenario.tabId
          ? singlePaneLayout(scenario.tabId, scenario.workspaceId, "pane-current")
          : layout
      ),
      workspaceId: scenario.workspaceId,
      tabId: scenario.tabId,
    });

    try {
      const result = await integration.run("focus", scenario.direction);

      assertEquals(result.code, 0, result.stderr);
      assertEquals(await integration.herdrCall(), ["pane", "focus", scenario.expectedPaneId]);
    } finally {
      await integration.dispose();
    }
  });
}

Deno.test("focus remembers the cross-axis anchor across panes", async () => {
  const integration = await setupIntegration({
    layouts: anchorLayouts,
    paneId: "pane-a",
    tabs: anchorTabs,
    tabId: "tab-anchor",
    workspaces: anchorWorkspaces,
    workspaceId: "workspace-1",
  });

  try {
    let result = await integration.runWithPane("pane-a", "focus", "right");
    assertEquals(result.code, 0, result.stderr);
    assertEquals(await integration.herdrCall(), ["pane", "focus", "pane-b"]);

    result = await integration.runWithPane("pane-b", "focus", "down");
    assertEquals(result.code, 0, result.stderr);
    assertEquals(await integration.herdrCall(), ["pane", "focus", "pane-c"]);

    result = await integration.runWithPane("pane-c", "focus", "left");
    assertEquals(result.code, 0, result.stderr);
    assertEquals(await integration.herdrCall(), ["pane", "focus", "pane-a"]);

    result = await integration.runWithPane("pane-a", "focus", "right");
    assertEquals(result.code, 0, result.stderr);
    assertEquals(await integration.herdrCall(), ["pane", "focus", "pane-c"]);
  } finally {
    await integration.dispose();
  }
});

Deno.test("focus resets a stale anchor to the current pane center", async () => {
  const integration = await setupIntegration({
    layouts: anchorLayouts,
    paneId: "pane-a",
    tabs: anchorTabs,
    tabId: "tab-anchor",
    workspaces: anchorWorkspaces,
    workspaceId: "workspace-1",
  });

  try {
    await integration.writeState({
      paneId: "pane-stale",
      tabId: "tab-anchor",
      workspaceId: "workspace-1",
      x: 0.3,
      y: 0.9,
    });

    const result = await integration.run("focus", "right");

    assertEquals(result.code, 0, result.stderr);
    assertEquals(await integration.herdrCall(), ["pane", "focus", "pane-b"]);
  } finally {
    await integration.dispose();
  }
});

Deno.test("focus recovers from malformed anchor state", async () => {
  const integration = await setupIntegration({
    layouts: anchorLayouts,
    paneId: "pane-a",
    tabs: anchorTabs,
    tabId: "tab-anchor",
    workspaces: anchorWorkspaces,
    workspaceId: "workspace-1",
  });

  try {
    await integration.writeRawState("not json");

    const result = await integration.run("focus", "right");

    assertEquals(result.code, 0, result.stderr);
    assertEquals(await integration.herdrCall(), ["pane", "focus", "pane-b"]);
  } finally {
    await integration.dispose();
  }
});

Deno.test("focus preserves the cross-axis anchor when wrapping across tabs", async () => {
  const boundaryLayouts = [
    {
      panes: [
        { pane_id: "pane-source", rect: { height: 200, width: 100, x: 0, y: 0 } },
      ],
      tab_id: "tab-source",
      workspace_id: "workspace-1",
    },
    {
      panes: [
        { pane_id: "pane-top", rect: { height: 50, width: 100, x: 0, y: 0 } },
        { pane_id: "pane-bottom", rect: { height: 50, width: 100, x: 0, y: 50 } },
      ],
      tab_id: "tab-target",
      workspace_id: "workspace-1",
    },
  ];
  const integration = await setupIntegration({
    layouts: boundaryLayouts,
    paneId: "pane-source",
    tabs: [
      { workspace_id: "workspace-1", tab_id: "tab-source", number: 1 },
      { workspace_id: "workspace-1", tab_id: "tab-target", number: 2 },
    ],
    tabId: "tab-source",
    workspaces: [
      { workspace_id: "workspace-1", active_tab_id: "tab-source", number: 1 },
    ],
    workspaceId: "workspace-1",
  });

  try {
    await integration.writeState({
      paneId: "pane-source",
      tabId: "tab-source",
      workspaceId: "workspace-1",
      x: 0.5,
      y: 0.75,
    });

    const result = await integration.run("focus", "right");

    assertEquals(result.code, 0, result.stderr);
    assertEquals(await integration.herdrCall(), ["pane", "focus", "pane-bottom"]);
  } finally {
    await integration.dispose();
  }
});

Deno.test("focus preserves the cross-axis anchor when wrapping across workspaces", async () => {
  const boundaryLayouts = [
    {
      panes: [
        { pane_id: "pane-source", rect: { height: 100, width: 200, x: 0, y: 0 } },
      ],
      tab_id: "tab-source",
      workspace_id: "workspace-source",
    },
    {
      panes: [
        { pane_id: "pane-left", rect: { height: 100, width: 50, x: 0, y: 0 } },
        { pane_id: "pane-right", rect: { height: 100, width: 50, x: 50, y: 0 } },
      ],
      tab_id: "tab-target",
      workspace_id: "workspace-target",
    },
  ];
  const integration = await setupIntegration({
    layouts: boundaryLayouts,
    paneId: "pane-source",
    tabs: [
      { workspace_id: "workspace-source", tab_id: "tab-source", number: 1 },
      { workspace_id: "workspace-target", tab_id: "tab-target", number: 1 },
    ],
    tabId: "tab-source",
    workspaces: [
      { workspace_id: "workspace-source", active_tab_id: "tab-source", number: 1 },
      { workspace_id: "workspace-target", active_tab_id: "tab-target", number: 2 },
    ],
    workspaceId: "workspace-source",
  });

  try {
    await integration.writeState({
      paneId: "pane-source",
      tabId: "tab-source",
      workspaceId: "workspace-source",
      x: 0.75,
      y: 0.5,
    });

    const result = await integration.run("focus", "down");

    assertEquals(result.code, 0, result.stderr);
    assertEquals(await integration.herdrCall(), ["pane", "focus", "pane-right"]);
  } finally {
    await integration.dispose();
  }
});

interface IntegrationOptions {
  readonly layouts?: typeof layouts;
  readonly paneId?: string;
  readonly tabs?: typeof tabs;
  readonly workspaceId: string;
  readonly tabId: string;
  readonly workspaces?: typeof workspaces;
}

async function setupIntegration(options: IntegrationOptions) {
  const directory = await Deno.makeTempDir({ prefix: "herdr-move-pane-test-" });
  const fakeHerdrPath = join(directory, "herdr");
  const callsPath = join(directory, "calls.json");
  const socketPath = join(directory, "herdr.sock");
  const statePath = join(directory, "state");
  const fakeSource = await Deno.readTextFile(join(root, "tests", "fake_herdr.ts"));
  await Deno.writeTextFile(
    fakeHerdrPath,
    `#!/usr/bin/env -S deno run --quiet --allow-env --allow-write\n${fakeSource}`,
  );
  await Deno.chmod(fakeHerdrPath, 0o755);

  return {
    async run(...args: readonly string[]) {
      return await this.runWithPane(options.paneId ?? "pane-current", ...args);
    },
    async runWithPane(paneId: string, ...args: readonly string[]) {
      const focusServer = args[0] === "focus" ? startFocusServer(socketPath, callsPath) : undefined;
      const command = new Deno.Command(Deno.execPath(), {
        args: [
          "run",
          "--quiet",
          "--allow-env=HERDR_BIN_PATH,HERDR_SOCKET_PATH,HERDR_WORKSPACE_ID,HERDR_TAB_ID,HERDR_PANE_ID,HERDR_PLUGIN_STATE_DIR",
          "--allow-net",
          "--allow-read",
          "--allow-run",
          "--allow-write",
          "src/main.ts",
          ...args,
        ],
        cwd: root,
        env: {
          FAKE_HERDR_CALLS: callsPath,
          FAKE_HERDR_EXPECTED_DIRECTION: "",
          FAKE_HERDR_NEIGHBOR_PANE_ID: "",
          FAKE_HERDR_SNAPSHOT: JSON.stringify({
            result: {
              snapshot: {
                layouts: options.layouts ?? layouts,
                tabs: options.tabs ?? tabs,
                workspaces: options.workspaces ?? workspaces,
              },
            },
          }),
          HERDR_BIN_PATH: fakeHerdrPath,
          HERDR_PANE_ID: paneId,
          HERDR_PLUGIN_STATE_DIR: statePath,
          HERDR_SOCKET_PATH: socketPath,
          HERDR_TAB_ID: options.tabId,
          HERDR_WORKSPACE_ID: options.workspaceId,
        },
        stderr: "piped",
        stdout: "piped",
      });
      const output = await command.output();
      if (focusServer !== undefined) {
        if (output.success) {
          await focusServer.finished;
        } else {
          focusServer.close();
          await focusServer.finished.catch(() => undefined);
        }
      }
      return {
        code: output.code,
        stderr: new TextDecoder().decode(output.stderr),
      };
    },
    async herdrCall(): Promise<readonly string[]> {
      return JSON.parse(await Deno.readTextFile(callsPath));
    },
    async writeRawState(text: string): Promise<void> {
      await Deno.mkdir(statePath, { recursive: true });
      await Deno.writeTextFile(join(statePath, "focus-anchor.json"), text);
    },
    async writeState(state: Record<string, unknown>): Promise<void> {
      await this.writeRawState(JSON.stringify(state));
    },
    dispose: () => Deno.remove(directory, { recursive: true }),
  };
}

function directionLayout(direction: "left" | "right" | "up" | "down", paneId: string) {
  const current = { height: 40, width: 40, x: 40, y: 40 };
  const target = {
    height: 40,
    width: 40,
    x: direction === "left" ? 0 : direction === "right" ? 80 : 40,
    y: direction === "up" ? 0 : direction === "down" ? 80 : 40,
  };
  return {
    panes: [
      { pane_id: "pane-current", rect: current },
      { pane_id: paneId, rect: target },
    ],
    tab_id: "tab-2-2",
    workspace_id: "workspace-2",
  };
}

function singlePaneLayout(tabId: string, workspaceId: string, paneId: string) {
  return {
    panes: [
      { pane_id: paneId, rect: { height: 80, width: 100, x: 0, y: 0 } },
    ],
    tab_id: tabId,
    workspace_id: workspaceId,
  };
}

function paneLayout(tabId: string, workspaceId: string, prefix: string) {
  return {
    panes: [
      {
        pane_id: `${prefix}-bottom-right`,
        rect: { height: 40, width: 50, x: 50, y: 40 },
      },
      {
        pane_id: `${prefix}-top-right`,
        rect: { height: 40, width: 50, x: 50, y: 0 },
      },
      {
        pane_id: `${prefix}-top-left`,
        rect: { height: 80, width: 50, x: 0, y: 0 },
      },
    ],
    tab_id: tabId,
    workspace_id: workspaceId,
  };
}

function startFocusServer(socketPath: string, callsPath: string) {
  const listener = Deno.listen({ path: socketPath, transport: "unix" });
  let closed = false;
  const close = () => {
    clearTimeout(timeout);
    if (closed) return;
    closed = true;
    listener.close();
  };
  const timeout = setTimeout(close, 2_000);
  const finished = (async () => {
    let connection: Deno.Conn;
    try {
      connection = await listener.accept();
    } catch (error) {
      if (closed) {
        throw new Error("timed out waiting for pane.focus socket request", { cause: error });
      }
      throw error;
    }
    try {
      const request = JSON.parse(await readLine(connection));
      const paneId = request.params?.pane_id;
      await Deno.writeTextFile(callsPath, JSON.stringify(["pane", "focus", paneId]));
      const response = `${
        JSON.stringify({
          id: request.id,
          result: { pane: { pane_id: paneId }, type: "pane_info" },
        })
      }\n`;
      await connection.write(new TextEncoder().encode(response));
    } finally {
      connection.close();
      close();
    }
  })();
  return { close, finished };
}

async function readLine(connection: Deno.Conn): Promise<string> {
  const decoder = new TextDecoder();
  const buffer = new Uint8Array(8_192);
  let text = "";
  while (!text.includes("\n")) {
    const size = await connection.read(buffer);
    if (size === null) throw new Error("socket closed before a response line was received");
    text += decoder.decode(buffer.subarray(0, size), { stream: true });
  }
  return text.slice(0, text.indexOf("\n"));
}
