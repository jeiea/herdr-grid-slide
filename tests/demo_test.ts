import { strict as assert } from "node:assert";
import { delimiter, join } from "node:path";
import { fileURLToPath } from "node:url";
import { writeFakeCommand } from "./support/fake-command.ts";

Deno.test("invalid recordings preserve the validation status and remove their isolated session files", async () => {
  const temporary = await Deno.makeTempDir({ prefix: "grid-slide demo test " });
  const root = fileURLToPath(new URL("../", import.meta.url));
  const project = join(temporary, "project with spaces");
  const tools = join(temporary, "tools");
  const windows = Deno.build.os === "windows";
  const binary = `herdr-grid-slide${windows ? ".exe" : ""}`;
  try {
    for (const directory of [".mise/tasks", "scripts", "target/release"]) {
      await Deno.mkdir(join(project, directory), { recursive: true });
    }
    await Deno.mkdir(tools);
    for (
      const file of [
        ".mise/tasks/demo.ts",
        "scripts/task.ts",
        "herdr-plugin.toml",
        "README.md",
      ]
    ) {
      await Deno.copyFile(join(root, file), join(project, file));
    }
    await Deno.writeTextFile(
      join(project, "target/release", binary),
      "local plugin",
    );
    await Deno.writeTextFile(
      join(project, "custom.tape"),
      "Set Shell zsh\nKey alt+m\nKey alt+shift+h\nKey ctrl+alt+m\n",
    );
    const fixture = join(root, "tests/fixtures/demo-tools.ts");
    for (const name of ["cargo", "vhs"]) {
      await writeFakeCommand(join(tools, name), [fixture, name]);
    }
    const demoPath = join(temporary, "demo-path");
    const output = await new Deno.Command(Deno.execPath(), {
      args: ["run", "-A", join(project, ".mise/tasks/demo.ts"), "custom.tape"],
      cwd: temporary,
      env: {
        PATH: `${tools}${delimiter}${Deno.env.get("PATH")}`,
        HERDR_WORKSPACE_ID: "inherited-workspace",
        HERDR_SOCKET_PATH: "inherited-socket",
        TEST_DEMO_PATH: demoPath,
      },
    }).output();
    assert.equal(output.code, 42, new TextDecoder().decode(output.stderr));
    const demo = await Deno.readTextFile(demoPath);
    await assert.rejects(Deno.stat(demo), Deno.errors.NotFound);
  } finally {
    await Deno.remove(temporary, { recursive: true });
  }
});

Deno.test({
  name: "recording the detach shortcut moves the focused pane to a new tab",
  ignore: Deno.env.get("GRID_SLIDE_TEST_DEMO") !== "1",
  async fn() {
    const temporary = await Deno.makeTempDir({
      prefix: "grid-slide recording ",
    });
    const root = fileURLToPath(new URL("../", import.meta.url));
    const tape = join(temporary, "test.tape");
    const output = join(temporary, "recording.gif").replaceAll("\\", "/");
    await Deno.writeTextFile(
      tape,
      `Output ${JSON.stringify(output)}
Set Shell zsh
Set Width 640
Set Height 360
Set Framerate 10
Hide
Type "herdr --session demo" Enter
Sleep 3s
Show
Key alt+n
Sleep 1s
Key alt+m
Sleep 3s
`,
    );
    const recording = new Deno.Command(Deno.execPath(), {
      args: ["run", "-A", join(root, ".mise/tasks/demo.ts"), tape],
      cwd: root,
      stdout: "piped",
      stderr: "piped",
    }).spawn();
    let transcript = "";
    const decoder = new TextDecoder();
    const stdout = recording.stdout.pipeTo(
      new WritableStream({
        write(chunk) {
          transcript += decoder.decode(chunk, { stream: true });
        },
      }),
    );
    let errors = "";
    const errorDecoder = new TextDecoder();
    const stderr = recording.stderr.pipeTo(
      new WritableStream({
        write(chunk) {
          const text = errorDecoder.decode(chunk, { stream: true });
          errors += text;
          transcript += text;
        },
      }),
    );
    const env = Object.fromEntries(
      Object.entries(Deno.env.toObject()).filter(([name]) =>
        !name.startsWith("HERDR_")
      ),
    );
    let detached = false;
    let exited = false;
    recording.status.then(() => exited = true);
    try {
      const deadline = Date.now() + 30000;
      while (!exited && Date.now() < deadline) {
        const config = transcript.match(/File: (.+)[\\/]demo\.tape/);
        if (config) {
          const demo = config[1].trim();
          const state = await new Deno.Command("herdr", {
            args: ["--session", "demo", "api", "snapshot"],
            clearEnv: true,
            env: {
              ...env,
              XDG_CONFIG_HOME: join(demo, "config"),
              XDG_STATE_HOME: join(demo, "state"),
              XDG_RUNTIME_DIR: join(demo, "runtime"),
              HERDR_CONFIG_PATH: join(demo, "config.toml"),
            },
          }).output();
          if (state.success) {
            const snapshot =
              JSON.parse(new TextDecoder().decode(state.stdout)).result
                .snapshot;
            detached ||= snapshot.tabs.length === 2 &&
              snapshot.tabs.every((tab: { pane_count: number }) =>
                tab.pane_count === 1
              );
          }
        }
        await new Promise((resolve) => setTimeout(resolve, 100));
      }
      assert((await recording.status).success, errors);
      assert(
        detached,
        `Alt+m must detach one of the two panes into a new tab\n${transcript}`,
      );
    } finally {
      await recording.status;
      await stdout;
      await stderr;
      await Deno.remove(temporary, { recursive: true });
    }
  },
});
