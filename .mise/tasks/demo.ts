#!/usr/bin/env -S deno run -A
//MISE description="Record demo/demo.gif and demo/demo.mp4 in an isolated Herdr session"
import { join } from "node:path";
import { setTimeout as delay } from "node:timers/promises";
import {
  binary,
  CommandError,
  reportError,
  root,
  run,
  windows,
} from "../../scripts/task.ts";

async function main() {
  Deno.chdir(root);
  await run("cargo", ["build", "--locked", "--release"]);
  // Herdr repeats the session root in its socket path, so keep it short.
  const demo = await Deno.makeTempDir({
    prefix: "gsd.",
    ...(windows ? {} : { dir: "/tmp" }),
  });
  let server: Deno.ChildProcess | undefined;
  let recording: Deno.ChildProcess | undefined;
  const keyboardAbort = new AbortController();
  let cleaning: Promise<void> | undefined;
  const cleanup = () =>
    cleaning ??= (async () => {
      keyboardAbort.abort();
      if (recording) await stopProcess(recording);
      if (server) {
        const panes = windows ? await paneProcesses() : [];
        await run("herdr", ["--session", "demo", "server", "stop"], {
          stdout: "null",
          stderr: "null",
        }).catch(() => {});
        await stopProcess(server);
        if (panes.length) {
          await powershell(`
          foreach ($id in @(${panes.join(",")})) {
            $process = Get-Process -Id $id -ErrorAction SilentlyContinue
            if ($process -and -not $process.WaitForExit(10000)) {
              $process.Kill(); $process.WaitForExit()
            }
          }
        `);
        }
        await run("herdr", ["session", "delete", "demo"], {
          stdout: "null",
          stderr: "null",
        }).catch(() => {});
      }
      // Remove only the exact temporary directory created for this recording.
      await Deno.remove(demo, { recursive: true });
    })();
  const interrupted = () => {
    cleanup().then(() => Deno.exit(130)).catch(reportError);
  };
  const signals: Deno.Signal[] = windows
    ? ["SIGINT", "SIGBREAK"]
    : ["SIGINT", "SIGHUP", "SIGTERM"];
  for (const signal of signals) Deno.addSignalListener(signal, interrupted);
  try {
    for (const name of Object.keys(Deno.env.toObject())) {
      if (name.startsWith("HERDR_")) Deno.env.delete(name);
    }
    for (
      const [name, path] of Object.entries({
        XDG_CONFIG_HOME: "config",
        XDG_STATE_HOME: "state",
        XDG_RUNTIME_DIR: "runtime",
        HERDR_CONFIG_PATH: "config.toml",
        GRID_SLIDE_DEMO_ROOT: ".",
        ZDOTDIR: "zsh",
      })
    ) Deno.env.set(name, join(demo, path));
    for (
      const directory of [
        "home",
        "config",
        "state",
        "runtime",
        "zsh",
        "plugin/bin",
      ]
    ) {
      await Deno.mkdir(join(demo, directory), { recursive: true });
    }
    await Deno.copyFile(
      join(root, "herdr-plugin.toml"),
      join(demo, "plugin/herdr-plugin.toml"),
    );
    await Deno.copyFile(
      join(root, "target/release", binary),
      join(demo, "plugin/bin", binary),
    );
    const shell = await prepareShell(demo);
    const readme = await Deno.readTextFile(join(root, "README.md"));
    const bindings = readme.match(/^```toml\r?\n(.*?)^```/ms)?.[1] ?? "";
    await Deno.writeTextFile(
      join(demo, "config.toml"),
      `onboarding = false

[ui]
prompt_new_tab_name = false

[ui.sound]
enabled = false

[terminal]
default_shell = ${JSON.stringify(shell.replaceAll("\\", "/"))}

${bindings}`,
    );
    const tape = await Deno.readTextFile(Deno.args[0] ?? "demo/demo.tape");
    const tapePath = join(demo, "demo.tape");
    await Deno.writeTextFile(tapePath, convertTape(tape));
    await run("vhs", ["validate", tapePath]);

    server = new Deno.Command("herdr", {
      args: ["--session", "demo", "server"],
      cwd: root,
      env: windows ? {} : { HOME: join(demo, "home") },
      stdout: "null",
      stderr: "piped",
    }).spawn();
    let serverError = "";
    const decoder = new TextDecoder();
    const serverOutput = server.stderr.pipeTo(
      new WritableStream({
        write(chunk) {
          serverError += decoder.decode(chunk, { stream: true });
        },
      }),
    );
    let serverExited = false;
    server.status.then(() => serverExited = true);
    await waitUntil(async () => {
      if (serverExited) {
        throw new Error(`Herdr server did not become ready: ${serverError}`);
      }
      return (await new Deno.Command("herdr", {
        args: ["--session", "demo", "api", "snapshot"],
        cwd: root,
        stdout: "null",
        stderr: "null",
      }).output()).success;
    }, { timeout: 15000, message: "Herdr server did not become ready" });
    await run("herdr", [
      "--session",
      "demo",
      "plugin",
      "link",
      join(demo, "plugin"),
    ], {
      stdout: "null",
    });

    const existing = windows ? await browserProcesses() : [];
    recording = new Deno.Command("vhs", {
      args: [tapePath],
      cwd: root,
      stdin: "inherit",
      stdout: "inherit",
      stderr: "inherit",
    }).spawn();
    const keyboard = windows
      ? installKeyboardHandler(
        existing.map((process) => process.ProcessId),
        keyboardAbort.signal,
      )
        .catch(async (error) => {
          await stopProcess(recording!);
          throw error;
        })
      : Promise.resolve();
    try {
      await Promise.all([
        keyboard,
        recording.status.then((status) => {
          if (!status.success) throw new CommandError("vhs", status.code);
        }),
      ]);
    } finally {
      keyboardAbort.abort();
      await keyboard.catch(() => {});
      await cleanup();
      await serverOutput;
    }
  } finally {
    await cleanup();
    for (const signal of signals) {
      Deno.removeSignalListener(signal, interrupted);
    }
  }
}

