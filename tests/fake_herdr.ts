const [command, subcommand, ...args] = Deno.args;

if (command === "api" && subcommand === "snapshot") {
  console.log(Deno.env.get("FAKE_HERDR_SNAPSHOT"));
  Deno.exit();
}

if (command === "pane" && subcommand === "move") {
  const callsPath = requiredEnv("FAKE_HERDR_CALLS");
  await Deno.writeTextFile(callsPath, JSON.stringify([command, subcommand, ...args]));
  Deno.exit();
}

if (command === "pane" && subcommand === "focus") {
  const direction = optionValue(args, "--direction");
  const expectedDirection = requiredEnv("FAKE_HERDR_EXPECTED_DIRECTION");
  if (direction !== expectedDirection) {
    throw new Error(`expected direction ${expectedDirection}, received ${direction}`);
  }
  const sourcePaneId = optionValue(args, "--pane");
  const neighborPaneId = requiredEnv("FAKE_HERDR_NEIGHBOR_PANE_ID");
  const changed = neighborPaneId !== sourcePaneId;
  if (changed) {
    const callsPath = requiredEnv("FAKE_HERDR_CALLS");
    await Deno.writeTextFile(callsPath, JSON.stringify(["pane", "focus", neighborPaneId]));
  }
  console.log(JSON.stringify({
    result: {
      focus: {
        changed,
        focused_pane_id: changed ? neighborPaneId : sourcePaneId,
        ...(changed ? {} : { reason: "no_neighbor" }),
        source_pane_id: sourcePaneId,
      },
      type: "pane_focus_direction",
    },
  }));
  Deno.exit();
}

console.error(`unexpected herdr command: ${Deno.args.join(" ")}`);
Deno.exit(2);

function requiredEnv(name: string): string {
  const value = Deno.env.get(name);
  if (value === undefined || value === "") {
    throw new Error(`missing ${name}`);
  }
  return value;
}

function optionValue(args: readonly string[], option: string): string {
  const index = args.indexOf(option);
  const value = args[index + 1];
  if (index === -1 || value === undefined) {
    throw new Error(`missing ${option}`);
  }
  return value;
}
