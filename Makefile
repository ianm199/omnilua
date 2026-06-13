# lua-rs: one way to build and test.
#
#   make test          build + Rust tests + conformance (what CI runs)
#   make rust          workspace unit/integration tests + embedding doctests
#   make conformance   official Lua 5.4 suite against the omnilua binary
#   make conformance-release  same suite, RELEASE-profile binary (#140: release
#                      cadence hid a GC use-after-free the debug suite missed)
#   make rooting-battery      quarantine/stress GC rooting battery (#140 P0)
#   make conformance-55 official Lua 5.5 suite against the omnilua binary
#   make lua55-reference build Lua 5.5 compat-on and compat-off C references
#   make oracle-55     behavioral diff snippets vs reference C Lua 5.5.0
#   make parity        behavioral diff vs reference C Lua 5.4.7 (the oracle)
#   make perf          benchmark vs reference C Lua (measurement, not a gate)
#   make scaling       flag superlinear (O(n^2)) behavior in hot operations
#   make profile F=... sample a hotpath profile of running script F (needs samply)
#   make build         debug omnilua binary
#   make setup         bootstrap the conformance test directory
#
# `make test` is the gate. It builds its own binary (no stale-binary trap)
# and runs exactly what CI runs.

CARGO ?= cargo
TEST_TIMEOUT_S ?= 90
export TEST_TIMEOUT_S

.PHONY: help test build setup rust conformance conformance-release rooting-battery conformance-55 lua55-reference oracle-55 parity perf scaling profile clean

help:
	@grep -E '^#   make ' Makefile | sed 's/^#   /  /'

test: build rust conformance conformance-release
	@echo "== all gates passed =="

build:
	$(CARGO) build --bin omnilua

# reference/lua-c is gitignored; recreate the testes symlink the harness
# expects, pointing at the committed test files. Idempotent.
setup:
	@mkdir -p reference/lua-c
	@ln -sfn ../lua-5.4.7-tests reference/lua-c/testes
	@echo "conformance tests available: $$(ls reference/lua-c/testes/*.lua 2>/dev/null | wc -l | tr -d ' ')"

rust:
	$(CARGO) test --workspace --lib --tests
	$(CARGO) test -p omnilua --doc

conformance: build setup
	./harness/run_official_all.sh

# Release-profile suite run. Optimized code changes allocation sizes and GC
# cadence, which re-rolls whether a rooting bug bites: #140 bug A segfaulted
# EVERY release run of db.lua while the debug suite stayed green. This gate
# would have caught it on day one.
conformance-release: setup
	$(CARGO) build --release --bin omnilua
	LUA_RS_BIN=$(CURDIR)/target/release/omnilua ./harness/run_official_all.sh

# GC exact-rooting battery (docs/EXACT_ROOTING_SPEC.md P0): quarantine +
# stress instruments over canaries and the repro set. Findings exit 1 with
# evidence saved. Stress-config findings are expected until #140 bug B is
# fixed (P4); the quarantine configs must stay clean and gate CI
# (asan-stress.sh --quarantine-only).
rooting-battery: build
	bash harness/asan-stress.sh

conformance-55: build
	./harness/run_official_all.sh --version 5.5 --tests-dir reference/lua-5.5.0-tests

lua55-reference:
	bash harness/build_lua55_compat_off.sh

oracle-55: build lua55-reference
	./specs/oracle/check.sh 5.5

# Behavioral parity oracle: same wrapped test through lua-rs AND reference C
# 5.4.7, diff normalized stdout+exit. Exits nonzero on any divergence.
# This is the truth-teller `conformance` (no-crash) cannot be.
parity: build setup
	./harness/parity_check.sh

perf:
	@[ -x reference/lua-5.4.7/src/lua ] || $(MAKE) -C reference/lua-5.4.7 guess
	$(CARGO) build --release --bin omnilua
	bash harness/bench/compare.sh

# The conformance gate must run the PGO binary it just built — the script's
# default is target/debug/omnilua, which silently gated a stale debug binary
# locally and is absent entirely on CI runners (0.0.33 dashboard job failure).
perf-pgo: setup
	@[ -x reference/lua-5.4.7/src/lua ] || $(MAKE) -C reference/lua-5.4.7 guess
	bash harness/bench/build-pgo.sh
	LUA_RS_BIN=$(CURDIR)/target/release/omnilua ./harness/run_official_all.sh
	BENCH_VARIANT=pgo bash harness/bench/compare.sh
	$(CARGO) build --release --bin omnilua

bytecode-parity:
	$(CARGO) build --release --bin omnilua -q
	python3 harness/bench/bytecode-parity.py

scaling:
	$(CARGO) build --release --bin omnilua
	python3 harness/bench/scaling-check.py

# Hotpath profile of a script: make profile F=harness/bench/workloads/gc_pressure.lua
profile:
	@command -v samply >/dev/null 2>&1 || { echo "samply not found: cargo install samply"; exit 1; }
	@[ -n "$(F)" ] || { echo "usage: make profile F=<script.lua>"; exit 1; }
	$(CARGO) build --release --bin omnilua
	samply record target/release/omnilua $(F)

clean:
	$(CARGO) clean
