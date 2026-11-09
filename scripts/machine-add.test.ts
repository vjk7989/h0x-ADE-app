import { expect, test } from "bun:test";
import { readFile } from "node:fs/promises";

const source = await readFile(new URL("../defaults/mods/machine-add/main.js", import.meta.url), "utf8");

function setup(result: any, platform = "linux") {
  const calls: string[][] = [];
  const overlays: unknown[] = [];
  const machines: any[] = [];
  const probes: unknown[] = [];
  const timers: ((cx: any) => void)[] = [];
  let task: Promise<void>;
  const values = {
    View: class {}, div() {}, svg() {}, TextView: {}, v_flex() {}, h_flex() {},
    InputState: { new: () => ({ on() {}, focus() {}, set_value() {} }) }, Input: {},
    home: () => "/home/test", readTextFile: async () => null,
    theme() {}, props: () => ({}), closeOverlay() {}, window: () => ({ platform }),
    openOverlay: (...args: unknown[]) => overlays.push(args),
    exec: async (_cmd: string, _args: string[], options: any) => {
      probes.push(options.on);
      return { stdout: "Darwin\n/remote/h0x-core\n" };
    },
    core: async (args: string[]) => {
      calls.push(args);
      if (args[1] === "add") {
        if (result.code === 0) machines.push({ target: args[2], name: args[2] });
        return result;
      }
      return { code: 0, stdout: JSON.stringify({ machines }) };
    },
  };
  const MachineAdd = new Function(...Object.keys(values), source.replace(/^import .*;\n/gm, "").replace("export default class MachineAdd", "return class MachineAdd"))(...Object.values(values));
  const view = new MachineAdd();
  const cx = {
    notify() {}, focus_handle() {}, spawn(fn: any) { task = fn(cx); },
    timer: { after() {}, every(_ms: number, fn: any) { timers.push(fn); } },
  };
  view.init({}, cx);
  return {
    view, calls, overlays, machines, probes,
    async add(target = "my-host") {
      await task;
      view.typed = target;
      view.add(` ${target} `, cx);
      await task;
    },
    async poll() { timers.forEach((fn) => fn(cx)); await task; },
  };
}

for (const kind of ["h0x_core_missing", "incompatible", "login_needed", "auth_failed", "host_key"]) {
  test(`${kind} gives terminal instructions without starting a terminal`, async () => {
    const s = setup({ code: 1, stdout: JSON.stringify({ error: { kind, message: "needs attention" } }) });
    await s.add();
    expect(s.view.state["my-host"]).toContain("Couldn't add my-host here");
    expect(s.view.state["my-host"]).toContain("`h0x-core machine add my-host`");
    expect(s.view.state["my-host"]).toContain("`h0x-core machine --help`");
    expect(s.view.isAdded("my-host")).toBe(false);
    expect(s.calls.every((args) => args[0] === "machine")).toBe(true);
    expect(s.overlays).toEqual([]);
  });
}

for (const message of ["Enter password", "Enter passphrase", "new host key", "Permission denied", "needs a login"]) {
  test(`${message} also gives instructions`, async () => {
    const s = setup({ code: 1, stderr: JSON.stringify({ error: { kind: "other", message } }) });
    await s.add();
    expect(s.view.state["my-host"]).toContain("`h0x-core machine add my-host`");
  });
}

for (const [target, quoted] of [
  ["ssh://user@[::1]:2222", "'ssh://user@[::1]:2222'"],
  ["host';echo $HOME", "'host'\\'';echo $HOME'"],
]) {
  test(`instructions quote ${target} as one shell argument`, async () => {
    const s = setup({ code: 1, stdout: JSON.stringify({ error: { kind: "host_key" } }) });
    await s.add(target);
    expect(s.view.state[target]).toContain(`\`h0x-core machine add ${quoted}\``);
    expect(s.calls.find((args) => args[1] === "add")?.[2]).toBe(target);
    const shell = Bun.spawnSync(["sh", "-c", `printf '%s' ${quoted}`]);
    expect(shell.exitCode).toBe(0);
    expect(shell.stdout.toString()).toBe(target);
  });
}

