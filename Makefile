.DEFAULT_GOAL := help
CARGO ?= cargo

.PHONY: help dev run build build-release check test fmt fmt-check lint verify clean \
	package package-linux windows-bootstrap windows-build release release-major \
	release-minor release-patch

help: ## List available targets
	@awk 'BEGIN {FS = ":.*## "; printf "Usage: make <target>\n\nTargets:\n"} /^[a-zA-Z0-9_.-]+:.*## / {printf "  %-20s %s\n", $$1, $$2}' $(MAKEFILE_LIST)

dev: run ## Run the app in the development profile

run: ## Run the app in the development profile
	$(CARGO) run

build: ## Build the development binary
	$(CARGO) build

build-release: ## Build the locked release binary
	$(CARGO) build --release --locked

check: ## Run a fast compile check
	$(CARGO) check

test: ## Run all tests
	$(CARGO) test

fmt: ## Format Rust source
	$(CARGO) fmt

fmt-check: ## Check Rust formatting
	$(CARGO) fmt -- --check

lint: ## Run Clippy across all targets
	$(CARGO) clippy --all-targets

verify: fmt-check check test ## Run the main local checks

clean: ## Remove Cargo build output
	$(CARGO) clean

package: ## Build package format(s), for example: make package FORMAT=appimage
	@test -n "$(FORMAT)" || { echo "Set FORMAT, for example: make package FORMAT=appimage" >&2; exit 2; }
	$(CARGO) packager --release --formats "$(FORMAT)"

package-linux: ## Build AppImage, deb, and pacman packages
	$(CARGO) packager --release --formats appimage,deb,pacman

windows-bootstrap: ## Install Windows GNU cross-build tools
	./scripts/bootstrap_windows_gnu_toolchain.sh

windows-build: ## Build the Windows GNU executable
	./scripts/build_windows_gnu_exe.sh

release: ## Create a release commit and tag; use BUMP=major|minor|patch
	@test -n "$(BUMP)" || { echo "Set BUMP to major, minor, or patch" >&2; exit 2; }
	./scripts/release.sh "$(BUMP)"

release-major: ## Create a major release commit and tag
	./scripts/release.sh major

release-minor: ## Create a minor release commit and tag
	./scripts/release.sh minor

release-patch: ## Create a patch release commit and tag
	./scripts/release.sh patch
