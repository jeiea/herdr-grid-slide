import { strict as assert } from "node:assert";
import { dirname, join } from "node:path";

if (Deno.args[0] === "cargo") {
  assert.deepEqual(Deno.args.slice(1), ["build", "--locked", "--release"]);
  Deno.exit(0);
}
assert.deepEqual(Deno.args.slice(0, 2), ["vhs", "validate"]);
const demo = dirname(Deno.args[2]);
const windows = Deno.build.os === "windows";
const binary = `herdr-grid-slide${windows ? ".exe" : ""}`;
assert.equal(Deno.env.get("HERDR_WORKSPACE_ID"), undefined);
assert.equal(Deno.env.get("HERDR_SOCKET_PATH"), undefined);
assert.equal(Deno.env.get("HERDR_CONFIG_PATH"), join(demo, "config.toml"));
assert.equal(
  await Deno.readTextFile(join(demo, "plugin/bin", binary)),
  "local plugin",
);
const config = await Deno.readTextFile(join(demo, "config.toml"));
assert.match(config, /onboarding = false/);
assert.match(config, /jeiea.grid-slide.focus-left/);
assert.match(config, windows ? /shell\.exe/ : /default_shell = "\/bin\/zsh"/);
const tape = await Deno.readTextFile(Deno.args[2]);
assert.equal(
  tape,
  windows
    ? 'Set Shell powershell\nAlt+"m\u200b"\nAlt+"H"\nCtrl+Alt+"m"\n'
    : 'Set Shell zsh\nType@0ms "\x1b[109;3u"\nType@0ms "\x1b[104;4u"\nType@0ms "\x1b[109;7u"\n',
);
await Deno.writeTextFile(Deno.env.get("TEST_DEMO_PATH")!, demo);
Deno.exit(42);
