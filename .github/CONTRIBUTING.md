# Contributing to Riflesso X

Thanks for helping! Riflesso X is a small project: clear reports and focused pull
requests are the best way to move it forward.

## Reporting a problem

Open an [issue](https://github.com/kannarozzi/Riflesso-X/issues) with:

- your Windows version, the sending device and its iOS / iPadOS / macOS version;
- what you did, what you expected and what happened instead;
- the log, if the problem is about connecting or the picture: it is
  `data\riflesso-x.log` next to `Riflesso X.exe`. Check it for anything you would
  rather not share (device names, IP addresses) before attaching it.

## Translations

Every language lives in [`ui/src/i18n.tsx`](../ui/src/i18n.tsx). To improve one, edit its
strings; to add one, follow the steps in [docs/DEVELOPMENT.md](../docs/DEVELOPMENT.md#translations).
Native speakers' corrections are very welcome: some languages were translated
without one.

## Code

1. Read [docs/DEVELOPMENT.md](../docs/DEVELOPMENT.md) to set up the project.
2. Keep changes focused, and match the style of the code around them.
3. Before opening a pull request, make sure these pass:

   ```sh
   cargo clippy --release --all-targets   # no warnings
   cargo test --workspace
   npm run build
   ```

4. If you change the types shared with the interface, update `ui/src/backend.ts`
   together with `apps/desktop/src/state.rs` and `settings.rs`.

By contributing you agree that your work is released under the
[GPL-3.0-or-later](../LICENSE.md), like the rest of the project.
