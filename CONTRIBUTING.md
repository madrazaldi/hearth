# Contributing to Hearth

Thanks for helping out. Bug reports, feature requests and pull requests are all welcome.

## Reporting bugs

Open an [issue](https://github.com/madrazaldi/hearth/issues/new/choose) with your Hearth version (shown at the bottom of Settings), your OS, and the server or bouncer you were connected to. Raw IRC lines help a lot. Remove passwords, tokens and SASL exchanges before you paste them.

## Making changes

The [Development](README.md#development) section of the README covers setup. [AGENTS.md](AGENTS.md) describes the architecture and the conventions the code follows, such as how to add a command end to end and where tests go.

Most UI work doesn't need an IRC server: `bun run dev` runs the interface in a browser against a scripted session (`src/lib/mock.ts`).

Before opening a pull request, make sure these pass. CI runs the same checks on Linux, macOS and Windows:

```sh
bun run check
bun run test
```

- Add tests for behaviour changes. Feed raw IRC lines through `src/lib/test-utils.ts` for frontend logic, and use the scripted server in `src/bun/core/*.test.ts` (see `harness.ts`) for protocol changes.
- For UI changes, include a screenshot in the pull request.
- Use [conventional commit](https://www.conventionalcommits.org) prefixes (`feat:`, `fix:`, `perf:`, `docs:`). The release notes are generated from them.
- Keep secrets out of code, logs and test fixtures.

By contributing you agree that your work is released under the [GPL-3.0-or-later](LICENSE) license.
