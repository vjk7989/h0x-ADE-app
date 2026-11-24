import { expect, test } from "bun:test";
import { mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";

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
  }, 60000); // the first PowerShell start can take longer than the 5 s default
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

// The ssh config reading with real files and a real sh, started the way HyperOx
// starts programs, so a Windows home (C:\Users\...) behaves as it does there.
async function hostsUnder(prefix: string) {
  const home = await mkdtemp(join(tmpdir(), prefix));
  try {
    await mkdir(join(home, ".ssh", "config.d"), { recursive: true });
    const config = "Include config.d/*\nHost direct\n";
    await writeFile(join(home, ".ssh", "config"), config);
    await writeFile(join(home, ".ssh", "config.d", "a-work"), "Host work\n  HostName 10.0.0.1\n");
    await writeFile(join(home, ".ssh", "config.d", "b-lab"), "Host lab\n");
    const values = {
      View: class {}, div() {}, svg() {}, TextView: {}, v_flex() {}, h_flex() {}, InputState: {}, Input: {},
      core() {}, theme() {}, props: () => ({}), closeOverlay() {}, openOverlay() {},
      home: () => home,
      readTextFile: (file: string) => readFile(file, "utf8"),
      exec: async (program: string, args: string[]) => {
        const p = Bun.spawnSync([program, ...args]);
        return { code: p.exitCode, stdout: p.stdout.toString(), stderr: p.stderr.toString() };
      },
    };
    const body = source.replace(/^import .*;\r?\n/gm, "").replace("export default class MachineAdd", "class MachineAdd");
    const parseConfig = new Function(...Object.keys(values), `${body}; return parseConfig;`)(...Object.values(values));
    return (await parseConfig(config)).map((h: any) => [h.alias, h.hostname]);
  } finally {
    await rm(home, { recursive: true, force: true });
  }
}

test("every file a wildcard Include matches is listed", async () => {
  expect(await hostsUnder("h0x-ssh-")).toEqual([["work", "10.0.0.1"], ["lab", null], ["direct", null]]);
});

test("a wildcard Include is listed under a home with a space", async () => {
  expect(await hostsUnder("h0x ssh ")).toEqual([["work", "10.0.0.1"], ["lab", null], ["direct", null]]);
});

// Copyable login commands (UPSTREAM-SYNC-2 A8): what Copy puts on the clipboard is
// exactly the quoted command the instructions show, and it parses as one argument.
function recordingElements() {
  const make = () => {
    const node: any = { calls: {} as Record<string, unknown[]>, children: [] as any[] };
    const el: any = new Proxy(node, { get: (_, method) => {
      if (method === "node") return node;
      return (...args: any[]) => {
        if (method === "child") node.children.push(args[0]?.node ?? args[0]);
        else if (method === "children") node.children.push(...args[0].map((c: any) => c?.node ?? c));
        else if (method === "when") { if (args[0]) args[1](el); }
        else node.calls[String(method)] = args;
        return el;
      };
    } });
    return el;
  };
  const all = (node: any): any[] => [node, ...(node?.children ?? []).flatMap(all)];
  const byId = (root: any, id: string) => all(root).find((n: any) => n?.calls?.id?.[0] === id);
  return { make, all, byId };
}
function messageView(platform: string) {
  const { make, byId } = recordingElements();
  const values = {
    View: class {}, div: make, svg: make, TextView: { html: make }, v_flex: make, h_flex: make,
    InputState: { new: () => ({ on() {}, focus() {}, set_value() {} }) }, Input: {},
    home: () => "/home/test", readTextFile: async () => null, theme() {}, props: () => ({}), closeOverlay() {},
    window: () => ({ platform }), openOverlay() {}, exec: async () => ({}), core: async () => ({}),
  };
  const MachineAdd = new Function(...Object.keys(values), source.replace(/^import .*;\n/gm, "").replace("export default class MachineAdd", "return class MachineAdd"))(...Object.values(values));
  const view = new MachineAdd();
  Object.assign(view, { state: {}, pending: new Set() });
  return { view, byId };
}