function convertTape(tape: string) {
  return tape.split(/\r?\n/).map((line) => {
    if (windows && line.startsWith("Set Shell ")) return "Set Shell powershell";
    if (!line.startsWith("Key ")) return line;
    const parts = line.slice(4).split("+");
    const key = parts.pop()!;
    const modifiers: Record<string, number> = { shift: 1, alt: 2, ctrl: 4 };
    for (const part of parts) {
      if (!(part in modifiers)) {
        throw new Error(`unknown modifier: ${line.slice(4)}`);
      }
    }
    if (!windows) {
      const modifier = 1 +
        parts.reduce((sum, part) => sum + modifiers[part], 0);
      return `Type@0ms "\x1b[${key.codePointAt(0)};${modifier}u"`;
    }
    if (!parts.includes("ctrl") && !parts.includes("alt")) {
      throw new Error(`unsupported demo key: ${line.slice(4)}`);
    }
    const chord = (parts.includes("ctrl") ? ["ctrl", "alt", "shift"] : ["alt"])
      .filter((part) => parts.includes(part))
      .map((part) => part[0].toUpperCase() + part.slice(1));
    const character = parts.includes("shift") ? key.toUpperCase() : key;
    // VHS 0.12.1 drops Alt+"m" as a time-unit keyword. An unmapped suffix
    // avoids that lookup; remove it after VHS fixes ExecuteAlt.
    const text = !parts.includes("ctrl") && character === "m"
      ? `${character}\u200b`
      : character;
    return `${chord.join("+")}+"${text}"`;
  }).join("\n");
}