test("pending guidance clears when another terminal adds the host and checks Mac permissions", async () => {
  const s = setup({ code: 1, stdout: JSON.stringify({ error: { kind: "host_key" } }) });
  await s.add();
  await s.poll();
  expect(s.view.pending.has("my-host")).toBe(true);
  expect(s.view.state["my-host"]).toContain("h0x-core machine add my-host");
  expect(s.probes).toEqual([]);
  s.machines.push({ target: "my-host", name: "remote" });
  await s.poll();
  expect(s.view.isAdded("my-host")).toBe(true);
  expect(s.view.state["my-host"]).toBeUndefined();
  expect(s.view.pending.size).toBe(0);
  expect(s.view.typed).toBe("");
  expect(s.view.macs["my-host"]).toEqual({ name: "remote", core: "/remote/h0x-core" });
  expect(s.probes).toEqual(["remote"]);
  const count = s.calls.length;
  await s.poll();
  expect(s.calls.length).toBe(count);
});

test("other failures keep their message and do not poll", async () => {
  const s = setup({ code: 1, stdout: JSON.stringify({ error: { kind: "offline", message: "connection refused" } }) });
  await s.add();
  expect(s.view.state["my-host"]).toBe("connection refused");
  const count = s.calls.length;
  await s.poll();
  expect(s.calls.length).toBe(count);
});

test("success refreshes the list and clears the typed target without opening anything", async () => {
  const s = setup({ code: 0, stdout: "{}" });
  await s.add();
  expect(s.view.isAdded("my-host")).toBe(true);
  expect(s.view.state["my-host"]).toBeUndefined();
  expect(s.view.typed).toBe("");
  expect(s.probes).toEqual(["my-host"]);
  expect(s.calls.every((args) => args[0] === "machine")).toBe(true);
  expect(s.overlays).toEqual([]);
});

for (const target of ["me@my host", "ssh://user@host/path with spaces", "C:\\Users\\me\\host", "user@host:2222", "host.example.com", ""]) {
  test(`instructions for ${JSON.stringify(target)} keep one argument or add nothing`, async () => {
    const s = setup({ code: 1, stdout: JSON.stringify({ error: { kind: "host_key" } }) });
    await s.add(target);
    if (!target) {
      expect(s.calls.some((args) => args[1] === "add")).toBe(false);
      expect(s.view.pending.size).toBe(0);
      return;
    }
    const plain = /^[a-z0-9_@%+=:,./-]+$/i.test(target);
    expect(s.view.state[target]).toContain(plain ? `\`h0x-core machine add ${target}\`` : `\`h0x-core machine add '`);
    expect(s.view.pending.has(target)).toBe(true);
    expect(s.overlays).toEqual([]);
  });
}

test("a core that cannot be run gives an error message, not instructions", async () => {
  const s = setup({ code: 1, stdout: JSON.stringify({ error: { kind: "unreachable", message: "no route to host" } }) });
  await s.add();
  expect(s.view.state["my-host"]).toBe("no route to host");
  expect(s.view.pending.size).toBe(0);
});

test("a missing remote core (kind h0x_core_missing) gives instructions", async () => {
  const s = setup({ code: 1, stdout: JSON.stringify({ error: { kind: "h0x_core_missing", message: "h0x-core is not installed there" } }) });
  await s.add();
  expect(s.view.state["my-host"]).toContain("`h0x-core machine add my-host`");
  expect(s.view.pending.has("my-host")).toBe(true);
});

const awkward = [
  "me@my host", "a,b", "it's", "it\u2019s", "a$b", "$(whoami)", "a`b", "a;b", "a&b", "a|b", "a\"b", "a\\b",
  "C:\\Users\\me\\My Documents\\host", "host';echo $HOME",
  "\\\\server\\share\\host",
];

