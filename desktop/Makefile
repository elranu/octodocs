.PHONY: run build check clean reset-state

run:
	cargo run -p octodocs-app

build:
	cargo build -p octodocs-app

check:
	cargo check -p octodocs-app

clean:
	cargo clean

reset-state:
	@if command -v secret-tool >/dev/null 2>&1; then \
		secret-tool clear service octodocs user github-token >/dev/null 2>&1 || true; \
	fi
	@rm -f "$${XDG_CONFIG_HOME:-$$HOME/.config}/octodocs/token"
	@rm -f "$${XDG_CONFIG_HOME:-$$HOME/.config}/octodocs/github_bindings.tsv"
	@rm -f "$${XDG_CONFIG_HOME:-$$HOME/.config}/octodocs/ui_state.tsv"
	@echo "OctoDocs auth/state reset complete"
