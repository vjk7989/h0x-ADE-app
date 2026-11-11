# To-do lists

A to-do panel sits next to **Files** and **Changes**. Open it with the **To-do** button or `Ctrl+Shift+J` (`Cmd+Shift+J` on macOS). The key opens the panel and focuses it; press it again while it has focus to close it.

The lists are the same in every space and folder. Each list is a plain Markdown file, so your agent can read and update them too.

## Lists, tasks and steps

- Pick a list from the switcher at the top, or add one with **+ List**. Its menu renames or deletes a list; delete asks first.
- Type in **Add a task** and press `Enter`. The field stays focused for the next one.
- A task can have steps (one level). Tick a task and its steps stay as they are; tick the last step and the task stays open.
- Done tasks move to a collapsed **Completed** group. Untick one to bring it back.
- Deleting a task or step shows an **Undo** toast for a few seconds.

Keys inside the panel:

| Key | Does |
|---|---|
| `Up` / `Down` | move between rows |
| `Right` / `Left` | show / hide steps |
| `Space` | tick or untick |
| `Enter` | edit the title (`Enter` saves, `Esc` cancels) |
| `N` or `Ctrl+Enter` | new task |
| `Shift+Enter` | new step under the selected task |
| `Tab` / `Shift+Tab` | make a task a step / a step a task |
| `Delete` | remove (with Undo) |
| `Alt+Up` / `Alt+Down` | reorder |
| `Ctrl+L` | list switcher |
| `Esc` | back to the terminal |

When you tick the last open task of a list, a **Congratulations** pop-up appears once. **Celebrate** shows a second of confetti; **Close** (or `Esc`) dismisses it. It never appears when a list is merely loaded.

## Where the files are

In `~/.config/h0x-ade/todo/` (or `$H0X_ADE_HOME/todo/`):

- `lists.md` lists the lists, one `- [Name](slug.md)` line each, in display order.
- `<slug>.md` is one list: `# Name` on the first line, then tasks.

```md
# Groceries
- [ ] Milk
  - [x] Check the fridge
- [x] Bread
```

Edit them in any editor. The open panel picks up changes from you or an agent within moments, and a save never overwrites a change made meanwhile. Lines the panel does not understand are kept.

## Turn it off

**Settings → Look → To-do list** hides the button and panel. Your files stay where they are and come back when you turn it on again.

[Back to HyperOx](../README.md)
