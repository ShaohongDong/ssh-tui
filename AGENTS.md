# Repository Guidelines

## Project Structure & Module Organization

- `src/` contains the Rust application: `main.rs` handles the CLI, `app.rs` coordinates state, `ui.rs` renders views, and `session.rs`, `ssh.rs`, `terminal.rs`, and `proxy.rs` manage connections. Configuration, themes, input, and translations have separate modules.
- Rust unit tests live beside implementation code; `src/app_ui_tests.rs` tests rendering. `tests/` holds CLI integration tests, Python acceptance and packaging tests, and `snapshots/` golden files.
- `examples/` contains sample TOML configuration and themes; `docs/` contains design notes, screenshots, and validation evidence. `scripts/package_release.py` packages releases; `.github/workflows/build.yml` defines CI. Generated output belongs in ignored `target/` or `dist/` directories.

## Build, Test, and Development Commands

Use Linux or macOS, Rust 1.88+, and system OpenSSH. Interactive execution requires a terminal. Python tooling requires Python 3.11+.

```bash
cargo build --release --locked         # Build target/release/ssh-tui
cargo run --locked -- --language en    # Run the development build
cargo fmt --all --check                # Check Rust formatting
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked                    # Run Rust unit/integration tests
python3 -m unittest discover -s tests -p 'test_release.py' -v
```

Clippy treats warnings as failures; the Python command checks release packaging.

## Coding Style & Naming Conventions

Follow Rust 2024 conventions and rustfmt formatting (four-space indentation). Use `snake_case` for modules/functions, `PascalCase` for types, and `SCREAMING_SNAKE_CASE` for constants. Match existing Python four-space indentation and `test_*` method naming. Add application text through `src/i18n.rs`, maintaining English and Chinese together.

## Testing Guidelines

Use Rust `#[test]`, Ratatui's `TestBackend`, and Python `unittest`; no numeric coverage threshold is configured. Name tests after observable behavior. For interactive changes, install `tests/requirements.txt` in a virtual environment and run acceptance in both languages:

```bash
for lang in en zh; do
  .venv/bin/python tests/e2e.py --binary target/release/ssh-tui --language "$lang" --output "target/e2e-$lang"
done
```

See README for fixture prerequisites. Review snapshot differences; regenerate intentional UI changes with `UPDATE_UI_SNAPSHOTS=1 cargo test --locked ui_snapshots_cover_responsive_views`.

## Commit & Pull Request Guidelines

The short history uses descriptive subjects, including `ci: build and release Linux and macOS binaries`. Prefer concise, imperative subjects and focused commits. PRs should explain behavior changes, link relevant issues, list validation performed, and include screenshots or snapshot diffs for UI changes. Identify platform checks still pending.

## Security & Configuration Tips

Keep credentials and personal configuration out of commits. Preserve OpenSSH host-key verification, proxy-failure behavior without direct fallback, and atomic configuration writes with `0600` permissions. Use isolated loopback fixtures for acceptance tests.
