# codemeter

A lightweight desktop app that tracks your AI coding tool usage limits. Lives in the system tray and shows your current usage at a glance.

<img width="500" height="406" alt="image" src="https://github.com/user-attachments/assets/3dc94272-b8d0-4d7d-93a5-4f11982841bd" />

## Features

- System tray app with click-to-toggle info window
- Tracks **Claude Code**, **Codex CLI**, **OpenCode Go** and **Grok** usage limits
- Shows usage percentages with progress bars for each service's rate-limit windows (5-hour, weekly, monthly — whichever the service exposes)
- Displays reset countdowns and exact reset times
- Auto-refreshes every 60 seconds with 5-minute API caching
- Automatic OAuth token refresh for Claude Code and Grok

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
- Logged into one or more of:
  - [Claude Code](https://github.com/anthropics/claude-code) (`claude login`)
  - [Codex CLI](https://github.com/openai/codex) (`codex --login`)
  - [OpenCode](https://opencode.ai/) with an [OpenCode Go](https://opencode.ai/go) subscription (`opencode auth login` → OpenCode Go)
  - [Grok](https://x.ai/cli) (`grok login`)

## Development

```bash
pnpm install
pnpm dev
```

## Build

```bash
pnpm build
```

Produces installers in `src-tauri/target/release/bundle/`.

## How it works

Codemeter reads your existing CLI credentials and queries the usage APIs directly:

- **Claude Code**: Reads OAuth token from macOS Keychain when available, otherwise falls back to `~/.claude/.credentials.json` or `~/.claude/credentials.json`, then calls the Anthropic usage API
- **Codex CLI**: Reads access token from `~/.codex/auth.json`, calls the OpenAI usage API
- **OpenCode Go**: Reads the `opencode-go` API key from `~/.local/share/opencode/auth.json` (or `$XDG_DATA_HOME/opencode/auth.json`), calls `https://opencode.ai/zen/go/v1/usage`
- **Grok**: Reads the OIDC session from `~/.grok/auth.json` (or `$GROK_HOME/auth.json`), calls the Grok CLI billing API. Expired sessions are refreshed via `auth.x.ai` and written back, using the same `auth.json.lock` advisory lock as the Grok CLI

No separate login required. CLIs that are not installed are hidden; installed-but-logged-out ones show a login hint.

## Tech stack

- [Tauri v2](https://v2.tauri.app/) (Rust backend)
- [React](https://react.dev/) + [TypeScript](https://www.typescriptlang.org/) (frontend)
- [Vite](https://vite.dev/) (bundler)

## License

[MIT](LICENSE.md)
