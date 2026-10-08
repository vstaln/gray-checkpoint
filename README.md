# gray-checkpoint

Git stash checkpoints per session; `/checkpoint list|restore`. Port of pi's `git-checkpoint` extension.

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

pi restored checkpoints automatically on `/fork`; gray's wire has no fork
event, so restore is manual via `/checkpoint`.

## Wire methods

`plugin/manifest`, `agent/before_start`, `command/run`, `plugin/shutdown`;
notification `event/notify` (`turn_end`). Protocol 2.0 (mutating hook
`agent/before_start`, though this port only uses it as a side effect and
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
