# codemeter

A lightweight desktop app that tracks your AI coding tool usage limits. Lives in the system tray and shows your current usage at a glance.

<img width="500" height="406" alt="image" src="https://github.com/user-attachments/assets/3dc94272-b8d0-4d7d-93a5-4f11982841bd" />

## Features

- System tray app with click-to-toggle info window
- Tracks **Claude Code** and **Codex CLI** usage limits
- Shows 5-hour and weekly usage percentages with progress bars
- Displays reset countdowns and exact reset times
- Auto-refreshes every 60 seconds with 5-minute API caching
- Automatic OAuth token refresh for Claude Code

## Runtime prerequisites

- Logged into [Claude Code](https://github.com/anthropics/claude-code) (`claude login`) and/or [Codex CLI](https://github.com/openai/codex) (`codex --login`)

## Installation

Download the latest installer for your platform from the [releases page](https://github.com/mikecao/codemeter/releases).

### macOS

The macOS build is unsigned, so the first launch may show a "codemeter is damaged and can't be opened" warning. This is expected for unsigned apps — clear the quarantine flag after moving the app to `/Applications`:

```bash
xattr -dr com.apple.quarantine /Applications/codemeter.app
```

Then open it normally.

## Development prerequisites

- [Rust](https://rustup.rs/)
- [Node.js](https://nodejs.org/) and [pnpm](https://pnpm.io/)

## Development

```bash
pnpm install
pnpm tauri:dev
```

## Build

```bash
pnpm tauri build
```

Produces installers in `src-tauri/target/release/bundle/`.

## How it works

Codemeter reads your existing CLI credentials and queries the usage APIs directly:

- **Claude Code**: Reads OAuth token from macOS Keychain when available, otherwise falls back to `~/.claude/.credentials.json` or `~/.claude/credentials.json`, then calls the Anthropic usage API
- **Codex CLI**: Reads access token from `~/.codex/auth.json`, calls the OpenAI usage API

No separate login required. If a CLI is not installed or not logged in, the app shows a helpful message.

## Tech stack

- [Tauri v2](https://v2.tauri.app/) (Rust backend)
- [React](https://react.dev/) + [TypeScript](https://www.typescriptlang.org/) (frontend)
- [Vite](https://vite.dev/) (bundler)

## License

[MIT](LICENSE.md)
