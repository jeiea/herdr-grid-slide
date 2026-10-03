#!/usr/bin/env -S deno run -A
//MISE description="Build and apply the local plugin, then reload Herdr configuration"
import { join } from "node:path";
import { binary, reportError, root, run, windows } from "../../scripts/task.ts";

async function main() {
  await run("cargo", ["build", "--locked", "--release"]);
  await Deno.mkdir(join(root, "bin"), { recursive: true });
  const staged = await Deno.makeTempFile({
    dir: join(root, "bin"),
    prefix: ".herdr-grid-slide.",
  });
  try {
    await Deno.copyFile(join(root, "target/release", binary), staged);
    if (!windows) await Deno.chmod(staged, 0o755);
    await Deno.rename(staged, join(root, "bin", binary));
    await run("herdr", ["plugin", "link", "."]);
    await run("herdr", ["server", "reload-config"]);
  } finally {
    await Deno.remove(staged).catch((error) => {
      if (!(error instanceof Deno.errors.NotFound)) throw error;
    });
  }
}

await main().catch(reportError);
