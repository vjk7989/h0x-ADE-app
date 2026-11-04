import { describe, expect, test } from "bun:test";
import { checkExample, checkMarkdown, commandPage, type Reference } from "./docs";

const ref: Reference = {
  help: "Usage:\n  h0x-ade status\n  h0x-ade ls\n  h0x-ade do <command> [args]\n  h0x-ade commands\n  h0x-ade --skill\n  h0x-ade --version\n",
  commands: [{ name: "pane.split", about: "Split a pane.", args: [{ name: "direction", required: true }] }],
};

describe("CLI examples", () => {
  test("validates commands and named arguments without executing examples", () => {
    for (const source of ["h0x-ade", "h0x-ade ls", "h0x-ade --help", "h0x-ade --skill", "h0x-ade do pane.split --direction right", `h0x-ade do pane.split '{"direction":"right"}'`]) {
      expect(checkExample(source, ref)).toEqual([]);
    }
    expect(checkExample("h0x-ade wat", ref)).toEqual(["unknown command or flag: wat"]);
    expect(checkExample("h0x-ade ls && h0x-ade wat", ref)).toEqual(["unknown command or flag: wat"]);
    expect(checkExample("h0x-ade --wat", ref)).toHaveLength(1);
    expect(checkExample("h0x-ade ls --wat", ref)).toHaveLength(1);
    expect(checkExample("h0x-ade do pane.wat", ref)).toHaveLength(1);
    expect(checkExample("h0x-ade do pane.split --wat right", ref)).toHaveLength(1);
    expect(checkExample("h0x-ade do pane.split --direction=right", ref)).toContain("unknown flag for do: --direction=right");
    expect(checkExample("h0x-ade do pane.split --direction='right'", ref)).toContain("unknown flag for do: --direction=right");
    expect(checkExample('h0x-ade ls "--wat"', ref)).toEqual(["unknown flag for ls: --wat"]);
    expect(checkExample('h0x-ade do pane.split "--wat" right', ref)).toEqual(["unknown flag for do: --wat"]);
    expect(checkExample('h0x-ade do pane.split "--direction" "right"', ref)).toEqual([]);
    expect(checkExample("h0x-ade do pane.split --direction", ref)).toEqual(["missing value for --direction"]);
  });

  test("ignores flags inside quotes and comments, and other programs", () => {
    expect(checkExample(`h0x-ade do pane.split --direction '--not-a-flag' # --comment`, ref)).toEqual([]);
    expect(checkExample("curl --not-our-flag https://example.org", ref)).toEqual([]);
    expect(checkExample("h0x-ade ls | other --not-our-flag", ref)).toEqual([]);
    expect(checkExample("h0x-ade do pane.split --direction h0x-ade", ref)).toEqual([]);
  });

  test("reports inline and fenced examples with file and line", () => {
    expect(checkMarkdown("README.md", "Text `h0x-ade wat`.\n\n```sh\nh0x-ade ls --wat\n```", ref)).toEqual([
      "README.md:1: unknown command or flag: wat",
      "README.md:4: unknown flag for ls: --wat",
    ]);
    expect(checkMarkdown("docs/example.md", "~~~sh\nh0x-ade do pane.split \\\n  --wat right\n~~~", ref)).toEqual(["docs/example.md:2: unknown flag for do: --wat"]);
    expect(checkMarkdown("README.md", "Plain prose h0x-ade wat is not code.", ref)).toEqual([]);
  });
});

test("command reference comes from the catalog", () => {
  const page = commandPage(ref.commands);
  expect(page).toContain("## `pane.split`");
  expect(page).toContain("| `direction` | yes |");
  expect(page).toContain("Split a pane.");
  expect(checkMarkdown("docs/commands.md", page, ref)).toEqual([]);
});

