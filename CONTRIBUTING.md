# Contributing to Dove

Thanks for helping. Dove is a native Rust app, so most changes are small, focused Rust edits plus a manual check in the running window.

## Ground rules

- **One change per pull request.** A bug fix, a feature or a refactor, not all three.
- **Keep the UI honest and calm.** Monochrome tokens from `theme.rs`, glass surfaces from `kit.rs`/`ui.rs`, no accent colours, badges or glow. Disabled controls say why. No fake buttons.
- **Zero idle cost.** Do not add timers or animation loops that run when nothing changes. An idle window should render zero frames.
- **The engine records, the UI renders.** Session activity goes through `dove-core`; UI code only reads `LiveSession` events and sends `Command`s.
- **No secrets in the binary.** OAuth uses Device Flow or PKCE public clients. Never commit tokens or client secrets.
- **Free infrastructure.** Features must work with the user's own keys and subscriptions. Do not add a dependency on a paid service.

## Setup

1. Install Rust stable 1.90+ (`rustup` picks the version from `rust-toolchain.toml`).
2. Linux only: install the GPUI system packages listed in the [README](README.md#build-from-source).
3. Install at least one agent CLI (for example `npm i -g @anthropic-ai/claude-code`) and sign in to it once.
4. Run the app:

```sh
cargo run -p dove
DOVE_HOME=/tmp/dove-dev cargo run -p dove   # throwaway data dir
DOVE_LOG=debug DOVE_SKIP_INTRO=1 cargo run -p dove
```

## Before you push

CI runs on Ubuntu, macOS and Windows. Run the same checks locally:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --exclude dove
CARGO_INCREMENTAL=0 cargo build -p dove --release
```

The release binary has a size gate of 100 MB. If your change adds a dependency, say why in the PR and check the size.

For UI changes, launch the build and look at it in both light and dark themes. Attach a screenshot or short recording to the PR.

## Where things live

See [ARCHITECTURE.md](ARCHITECTURE.md) for the full map. Common starting points:

| You want to… | Start in |
| --- | --- |
| Support a new agent CLI | `crates/dove-harness` (`acp.rs` for ACP agents, `detect.rs`, `models.rs`) and `HarnessKind` in `crates/dove-proto` |
| Change how a transcript renders | `apps/dove/src/transcript.rs`, `markdown.rs`, `widget.rs` |
| Add a tool every agent can call | `apps/dove/src/mcp_browser.rs` and the app-side handler |
| Change worktree or diff behaviour | `crates/dove-git` |
| Add a stored field | `crates/dove-store` (add a migration, never edit the base schema in place) |
| Change the website | `apps/landing` (plain HTML/CSS/JS) |

## Vendored code

`vendor/` holds copies of Zeron and gpui-component crates. Keep changes there minimal and record each one in `vendor/README.md`. Code ported from Zeron or Cosmos keeps its license notice; credit it in `THIRD_PARTY_NOTICES.md`.

## Commits and pull requests

- Branch from `main`. Write commit messages in the imperative ("Fold tool calls by default").
- Describe what changed and how you verified it. Link the issue it closes.
- Keep PRs reviewable: if a change touches more than a few hundred lines, split it.

## Reporting bugs

Open an issue with your OS, Dove commit, agent CLI and version, what you did, what you expected and what happened. Logs from `DOVE_LOG=debug` help. Remove anything private first.

## Security

Do not open public issues for vulnerabilities. Email buildwithai.dev@gmail.com with details and steps to reproduce.

## License

By contributing you agree that your contributions are licensed under Apache-2.0, the license of this repository.
