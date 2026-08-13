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

console.error(`unexpected herdr command: ${Deno.args.join(" ")}`);
Deno.exit(2);

function requiredEnv(name: string): string {
  const value = Deno.env.get(name);
  if (value === undefined || value === "") {
    throw new Error(`missing ${name}`);
  }
  return value;
}
