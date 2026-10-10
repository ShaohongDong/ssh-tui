# SSH TUI

A Rust SSH connection manager for Linux with a compact terminal interface, multiple session tabs, control-key forwarding, and HTTP CONNECT proxy support. SSH transport, authentication, and host-key verification use the system OpenSSH client.

- Saved connection profiles, OpenSSH aliases, and temporary quick connections.
- Independent terminal tabs with in-memory scrollback and manual reconnection.
- Remote keyboard forwarding, including Ctrl+C, Ctrl+D, and full-screen terminal applications.
- Six built-in color themes, live previews, and custom TOML themes.
- HTTP/1.1 CONNECT proxies with optional Basic authentication and per-connection routing.
- Content-sized panels, aligned lists, responsive forms, and scrollable help and error details.

The interface supports English and Chinese, with English as the default.

## Interface previews

| Dark theme | Light theme |
| --- | --- |
| ![Compact SSH TUI with a dark theme](docs/ui-dark.png) | ![Compact SSH TUI with a light theme](docs/ui-light.png) |

The remote terminal reserves one row for tabs and one for status. Lists show up to ten entries at a time, and narrow windows use stacked form fields. See the [UI design notes](docs/ui-design.md) for layout and theme details; that document is in Chinese.

## Build and run

Requirements:

- Linux.
- Rust 1.88 or newer and Cargo to build from source.
- An OpenSSH client (`ssh`) available on `PATH`.
- An interactive terminal for both standard input and standard output. True-color support is recommended for theme rendering.

From the repository root:

```bash
cargo build --release --locked
./target/release/ssh-tui
```

In the connection manager, press `a` to add a saved profile or `c` to open a temporary quick connection. Select a profile and press Enter to connect. Host fields accept aliases from `~/.ssh/config`; OpenSSH resolves `Host`, `Include`, `Match`, identities, agent authentication, and jump-host settings. SSH TUI does not edit your OpenSSH configuration.

Connect immediately from the command line:

```bash
# These connections are not saved as profiles.
./target/release/ssh-tui user@example.org --port 22
./target/release/ssh-tui my-ssh-alias --identity ~/.ssh/id_ed25519

# HTTP CONNECT proxy; the username is separate from the URL.
./target/release/ssh-tui my-ssh-alias --proxy http://127.0.0.1:7890
./target/release/ssh-tui my-ssh-alias --proxy http://proxy.example.org:8080 --proxy-user alice
./target/release/ssh-tui my-ssh-alias --no-proxy

# Choose a temporary language, theme, or a different configuration file.
./target/release/ssh-tui --language zh
./target/release/ssh-tui --language en
./target/release/ssh-tui --theme 'Solarized Light'
./target/release/ssh-tui --config ./examples/config.toml

./target/release/ssh-tui --help
./target/release/ssh-tui --version
```

`--port` (`-p`) and `--identity` (`-i`) require a target. `--proxy-user` requires `--proxy`, and `--proxy` conflicts with `--no-proxy`. Command-line theme and proxy overrides last for the current run. Saving unrelated settings does not persist those overrides; pressing Enter in the theme picker explicitly saves the selected theme.

Cargo produces `target/release/ssh-tui`. The ignored `dist/` directory is used for local distribution artifacts and is not included in a clone or created by the build command above. If you receive a prebuilt executable, it still requires Linux, compatible system libraries, OpenSSH, and an interactive terminal.

## Keyboard controls

When the remote terminal has focus, Ctrl+C, Ctrl+D, Ctrl+Z, Ctrl+L, Tab, Esc, arrow keys, Home/End, PageUp/PageDown, Insert/Delete, and F1–F12 are forwarded to the remote session. Ctrl+C interrupts the remote program; Ctrl+D sends the EOF control byte. When SSH exits, the tab keeps its terminal contents. Press `r` in that tab to reconnect.

### Local command prefix

Press **Ctrl+]**, release it, then press a command key:

| Command key | Action |
| --- | --- |
| `c` | Open the connection manager |
| `n` / `p` | Next / previous tab |
| `1`–`9` | Select one of the first nine tabs |
| `t` | Open the theme picker with live previews |
| `l` | Open the language picker |
| `[` | Browse scrollback |
| `x` | Close the current tab |
| `q` | Quit |
| `!` | Show full status or error details |
| `?` | Show shortcut help |
| Ctrl+] again | Send one literal Ctrl+] to the remote session |
| Esc | Cancel the pending prefix |

Unknown prefix combinations forward both the prefix and the following key. Pasted text does not trigger local commands. In scrollback mode, use arrows or `j`/`k`, PageUp/PageDown, and Home/End; Esc returns to the live terminal. Each tab keeps 10,000 scrollback lines by default. SSH TUI does not write input, output, or password logs.

### Local views

