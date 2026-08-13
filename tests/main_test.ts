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
      assertEquals(await integration.moveCall(), [
        ...expectedMovePrefix,
        scenario.targetTabId,
        ...expectedMoveSuffix,
      ]);
    } finally {
      await integration.dispose();
    }
  });
}

interface IntegrationOptions {
  readonly workspaceId: string;
  readonly tabId: string;
}

async function setupIntegration(options: IntegrationOptions) {
  const directory = await Deno.makeTempDir({ prefix: "herdr-move-pane-test-" });
  const fakeHerdrPath = join(directory, "herdr");
  const callsPath = join(directory, "calls.json");
  const fakeSource = await Deno.readTextFile(join(root, "tests", "fake_herdr.ts"));
  await Deno.writeTextFile(
    fakeHerdrPath,
    `#!/usr/bin/env -S deno run --quiet --allow-env --allow-write\n${fakeSource}`,
  );
  await Deno.chmod(fakeHerdrPath, 0o755);

  return {
    async run(scope: "workspace" | "tab", direction: "next" | "previous") {
      const command = new Deno.Command(Deno.execPath(), {
        args: [
          "run",
          "--quiet",
          "--allow-env=HERDR_BIN_PATH,HERDR_WORKSPACE_ID,HERDR_TAB_ID,HERDR_PANE_ID",
          "--allow-run",
          "src/main.ts",
          scope,
          direction,
        ],
        cwd: root,
        env: {
          FAKE_HERDR_CALLS: callsPath,
          FAKE_HERDR_SNAPSHOT: JSON.stringify({ result: { snapshot: { tabs, workspaces } } }),
          HERDR_BIN_PATH: fakeHerdrPath,
          HERDR_PANE_ID: "pane-current",
          HERDR_TAB_ID: options.tabId,
          HERDR_WORKSPACE_ID: options.workspaceId,
        },
        stderr: "piped",
        stdout: "piped",
      });
      const output = await command.output();
      return {
        code: output.code,
        stderr: new TextDecoder().decode(output.stderr),
      };
    },
    async moveCall(): Promise<readonly string[]> {
      return JSON.parse(await Deno.readTextFile(callsPath));
    },
    dispose: () => Deno.remove(directory, { recursive: true }),
  };
}
