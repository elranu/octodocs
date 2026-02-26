# Reset local auth/state

Use this to test first-run onboarding from a clean local state.

```bash
cd desktop
make reset-state
```

What it clears:
- GitHub token file (`~/.config/octodocs/token`)
- GitHub bindings (`~/.config/octodocs/github_bindings.tsv`)
- UI persisted state (`~/.config/octodocs/ui_state.tsv`)
- Linux keyring token (best-effort via `secret-tool`, if installed)

Then run:

```bash
cargo run -p octodocs-app
```