| View | Controls |
| --- | --- |
| Connection manager | Up/Down or `j`/`k` to select; Enter to connect; `a`/`e`/`d` to add/edit/delete; `c` for a temporary quick connection; `/` to search; `p` for proxies; `t` for themes; `l` for language; Tab/Esc to return to the terminal; `q` to quit |
| Proxy manager | Up/Down or `j`/`k` to select; `a`/`e`/`d` to add/edit/delete; Enter to set the global default; `0` to disable the default; `l` for language; Esc to return to the connection manager |
| Forms | Tab, Shift+Tab, or Up/Down to change fields; Left/Right and Home/End to move the cursor; Ctrl+U to clear a field; Enter to submit; Esc to cancel |
| Language picker | Up/Down or `j`/`k` to preview; Enter to save; Esc to restore the previous language and return |
| Theme picker | Up/Down or `j`/`k` to preview; Enter to save; Esc to restore the previous theme |
| Help and details | Up/Down or `j`/`k`, PageUp/PageDown, and Home/End to scroll; Esc or Enter to return to the previous view |
| Confirmation | `y`/`Y` to confirm; `n`/`N` or Esc to cancel |

Saved-profile forms persist on submission; quick-connection forms only start a session. Closing a running tab, quitting with running sessions, and deleting saved configuration entries require confirmation.

Press `?` for help or `!` for details in the connection and proxy managers. F2 opens details in local views other than help and details themselves. In the remote terminal, use `Ctrl+] !`; F2 remains a remote key. Save notices disappear after about three seconds. Errors remain until the next operation, and details expose messages that do not fit the status row. A running SSH process in the status bar indicates that the local process is alive; use the remote terminal output to determine whether authentication succeeded.