for (const target of awkward) {
  test(`posix quoting keeps ${JSON.stringify(target)} one literal argument`, async () => {
    const s = setup({ code: 1, stdout: JSON.stringify({ error: { kind: "host_key" } }) }, "linux");
    await s.add(target);
    const quoted = s.view.state[target].match(/`h0x-core machine add (.*?)` in a terminal\./s)![1];
    if (!Bun.which("sh")) return;
    const shell = Bun.spawnSync(["sh", "-c", `printf '%s' ${quoted}`]);
    expect(shell.stdout.toString()).toBe(target);
  });

  test(`powershell quoting keeps ${JSON.stringify(target)} one literal argument`, async () => {
    const s = setup({ code: 1, stdout: JSON.stringify({ error: { kind: "host_key" } }) }, "windows");
    await s.add(target);
    const quoted = s.view.state[target].match(/`h0x-core machine add (.*?)` in a terminal\./s)![1];
    expect(quoted.startsWith("'") && quoted.endsWith("'")).toBe(true);
    // Inside single quotes only a doubled quote character may appear.
    expect(quoted.slice(1, -1).replace(/(['\u2018\u2019\u201A\u201B])\1/g, "")).not.toMatch(/['\u2018\u2019\u201A\u201B]/);
    const ps = Bun.which("pwsh") ?? Bun.which("powershell");
    if (!ps) return;
    const script = `[Console]::OutputEncoding=[Text.Encoding]::UTF8; [Console]::Out.Write(${quoted})`;
    const encoded = Buffer.from(script, "utf16le").toString("base64");
    const shell = Bun.spawnSync([ps, "-NoProfile", "-NonInteractive", "-EncodedCommand", encoded]);
    expect(shell.stdout.toString()).toBe(target);
  });
}

test("plain targets are not quoted on either platform", async () => {
  for (const platform of ["linux", "windows"]) {
    const s = setup({ code: 1, stdout: JSON.stringify({ error: { kind: "host_key" } }) }, platform);
    await s.add("user@host.example.com:2222");
    expect(s.view.state["user@host.example.com:2222"]).toContain("`h0x-core machine add user@host.example.com:2222`");
  }
});

for (const platform of ["linux", "windows"]) {
  test(`a target with a newline stays one quoted argument on ${platform}`, async () => {
    const target = "host\nrm -rf ~";
    const s = setup({ code: 1, stdout: JSON.stringify({ error: { kind: "host_key" } }) }, platform);
    await s.add(target);
    const quoted = s.view.state[target].match(/`h0x-core machine add (.*?)` in a terminal\./s)![1];
    expect(quoted).toBe(`'${target}'`);
  });
}

// The core rejects a target that ssh would read as an option; its message
// repeats the target, which must not decide that the person should run it.
test("a target rejected as an ssh option gets the core's message, not a command to run", async () => {
  const target = "-oProxyCommand=login";
  const message = `bad ssh target "${target}"`;
  const s = setup({ code: 1, stdout: JSON.stringify({ error: { kind: "invalid_params", message } }) });
  await s.add(target);
  expect(s.calls.find((args) => args[1] === "add")).toEqual(["machine", "add", target, "--json"]);
  expect(s.view.state[target]).toBe(message);
  expect(s.view.pending.size).toBe(0);
});

test("a target that spells a keyword the core echoes still gets only the core's message when rejected", async () => {
  const message = 'bad target "login"';
  const s = setup({ code: 1, stdout: JSON.stringify({ error: { kind: "invalid_params", message } }) });
  await s.add("login");
  expect(s.view.state["login"]).toBe(message);
  expect(s.view.pending.size).toBe(0);
});

test("a target starting with a dash never gets a command to run, whatever the kind", async () => {
  const s = setup({ code: 1, stdout: JSON.stringify({ error: { kind: "host_key", message: "new host key" } }) });
  await s.add("-oProxyCommand=x");
  expect(s.view.state["-oProxyCommand=x"]).toBe("new host key");
  expect(s.view.pending.size).toBe(0);
});

test("ssh's own words under the unreachable kind still give instructions", async () => {
  const s = setup({ code: 1, stdout: JSON.stringify({ error: { kind: "unreachable", message: "Permission denied (publickey,password)" } }) });
  await s.add();
  expect(s.view.state["my-host"]).toContain("`h0x-core machine add my-host`");
});
