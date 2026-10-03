export async function writeFakeCommand(path: string, args: string[]) {
  const windows = Deno.build.os === "windows";
  const commandPath = path + (windows ? ".cmd" : "");
  const quote = (value: string) =>
    windows ? `"${value}"` : "'" + value.replaceAll("'", "'\\''") + "'";
  const command = [Deno.execPath(), "run", "-A", ...args].map(quote).join(" ");
  await Deno.writeTextFile(
    commandPath,
    windows
      ? `@echo off\r\n${command} %*\r\n`
      : `#!/bin/sh\nexec ${command} "$@"\n`,
  );
  if (!windows) await Deno.chmod(commandPath, 0o755);
}