async function prepareShell(demo: string) {
  if (!windows) {
    await Deno.writeTextFile(
      join(demo, "zsh/.zshrc"),
      `count=$(( $(cat "$ZDOTDIR/count" 2>/dev/null || echo 0) + 1 ))
echo "$count" > "$ZDOTDIR/count"
label=\${\${:-ABCDEFGHIJKLMNOP}[count]}
color=$(( (count - 1) % 6 + 1 ))
PROMPT="%F{$color}$label ❯%f "
mkdir -p "$HOME/$label" && cd "$HOME/$label"
print -P "%K{$color}%F{black}   $label   %f%k"
`,
    );
    return "/bin/zsh";
  }
  // Herdr accepts a shell executable without arguments; keep profiles out of each pane.
  await powershell(
    `Add-Type -OutputAssembly (Join-Path $env:GRID_SLIDE_DEMO_ROOT 'shell.exe') -OutputType ConsoleApplication -TypeDefinition @'
using System;
using System.Diagnostics;
class DemoShell {
    static int Main() {
        var script = Environment.GetEnvironmentVariable("GRID_SLIDE_DEMO_ROOT") + "\\\\pane.ps1";
        var start = new ProcessStartInfo("powershell.exe",
            "-NoLogo -NoProfile -NoExit -ExecutionPolicy Bypass -File \\"" + script + "\\"");
        start.UseShellExecute = false;
        var child = Process.Start(start);
        child.WaitForExit();
        return child.ExitCode;
    }
}
'@`,
  );
  await Deno.writeTextFile(
    join(demo, "pane.ps1"),
    `$ErrorActionPreference = 'Stop'
$env:PSModulePath = "$PSHOME/Modules"
Set-PSReadLineOption -HistorySaveStyle SaveNothing
Set-PSReadLineKeyHandler -Chord Ctrl+u -Function RevertLine
$countPath = Join-Path $env:GRID_SLIDE_DEMO_ROOT 'count'
$count = 1
if (Test-Path -LiteralPath $countPath) { $count += [int][System.IO.File]::ReadAllText($countPath) }
[System.IO.File]::WriteAllText($countPath, [string]$count)
$global:demoLabel = [string][char](64 + $count)
$global:demoColor = ($count - 1) % 6 + 1
$directory = Join-Path $env:GRID_SLIDE_DEMO_ROOT "home/$demoLabel"
[System.IO.Directory]::CreateDirectory($directory) | Out-Null
Set-Location -LiteralPath $directory
$esc = [string][char]27
[Console]::Write("\${esc}[4\${demoColor};30m   $demoLabel   \${esc}[0m\`r\`n")
function global:prompt {
    $esc = [string][char]27
    [Environment]::CurrentDirectory = $PWD.ProviderPath
    "\${esc}]9;9;$($PWD.ProviderPath)\${esc}\\\${esc}[3\${demoColor}m$demoLabel > \${esc}[0m"
}
`,
  );
  return join(demo, "shell.exe");
}

async function powershell(script: string) {
  // An already exited pane is an expected lookup miss during cleanup.
  const output = await new Deno.Command("powershell.exe", {
    args: [
      "-NoProfile",
      "-NonInteractive",
      "-Command",
      `$ErrorActionPreference = 'Stop'; $env:PSModulePath = "$PSHOME/Modules"; ${script}; exit 0`,
    ],
    cwd: root,
  }).output();
  if (!output.success) throw new Error(new TextDecoder().decode(output.stderr));
  return new TextDecoder().decode(output.stdout).trim();
}

async function paneProcesses(): Promise<number[]> {
  const output = await powershell(
    `ConvertTo-Json -Compress -InputObject @(Get-CimInstance Win32_Process -Filter "Name = 'shell.exe' OR Name = 'powershell.exe'" | Where-Object {
    $_.CommandLine -and $_.CommandLine.Contains($env:GRID_SLIDE_DEMO_ROOT) -and $_.ProcessId -ne $PID
  } | Select-Object -ExpandProperty ProcessId)`,
  );
  return JSON.parse(output);
}

