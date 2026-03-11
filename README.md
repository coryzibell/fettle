# strop

**The final sharpening.** Replaces Claude Code's constrained file tools with sharp, unrestricted alternatives.

## The Problem

Claude Code's built-in `Read` and `Write` tools have artificial limits:

- **Read** fails on text files over 25,000 tokens (~1,500 lines of code) or 256KB
- **Read** silently truncates files between 48-126KB to a 2KB preview
- **Write** refuses to write if you haven't `Read` the file first in the current session
- The docs claim lines over 2,000 characters are truncated. [They aren't.](https://github.com/coryzibell/strop/issues/1)

Images, PDFs, and notebooks are fine — the limits only affect text files, which is most of what coding agents work with.

## What strop Does

`strop` installs as Claude Code pre-tool-use hooks. It transparently intercepts `Read` and `Write` calls:

- **Text files ≥ 48KB** → strop reads them directly, no token limits, no size caps
- **Text files < 48KB** → passed through to builtins (they work fine here)
- **Images, PDFs, notebooks** → passed through to builtins (multimodal rendering)
- **All writes** → strop handles directly, no read-before-write gate

Agents don't need to change anything. They call `Read` and `Write` as normal. strop makes them work.

## Install

```bash
cargo install strop
strop install  # sets up Claude Code hooks
```

## CLI Usage

Also works as a standalone tool:

```bash
strop read src/main.rs                  # full file, cat -n formatting
strop read big_file.rs --offset 100 --limit 50  # chunked
strop write output.txt                  # reads content from stdin
strop info                              # show config and detected limits
```

## The Name

Four meanings:

1. **Strop** — the leather strip for the final sharpening of a blade
2. **"Strop using the shit tool"** — instructions included
3. **Throwing a strop** — British slang for tantrum, which is what `Read` does at 48KB
4. Also what `Write` does when you haven't read the file first

## Empirical Testing

All limits were [empirically tested](https://github.com/coryzibell/strop/issues/1), not assumed from documentation (which turned out to be wrong about several things).

| Scenario | Built-in | strop |
|----------|----------|-------|
| 2,000-line source file | Token error | Works |
| 500KB log file | Size error | Works |
| Write after creating file via shell | "Read it first" | Works |
| 8MB PNG screenshot | Works | Passes through |
| 50-page PDF | Works | Passes through |

## License

Apache 2.0