const copyTargets = ["me@my host", "my user@host", "it's", "it\u2019s", "a$b", "$(whoami)", "a\"b", "host';echo $HOME", "C:\\Users\\me\\host"];
for (const platform of ["linux", "windows"]) for (const target of copyTargets) {
  test(`Copy on ${platform} puts the same quoted command as the instructions for ${JSON.stringify(target)}`, async () => {
    const s = setup({ code: 1, stdout: JSON.stringify({ error: { kind: "host_key" } }) }, platform);
    await s.add(target);
    const quoted = s.view.state[target].match(/`h0x-core machine add (.*?)` in a terminal\./s)![1];
    const m = messageView(platform);
    Object.assign(m.view, { state: { [target]: s.view.state[target] }, pending: new Set([target]) });
    const button = m.byId(m.view.message({}, target).node, `ma-copy-${target}`);
    expect(button.children).toEqual(["Copy"]);
    const clipboard: string[] = [];
    let notified = 0;
    button.calls.on_click[0](null, { write_to_clipboard: (text: string) => clipboard.push(text), notify: () => { notified++; } });
    expect(clipboard).toEqual([`h0x-core machine add ${quoted}`]);
    expect(notified).toBe(1);
    expect(m.byId(m.view.message({}, target).node, `ma-copy-${target}`).children).toEqual(["Copied"]);
    if (platform !== "linux" || !Bun.which("sh")) return;
    // The copied line is the command plus exactly one argument: the target as typed.
    const shell = Bun.spawnSync(["sh", "-c", `set -- ${clipboard[0]}; printf '%s|%s' "$#" "$4"`]);
    expect(shell.stdout.toString()).toBe(`4|${target}`);
  });
}

test("no Copy button once nothing is pending for the target", () => {
  const m = messageView("linux");
  Object.assign(m.view, { state: { "my host": "it didn't work" }, pending: new Set() });
  expect(m.byId(m.view.message({}, "my host").node, "ma-copy-my host")).toBeUndefined();
});

// The machine-login toast: only a name the core accepts becomes a copyable command.
const toastSource = (await readFile(new URL("../defaults/mods/toast/main.js", import.meta.url), "utf8"))
  .replace(/^import .*;\n/gm, "").replace("export default class Toast", "class Toast");
function toast(p: Record<string, unknown>) {
  const { make, all, byId } = recordingElements();
  const globals = {
    View: class {}, div: make, image: make, v_flex: make, h_flex: make,
    props: () => p, closeOverlay() {}, run() {}, theme: () => new Proxy({}, { get: () => "#000000" }),
  };
  const Toast = new Function(...Object.keys(globals), `${toastSource}; return Toast;`)(...Object.values(globals));
  const view = new Toast();
  const render = () => view.render({}).node;
  const texts = () => all(render()).filter((n: any) => typeof n === "string");
  return { view, render, texts, byId };
}
// The core names the machine twice: "<name> needs login; run h0x-core machine login <name>".
const loginBody = (name: string) => `${name} needs login; run h0x-core machine login ${name}`;

for (const name of ["box", "my-box.lan", "a", "9lives_2", "a".repeat(32)]) test(`a login toast for ${name} offers its command to copy`, () => {
  const t = toast({ title: "Machine", body: loginBody(name) });
  expect(t.texts()).toContain(`${name} needs a login: run`);
  expect(t.texts()).toContain(`h0x-core machine login ${name}`);
  expect(t.texts()).not.toContain(loginBody(name));
  const clipboard: string[] = [];
  let stopped = 0;
  t.byId(t.render(), "toast-login-copy").calls.on_click[0](null, { stop_propagation: () => { stopped++; }, write_to_clipboard: (s: string) => clipboard.push(s), notify() {} });
  expect(clipboard).toEqual([`h0x-core machine login ${name}`]);
  expect(stopped).toBe(1);
  expect(t.byId(t.render(), "toast-login-copy").children).toEqual(["Copied"]);
});

for (const name of ["my box", "it's", "a$b", "$(whoami)", "a\"b", "Box", "-box", ".box", "a".repeat(33), "box\nrm -rf ~", ""]) test(`a login toast naming ${JSON.stringify(name)} offers no command to copy`, () => {
  // The bundled toast grows to fit its text (a833e0b), so it shows the whole message; only a fixed-height toast clips to the first line.
  const t = toast({ title: "Machine", body: loginBody(name), fitHeight: true });
  expect(t.byId(t.render(), "toast-login-copy")).toBeUndefined();
  expect(t.texts()).toContain("Machine");
  expect(t.texts()).toContain(loginBody(name));
});

test("a toast without a body, or with another body, is a plain toast", () => {
  for (const body of [undefined, null, "", "Build finished"]) {
    const t = toast({ title: "Done", body });
    expect(t.byId(t.render(), "toast-login-copy")).toBeUndefined();
    expect(t.texts()).toContain("Done");
  }
});

test("a login toast whose two names differ, or with text after the name, offers no command", () => {
  for (const body of ["box needs login; run h0x-core machine login other", `${loginBody("box")} now`, `${loginBody("box")}\n`, "box needs login; run h0x-core machine login "]) {
    const t = toast({ title: "Machine", body });
    expect(t.byId(t.render(), "toast-login-copy")).toBeUndefined();
  }
});