async function browserProcesses(): Promise<
  { ProcessId: number; CommandLine: string }[]
> {
  return JSON.parse(
    await powershell(
      `ConvertTo-Json -Compress -InputObject @(Get-CimInstance Win32_Process | Where-Object {
    $_.CommandLine -match '--remote-debugging-port='
  } | Select-Object ProcessId, CommandLine)`,
    ),
  );
}

async function installKeyboardHandler(existing: number[], signal: AbortSignal) {
  const options = {
    timeout: 20000,
    message: "VHS browser did not become ready",
    signal,
  };
  const browser = await waitUntil(
    async () =>
      (await browserProcesses()).find((process) =>
        !existing.includes(process.ProcessId) &&
        /--user-data-dir=.*vhs-/.test(process.CommandLine)
      ),
    options,
  );
  const port = browser.CommandLine.match(/--remote-debugging-port=(\d+)/)![1];
  const page = await waitUntil(async () => {
    const pages =
      await (await fetch(`http://127.0.0.1:${port}/json`, { signal })).json();
    return pages.find((page: { type: string; url: string }) =>
      page.type === "page" && /^http:\/\/localhost:/.test(page.url)
    ) as { webSocketDebuggerUrl: string } | undefined;
  }, { ...options, message: "VHS terminal page did not become ready" });
  // Windows Chromium treats Ctrl+Alt as AltGr; encode browser chords as CSI u.
  const expression = `new Promise(resolve => {
    const timer = setInterval(() => {
      if (!window.term) return;
      clearInterval(timer);
      term.attachCustomKeyEventHandler(event => {
        if (!event.altKey || event.key.length !== 1) return true;
        if (event.type === 'keydown') {
          const key = event.key.toLowerCase();
          const shift = event.shiftKey || event.key !== key;
          const modifier = 1 + (shift ? 1 : 0) + 2 + (event.ctrlKey ? 4 : 0);
          term.input('\\x1b[' + key.codePointAt(0) + ';' + modifier + 'u', true);
        }
        return false;
      });
      resolve(true);
    }, 25);
  })`;
  const socket = new WebSocket(page.webSocketDebuggerUrl);
  const abort = () => socket.close();
  signal.addEventListener("abort", abort, { once: true });
  try {
    await new Promise<void>((resolve, reject) => {
      socket.onopen = () =>
        socket.send(JSON.stringify({
          id: 1,
          method: "Runtime.evaluate",
          params: { expression, awaitPromise: true, returnByValue: true },
        }));
      socket.onmessage = (event) => {
        const result = JSON.parse(event.data);
        if (result.id !== 1) return;
        if (result.result?.result?.value === true) resolve();
        else {reject(
            new Error(`Could not install demo keyboard handler: ${event.data}`),
          );}
      };
      socket.onerror = () => reject(new Error("VHS browser connection failed"));
      socket.onclose = () => reject(new Error("VHS browser connection closed"));
    });
  } finally {
    signal.removeEventListener("abort", abort);
    socket.close();
  }
}

async function waitUntil<T>(
  check: () => Promise<T>,
  options: { timeout: number; message: string; signal?: AbortSignal },
): Promise<NonNullable<T>> {
  const deadline = Date.now() + options.timeout;
  do {
    options.signal?.throwIfAborted();
    const value = await check();
    if (value) return value;
    await delay(100, undefined, { signal: options.signal });
  } while (Date.now() < deadline);
  throw new Error(options.message);
}

async function stopProcess(process: Deno.ChildProcess) {
  const timer = new AbortController();
  try {
    const exited = await Promise.race([
      process.status.then(() => true),
      delay(10000, false, { signal: timer.signal }),
    ]);
    if (!exited) {
      process.kill();
      await process.status;
    }
  } finally {
    timer.abort();
  }
}

await main().catch(reportError);