Your outer terminal must deliver the keys to SSH TUI. Traditional terminal protocols cannot distinguish Ctrl+] from Ctrl+5, Tab from Ctrl+I, or Esc from Ctrl+[. Keys intercepted by the outer terminal cannot be forwarded. Paste using your terminal's paste command.

## HTTP proxy support

Open the proxy manager with `p` from the connection manager. Proxy URLs accept DNS names, IPv4 addresses, and bracketed IPv6 addresses, such as `http://proxy.example.org:8080` or `http://[::1]:7890`. The default HTTP port is 80. Do not include credentials, a non-root path, a query, or a fragment in the URL.

The proxy transport uses **HTTP/1.1 CONNECT**. HTTPS proxy transport and HTTP/2 are not supported. The proxy must allow CONNECT to the target SSH port, and the target hostname is resolved by the proxy.

Leave the proxy username empty for an unauthenticated proxy. With a username, the session prompts for a Basic-authentication password without echoing it. The password is not stored in the application configuration, command-line arguments, environment variables, or logs. Plain HTTP does not encrypt Basic credentials between the client and proxy. The tunneled SSH session remains encrypted and uses OpenSSH host-key verification.

### Routing precedence

The connection form's proxy field accepts:

| Form value | TOML route | Behavior |
| --- | --- | --- |
| Empty or `inherit` | `{ mode = "inherit" }` | Use a command-line proxy override, otherwise the global default proxy; without either, use OpenSSH routing |
| `direct` | `{ mode = "direct" }` | Disable the application's HTTP proxy and use OpenSSH routing |
| A saved proxy name | `{ mode = "http", name = "local" }` | Use that proxy for this connection |

`--proxy` and `--no-proxy` affect inherited routes only. A profile with an explicit HTTP proxy or `direct` route keeps its own setting. `direct` and `--no-proxy` still allow `ProxyJump` or `ProxyCommand` from the effective OpenSSH configuration.

An application HTTP proxy cannot be combined with an SSH jump host. Before an HTTP-proxy connection, SSH TUI checks the effective OpenSSH configuration for `ProxyJump` and `ProxyCommand`; a conflict stops the connection. Use an alias without that route when selecting an HTTP proxy. A proxy failure never falls back to a direct connection.

Proxy DNS/TCP connection and CONNECT handshake budgets are 15 seconds each. A 407 response indicates an authentication failure; 403 indicates that the proxy denied the target. Read the session output or full error details, correct the settings, and reconnect. OpenSSH sessions use a 30-second server-alive interval and disconnect after three unanswered probes.

## Interface language

English (`en`) and Chinese (`zh`) are available. The precedence is `--language` > the saved `language` setting > English. Existing configuration files without a language setting also use English; the system locale does not select the interface language.

Press `l` in the connection or proxy manager, or `Ctrl+] l` in a remote terminal, to open the language picker. The choices are shown as `English` and `中文`. Up/Down previews the language immediately, Enter saves it, and Esc restores the previous language. Both save and cancel return to the view that opened the picker. A save failure keeps the picker open and leaves the saved configuration unchanged.

`--language en` or `--language zh` is temporary. Saving other settings does not persist that override; Enter in the language picker explicitly saves the selected language. Language changes update application labels and status without reconnecting active SSH sessions. Plain `l` is still forwarded when the remote terminal has focus.

Application menus, help, confirmations, notices, status, validation errors, and proxy password prompts are translated. User data, theme names, OpenSSH and remote output, and raw system/library diagnostics remain unchanged. The proxy helper uses the language selected when a connection starts; existing terminal contents are not rewritten, and reconnecting uses the current language. CLI help is always in English. Before configuration has loaded, application errors use an explicit CLI language override or English.

## Configuration

The default configuration file is `$XDG_CONFIG_HOME/ssh-tui/config.toml`, or `~/.config/ssh-tui/config.toml` when `XDG_CONFIG_HOME` is unset or empty. Use `--config PATH` to select another file.

A missing file starts with defaults. Saving settings creates the file using an atomic replacement with permissions `0600`. Invalid configuration is reported without overwriting the original file. Passwords are not part of the configuration format.

Example configuration:

```toml
language = "en"
theme = "Tokyo Night"
scrollback = 10000
default_proxy = "local"

[[proxies]]
name = "local"
url = "http://127.0.0.1:7890"
username = ""

[[connections]]
name = "work-server"
host = "my-ssh-alias"
user = ""
identity = ""
jump = ""
route = { mode = "inherit" }
```

Omit `port` to use OpenSSH's effective port, or add `port = 2222` to override it. Empty `user` and `identity` fields leave those choices to OpenSSH. The `jump` field accepts comma-separated hops and `user@host:port` syntax; bracket IPv6 addresses when specifying a port. Scrollback can be configured up to 100,000 lines per tab.

Connection names must be unique, and proxy names must be unique. Renaming a proxy updates its references. Before deleting a proxy explicitly referenced by a connection, change that connection's route. See [examples/config.toml](examples/config.toml) for inherited, direct, and explicit HTTP-proxy profiles.

## Color themes

Built-in themes:

- Tokyo Night (default)
- Dracula
- Nord
- Gruvbox Dark
- Solarized Dark
- Solarized Light

One theme controls both the application UI and terminal colors. Default foreground/background and ANSI colors 0–15 follow the theme; explicit indexed colors 16–255 and RGB colors retain the remote application's choices. The picker previews a shell prompt, a CJK directory name, errors, selected rows, and ANSI colors. Wide windows show the preview beside the theme list; narrow windows stack them.

### Custom themes

Place custom `*.toml` files in a `themes/` directory beside the selected configuration file, then restart. For the default configuration, this is `$XDG_CONFIG_HOME/ssh-tui/themes/` or `~/.config/ssh-tui/themes/`. Theme names must be unique across built-in and custom themes.

Each theme defines `name`, `foreground`, `background`, `accent`, `muted`, and a `palette` of exactly 16 `#RRGGBB` colors. See the complete [Amber theme example](examples/themes/amber.toml). Try the repository's bundled example with:

```bash
./target/release/ssh-tui --config ./examples/config.toml --theme Amber
```

The following UI roles are optional. Append them to a complete theme file to override derived defaults; themes without this section remain supported.

```toml
[ui]
surface = "#242633"
border = "#414868"
selection_background = "#303a55"
selection_foreground = "#c0caf5"
success = "#9ece6a"
attention = "#e0af68"
failure = "#f7768e"
```

Application text colors are adjusted to at least 4.5:1 contrast against their actual UI backgrounds, while borders remain subdued. This adjustment applies to UI text; terminal theme colors and remote explicit colors follow the rules above.

## Development and validation

Run the Rust checks from the repository root:

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test --locked
cargo build --release --locked
```

Interactive acceptance tests require Python 3.11 or newer, the dependencies in [tests/requirements.txt](tests/requirements.txt), and local `ssh`, Bash, `top`, `less`, and `vi` commands. Install the Python dependencies in a virtual environment:

```bash
python3 -m venv --system-site-packages .venv
.venv/bin/pip install -r tests/requirements.txt
.venv/bin/python tests/e2e.py --binary target/release/ssh-tui --language en --output target/e2e-en
.venv/bin/python tests/e2e.py --binary target/release/ssh-tui --language zh --output target/e2e-zh
```

The script uses loopback SSH and HTTP-proxy fixtures, temporary keys, isolated SSH configuration, and separate `known_hosts` files. It does not use personal credentials or connect to real servers. Reports and terminal text snapshots are written to `target/e2e/` by default; use `--output PATH` to choose another directory.

UI golden snapshots cover both languages at 80×24, 120×36, 48×12, and 24×6 and are stored in [tests/snapshots](tests/snapshots). Recorded local acceptance results are available in [docs/validation.md](docs/validation.md) (Chinese) and the [machine-readable report](docs/e2e-release.json). Local fixture results do not establish compatibility with every production SSH server, enterprise proxy, or outer terminal.

## Current limitations

This version supports Linux and depends on the system `ssh` command. It does not include HTTPS/HTTP2 proxy transport, SOCKS, NTLM/Kerberos proxy authentication, combined HTTP-proxy and SSH-jump routing, split panes, SFTP, a port-forwarding management UI, mouse interaction, or system clipboard integration.

## License

[MIT](LICENSE).
