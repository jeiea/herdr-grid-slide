import { strict as assert } from "node:assert";
import { delimiter, join } from "node:path";
import { fileURLToPath } from "node:url";
import { writeFakeCommand } from "./support/fake-command.ts";

const root = fileURLToPath(new URL("../", import.meta.url));
const mainSha = "a".repeat(40);
const releaseSha = "b".repeat(40);
const runUrl = "https://github.com/example/plugin/actions/runs/123";
const runId = 123;

Deno.test("releasing main waits for its latest CI run and proceeds only after success", async () => {
  const fixture = await prepare();
  try {
    for (
      const result of [
        [],
        [{ status: "completed", conclusion: "failure", url: runUrl }],
        [{ status: "completed", conclusion: "cancelled", url: runUrl }],
        [{ status: "completed", conclusion: "skipped", url: runUrl }],
      ]
    ) {
      const { output, calls } = await fixture.release([], result);
      assert.equal(
        output.success,
        false,
        "unverified commits must not release",
      );
      assert.deepEqual(calls, [mainLookup(), ciLookup(mainSha)]);
    }
    const { output, calls } = await fixture.release([], [{
      status: "completed",
      conclusion: "success",
      url: runUrl,
    }]);
    assert.equal(output.code, 0, new TextDecoder().decode(output.stderr));
    assert.deepEqual(calls, [
      mainLookup(),
      ciLookup(mainSha),
      ["workflow", "run", "release.yml", "--ref", "main"],
    ]);
    assert(new TextDecoder().decode(output.stdout).includes(runUrl));
    for (const status of ["queued", "in_progress"]) {
      const pending = [{
        databaseId: runId,
        status,
        conclusion: "",
        url: runUrl,
      }];
      const successful = await fixture.release([], pending);
      assert.equal(successful.output.code, 0);
      assert.deepEqual(successful.calls, [
        mainLookup(),
        ciLookup(mainSha),
        ciWatch(),
        ["workflow", "run", "release.yml", "--ref", "main"],
      ]);
      const failed = await fixture.release([], pending, { watchExitCode: 1 });
      assert.equal(failed.output.code, 1);
      assert.deepEqual(failed.calls, [
        mainLookup(),
        ciLookup(mainSha),
        ciWatch(),
      ]);
    }
  } finally {
    await fixture.cleanup();
  }
});

Deno.test("a release checks its own commit and blocks missing results or lookup errors", async () => {
  const fixture = await prepare();
  try {
    const successful = [{
      status: "completed",
      conclusion: "success",
      url: runUrl,
    }];
    const { output, calls } = await fixture.release([
      "--check",
      releaseSha,
    ], successful);
    assert.equal(output.code, 0, new TextDecoder().decode(output.stderr));
    assert.deepEqual(calls, [ciLookup(releaseSha)]);

    const pending = [{
      databaseId: runId,
      status: "in_progress",
      conclusion: "",
      url: runUrl,
    }];
    const waited = await fixture.release(["--check", releaseSha], pending);
    assert.equal(waited.output.code, 0);
    assert.deepEqual(waited.calls, [ciLookup(releaseSha), ciWatch()]);
    const interrupted = await fixture.release(
      ["--check", releaseSha],
      pending,
      {
        watchExitCode: 42,
      },
    );
    assert.equal(interrupted.output.code, 42);
    assert.deepEqual(interrupted.calls, [ciLookup(releaseSha), ciWatch()]);

    const missing = await fixture.release(["--check", releaseSha], []);
    assert.equal(missing.output.success, false);
    assert.deepEqual(missing.calls, [ciLookup(releaseSha)]);

    const failed = await fixture.release(["--check", releaseSha], [{
      status: "completed",
      conclusion: "failure",
      url: runUrl,
    }]);
    assert.equal(failed.output.success, false);
    assert.deepEqual(failed.calls, [ciLookup(releaseSha)]);
    assert(new TextDecoder().decode(failed.output.stderr).includes(runUrl));

    const unavailable = await fixture.release([], successful, { error: 42 });
    assert.equal(unavailable.output.code, 42);
    assert.deepEqual(unavailable.calls, [mainLookup(), ciLookup(mainSha)]);

    const invalid = await fixture.release(["--check"], successful);
    assert.equal(invalid.output.success, false);
    assert.deepEqual(invalid.calls, []);
  } finally {
    await fixture.cleanup();
  }
});

function mainLookup() {
  return ["api", "repos/{owner}/{repo}/commits/main", "--jq", ".sha"];
}

function ciLookup(sha: string) {
  return [
    "run",
    "list",
    "--workflow",
    "ci.yml",
    "--branch",
    "main",
    "--event",
    "push",
    "--commit",
    sha,
    "--limit",
    "1",
    "--json",
    "databaseId,status,conclusion,url",
  ];
}

function ciWatch() {
  return ["run", "watch", String(runId), "--exit-status"];
}

async function prepare() {
  const temporary = await Deno.makeTempDir({
    prefix: "grid-slide release test ",
  });
  const callsPath = join(temporary, "calls.jsonl");
  const tool = join(temporary, "gh.ts");
  await Deno.writeTextFile(
    tool,
    `await Deno.writeTextFile(Deno.env.get("TEST_GH_CALLS"), JSON.stringify(Deno.args) + "\\n", { append: true });
if (Deno.args[0] === "api") console.log(${JSON.stringify(mainSha)});
else if (Deno.args[0] === "run" && Deno.args[1] === "list") {
  Deno.exitCode = Number(Deno.env.get("TEST_GH_ERROR"));
  console.log(Deno.env.get("TEST_GH_RUNS"));
} else if (Deno.args[0] === "run" && Deno.args[1] === "watch") {
  Deno.exitCode = Number(Deno.env.get("TEST_GH_WATCH_EXIT_CODE"));
} else if (Deno.args[0] !== "workflow") Deno.exit(99);
`,
  );
  await writeFakeCommand(join(temporary, "gh"), [tool]);
  return {
    async release(
      args: string[],
      runs: unknown,
      options: { error?: number; watchExitCode?: number } = {},
    ) {
      await Deno.writeTextFile(callsPath, "");
      const output = await new Deno.Command(Deno.execPath(), {
        args: ["run", "-A", join(root, "scripts/release.ts"), ...args],
        cwd: temporary,
        env: {
          PATH: `${temporary}${delimiter}${Deno.env.get("PATH")}`,
          TEST_GH_CALLS: callsPath,
          TEST_GH_RUNS: JSON.stringify(runs),
          TEST_GH_ERROR: String(options.error ?? 0),
          TEST_GH_WATCH_EXIT_CODE: String(options.watchExitCode ?? 0),
        },
      }).output();
      const recorded = await Deno.readTextFile(callsPath);
      const calls = recorded.trim()
        ? recorded.trim().split("\n").map((line) => JSON.parse(line))
        : [];
      return { output, calls };
    },
    cleanup: () => Deno.remove(temporary, { recursive: true }),
  };
}
