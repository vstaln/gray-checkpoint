<p align="center">
  <img src="assets/gray-logo.svg" alt="gray" width="96">
</p>
<h1 align="center">gray-checkpoint</h1>
<p align="center">Git stash checkpoints per turn, with `/checkpoint list|restore`.</p>
<p align="center">
  <a href="https://github.com/vstaln/gray-checkpoint/blob/main/LICENSE"><img alt="MIT License" src="https://img.shields.io/badge/license-MIT-blue.svg"></a>
  <img alt="gray plugin" src="https://img.shields.io/badge/gray-plugin-7aa2f7.svg">
  <img alt="rust" src="https://img.shields.io/badge/built%20with-rust-orange.svg">
</p>

Git stash checkpoints per session; `/checkpoint list|restore`.

A sidecar plugin for [gray](https://github.com/vstaln/gray), scaffolded by
[gray-account](https://github.com/vstaln/gray-account).

## What it does

On `agent/before_start` and after every `turn_end` notification, the sidecar
runs `git stash create` in the session's repo (`session.cwd`). When the tree
is dirty the resulting commit ref is appended as `{turn, ref}` to
`~/.gray/checkpoint/<session-id>.jsonl`.

- `/checkpoint list` — show this session's checkpoints.
- `/checkpoint restore <n>` — `git stash apply <ref>` in the session repo;
  refuses when the cwd isn't a repo or the checkpoint number doesn't exist.
- `/checkpoint restore` (no number) — interactive picker via `host/ask`:
  options are the recorded checkpoints (label `#<turn> <short ref>`,
  description = full ref) plus `cancel`. Picking one runs the same restore
  path. When the ask channel isn't available (capability ungranted,
  non-interactive host, timeout) it falls back to the
  `usage: /checkpoint restore <n>` text.

Gray's wire has no fork event, so restore is manual via `/checkpoint`.

## Wire methods

`plugin/manifest`, `agent/before_start`, `command/run`, `plugin/shutdown`;
notification `event/notify` (`turn_end`). Protocol 2.0 (mutating hook
`agent/before_start`, though this plugin only uses it as a side effect and
returns `{}`). Capability `host.ask` powers the interactive restore picker.

## Install

```sh
gray plugin install checkpoint
gray plugin capabilities checkpoint --all   # grants host.ask for the picker
```

Without the capability the plugin still works; bare `/checkpoint restore`
prints usage instead of opening the picker.

## Develop

```sh
cargo test
cargo build --release
gray account check      # entry point + manifest handshake
```

---
Part of the [gray](https://github.com/vstaln/gray) plugin ecosystem —
the open-source AI agent harness. <https://gray.alignment.id>
