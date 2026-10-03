import { CommandError, reportError, root, run } from "./task.ts";

async function main() {
  if (
    Deno.args.length !== 0 &&
    !(Deno.args.length === 2 && Deno.args[0] === "--check")
  ) {
    throw new Error("usage: release.ts [--check <commit-sha>]");
  }
  const sha = Deno.args[1] ?? await gh([
    "api",
    "repos/{owner}/{repo}/commits/main",
    "--jq",
    ".sha",
  ]);
  const [result] = JSON.parse(
    await gh([
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
      "status,conclusion,url",
    ]),
  );
  if (!result) throw new Error(`CI has not run for ${sha} on main.`);
  if (result.status !== "completed" || result.conclusion !== "success") {
    throw new Error(
      `CI must pass for ${sha}: ${result.status}/${result.conclusion} ${result.url}`,
    );
  }
  console.log(`CI passed for ${sha}: ${result.url}`);
  if (Deno.args.length === 0) {
    await run("gh", ["workflow", "run", "release.yml", "--ref", "main"]);
  }
}

async function gh(args: string[]) {
  const output = await new Deno.Command("gh", {
    args,
    cwd: root,
    stderr: "inherit",
  }).output();
  if (!output.success) throw new CommandError("gh", output.code);
  return new TextDecoder().decode(output.stdout).trim();
}

await main().catch(reportError);
