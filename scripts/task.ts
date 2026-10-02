import { fileURLToPath } from "node:url";

export const root = fileURLToPath(new URL("../", import.meta.url));
export const windows = Deno.build.os === "windows";
export const binary = `herdr-grid-slide${windows ? ".exe" : ""}`;

export async function run(
  command: string,
  args: string[],
  options: Deno.CommandOptions = {},
) {
  const status = await new Deno.Command(command, {
    cwd: root,
    args,
    stdin: "inherit",
    stdout: "inherit",
    stderr: "inherit",
    ...options,
  }).spawn().status;
  if (!status.success) throw new CommandError(command, status.code);
}

export function reportError(error: unknown): never {
  console.error(error instanceof Error ? error.message : error);
  Deno.exit(error instanceof CommandError ? error.code : 1);
}

export class CommandError extends Error {
  constructor(command: string, readonly code: number) {
    super(`${command} exited with status ${code}`);
  }
}
