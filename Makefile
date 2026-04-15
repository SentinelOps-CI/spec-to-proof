.PHONY: help dev build test lint clean install deps docker

# Default target
help:
	@echo "Spec-to-Proof development commands (Cargo + npm; see CONTRIBUTING.md)"
	@echo "================================================================"
	@echo "make build     - cargo build --workspace"
	@echo "make test      - cargo test --workspace && npm test"
	@echo "make lint      - ./scripts/ci-lint.sh"
	@echo "make clean     - cargo clean; remove node/.next artifacts"
	@echo "make install   - npm ci"
	@echo "make docker    - docker build lean-farm image"

dev:
	@echo "Start components you need, for example:"
	@echo "  cargo run -p nlp --bin invariant_extractor"
	@echo "  cargo run -p proof --bin lean_compiler"
	@echo "  npm run dev   # Next.js (platform/ui)"

build:
	cargo build --workspace

test:
	cargo test --workspace
	npm test

lint:
	./scripts/ci-lint.sh

clean:
	cargo clean
	rm -rf node_modules .next platform/ui/.next
	@echo "Clean completed"

install:
	npm ci

deps:
	npm update

docker:
	docker build -f lean-farm/Dockerfile -t spec-to-proof/lean-farm:local lean-farm

format:
	npm run format 2>/dev/null || true
	cargo fmt --all

type-check:
	npm run type-check

security:
	cargo audit
	npm audit

bench:
	@echo "Benchmarks are not wired in this Makefile; see crate-specific benches."

docs:
	@echo "See README.md and docs/adr/"

status:
	@echo "Rust: $$(rustc --version 2>/dev/null || echo not installed)"
	@echo "Node: $$(node --version 2>/dev/null || echo not installed)"
	@echo "Branch: $$(git branch --show-current 2>/dev/null || echo unknown)"

smoke-qa:
	./scripts/smoke-qa.sh
