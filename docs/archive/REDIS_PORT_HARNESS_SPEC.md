# Redis Port Harness Spec

Status: draft, 2026-05-16.

This note specifies how to apply the current Lua porting technique to Redis as
the next target, and what the harness needs to become if the intended product is
"source repo plus test suite in, target-language port out."

The concrete target assumed here is Redis C to Rust. The methodology should stay
language-pair agnostic.

## Research Basis

Pinned upstream source used for this pass:

- Repository: `https://github.com/redis/redis`
- Branch: `unstable`
- Commit: `b1a53ea21f6ba49bb98dcfd405ef507ff3187573`
- Local research clone: `/tmp/redis-port-research`

Local facts from that checkout:

- Top-level `src`: 125 `.c` files, 82 `.h` files, about 182k lines of C across
  top-level `.c` files.
- Largest source files by line count: `module.c` 15.7k, `redis-cli.c` 11.1k,
  `server.c` 8.2k, `cluster_legacy.c` 6.5k, `t_stream.c` 6.2k,
  `networking.c` 5.9k, `sentinel.c` 5.5k, `replication.c` 5.4k, `t_zset.c`
  5.0k, `rdb.c` 4.9k.
- Tests: 222 Tcl files, 45 module test C files, 9 RDB assets.
- Command metadata: 442 JSON command specs under `src/commands`.
- Command groups in the JSON specs: server 84, cluster 35, sorted_set 35,
  generic 34, stream 30, hash 28, connection 26, string 26, scripting 23,
  list 22, sentinel 22, array 18, set 17, pubsub 15, geo 10, bitmap 7,
  hyperloglog 5, transactions 5.
- Current checkout builds locally with:

```sh
make -j4 BUILD_TLS=no DISABLE_WERRORS=yes
```

I also sanity-checked official external-server test mode by launching the C
server with `--enable-debug-command yes` and running:

```sh
./runtest --host 127.0.0.1 --port 6391 --single unit/protocol --clients 1 --timeout 120 --tags -needs:repl
```

That `unit/protocol` external-mode run passed.

Primary upstream notes:

- Redis README documents build flags, including plain `make`, TLS, module builds,
  and `make test`: `https://github.com/redis/redis/blob/unstable/README.md`
- Redis tests support external-server mode via `--host` and `--port`, and
  provide tags such as `external:skip`, `needs:repl`, `needs:debug`, and
  `needs:save`: `https://github.com/redis/redis/blob/unstable/tests/README.md`
- `src/commands/README.md` says the command JSON files generate internal command
  metadata and `commands.def`; for third-party command info, Redis wants consumers
  to use `COMMAND INFO` / `COMMAND DOCS`: `https://github.com/redis/redis/blob/unstable/src/commands/README.md`
- Current Redis 8+ source is tri-licensed RSALv2 / SSPLv1 / AGPLv3, while Redis
  7.2 and prior were BSD-3 according to `LICENSE.txt`:
  `https://github.com/redis/redis/blob/unstable/LICENSE.txt`

## Thesis

Redis is a strong next harness target because it has three properties Lua did
not have at this scale:

1. A network protocol boundary. RESP gives us a clean differential oracle: feed
   the same bytes to C Redis and the Rust port, compare normalized RESP frames.
2. A serious official test suite with external-server mode. We can use real Tcl
   tests before the Rust server can spawn its own subprocess topologies.
3. Machine-readable command metadata. The command JSON and generated
   `commands.def` let the harness enforce command arity, flags, key specs,
   ACL categories, tips, and reply schemas.

Redis is also a warning shot before nginx. It has real C global state, forked
background persistence, custom data structures, modules, cluster, sentinel,
embedded Lua, command metadata generation, and nondeterministic outputs. If the
harness can keep agents coherent here, nginx becomes a larger instance of a
known class rather than a leap.

## Formal Goal

Let:

- `S` be a source repository at pinned commit `c`.
- `T` be its upstream test suite.
- `L` be a target language and runtime platform.
- `P` be the generated target-language project.
- `O` be a set of executable oracles derived from `S` and `T`.

The porting harness must transform `(S, T, L, c)` into `P` such that:

1. `P` builds under its target toolchain.
2. `P` exposes the same externally-observable public interfaces selected for
   the current phase.
3. `P` passes the phase's selected subset of `T`, either directly or through a
   harness adapter.
4. `P` matches `S` under differential oracles for the selected behavioral
   surface.
5. Every cross-boundary semantic object has exactly one canonical owner in `P`.
6. Every agent change includes machine-checkable evidence: build result, hook
   result, test result, or oracle result.

For Redis specifically, phase 1 `P` is not "all Redis." It is a Rust workspace
that can run a `redis-server` binary compatible enough to pass protocol and
basic keyspace tests. Later phases expand the compatibility frontier.

## Architecture

High-level harness loop:

```text
        upstream source repo S                  upstream tests T
        pinned commit c                         Tcl, assets, module C
               |                                       |
               v                                       v
   +-------------------------+             +-------------------------+
   | source/test ingestion   |             | test catalog builder    |
   | clang/tree-sitter/json  |             | tags, fixtures, costs   |
   +-----------+-------------+             +------------+------------+
               |                                        |
               v                                        v
   +--------------------------------------------------------------+
   | committed harness facts                                      |
   | types.tsv, functions.tsv, macros.tsv, command-registry.json  |
   | test-catalog.json, vocab registries, phase gates             |
   +----------------------------+---------------------------------+
                                |
                                v
   +--------------------------------------------------------------+
   | work packet generator                                         |
   | section chunks, owner crate, allowed writes, oracle target    |
   +----------------------------+---------------------------------+
                                |
              +-----------------+-----------------+
              |                 |                 |
              v                 v                 v
        Translator        Compiler-fixer       Test-fixer
        claude -p         claude -p            claude -p
              |                 |                 |
              +-----------------+-----------------+
                                |
                                v
   +--------------------------------------------------------------+
   | hooks and validators                                          |
   | no stubs, vocabulary, cargo, generated-code sync, oracle log  |
   +----------------------------+---------------------------------+
                                |
                                v
   +--------------------------------------------------------------+
   | evidence ledger                                               |
   | per packet JSONL, test artifacts, diffs, verifier summaries   |
   +--------------------------------------------------------------+
```

Redis-specific target workspace:

```text
crates/redis-types
  ByteString, RespValue, RedisError, RedisResult, TimeSpec, DbIndex
        |
        +------------------+
        |                  |
        v                  v
crates/redis-ds       crates/redis-protocol
  sds-like bytes        RESP2/RESP3 parser/serializer
  dict/intset           client-visible frame types
  listpack/quicklist
  rax/streams support
        |                  |
        +---------+--------+
                  v
          crates/redis-core
            RedisServer, Client, RedisDb, RedisObject
            expiry, keyspace, command context, config
                  |
        +---------+----------+-------------+
        |                    |             |
        v                    v             v
crates/redis-commands  crates/redis-persist  crates/redis-repl
  generated registry     RDB/AOF/rio          replication/PSYNC
  command impls
        |
        v
crates/redis-server
  binary: redis-server
  event loop, networking, process config

later:
crates/redis-cluster
crates/redis-sentinel
crates/redis-modules
crates/redis-cli
crates/redis-benchmark
```

The first pass should not split into too many crates. The split exists to
prevent semantic duplication and dependency cycles, not to mirror every C file.
If a type is shared by more than one subsystem, it must live in an owner crate
and be registered before body translation starts.

## What Carries Over From Lua

Keep these mechanics unchanged:

- Static `PORTING.md` as the agent-facing contract.
- Precomputed analyses instead of asking every agent to rediscover cross-file
  facts.
- Phase split: translate shape, compile, test, then harden.
- Work packets sized by syntactic boundaries, not whole giant files.
- Per-worker worktrees and temp directories.
- Hooks scoped to the changed packet, not global scans that race other workers.
- Verifier role with no write tools.
- Evidence-first phase gates.
- `claude -p` invocations for heavy work, with bounded tools and budgets.

Add these Redis-specific layers:

- RESP wire differential oracle.
- Command registry enforcement.
- Official Tcl test adapter.
- Binary shim mode for tests that exec `src/redis-server`, `src/redis-cli`, or
  `src/redis-benchmark`.
- Dataset digest and RDB/AOF oracles.
- Nondeterministic-output normalizers.
- Module ABI boundary planning.

## Ingestion Outputs

The harness should commit these generated facts before any translator runs.

### Source Inventory

`ANALYSES/redis/source-inventory.json`

Fields:

- Source file path.
- Header dependencies.
- Function symbols with line ranges.
- Type declarations with owner candidate.
- Macro declarations.
- Global variables.
- Generated file status.
- Estimated difficulty score.

Difficulty score should weight:

- LoC.
- Number of external symbols.
- Pointer-heavy code.
- Macro density.
- Test coverage density.
- Runtime/OS coupling.
- Whether the file owns public cross-crate types.

### Type Vocabulary

`harness/redis/type-vocabulary.tsv`

Canonical examples:

```text
name                    owner
RedisServer             crates/redis-core/src/server.rs
Client                  crates/redis-core/src/client.rs
RedisDb                 crates/redis-core/src/db.rs
RedisObject             crates/redis-core/src/object.rs
RedisString             crates/redis-types/src/string.rs
RespFrame               crates/redis-protocol/src/frame.rs
CommandSpec             crates/redis-commands/src/spec.rs
CommandContext          crates/redis-core/src/command_context.rs
StreamId                crates/redis-ds/src/stream.rs
ListPack                crates/redis-ds/src/listpack.rs
QuickList               crates/redis-ds/src/quicklist.rs
RadixTree               crates/redis-ds/src/rax.rs
ModuleApi               crates/redis-modules/src/api.rs
```

The Lua failure we just hit makes this mandatory. Redis will otherwise produce
fake local `Client`, `RedisObject`, `RedisDb`, `Command`, `RespValue`, and
`Server` structs everywhere.

### API Vocabulary

`harness/redis/api-vocabulary.tsv`

This catches semantic drift beyond nominal types:

- Command entry signatures.
- Reply writer API.
- Object lookup/update API.
- Expiry API.
- Propagation/AOF API.
- Config getter/setter API.
- Module API surface.

Example:

```text
symbol                         owner                                      signature
lookup_key_read                crates/redis-core/src/db.rs                (&RedisDb, &RedisString) -> Option<&RedisObject>
lookup_key_write               crates/redis-core/src/db.rs                (&mut RedisDb, &RedisString) -> Option<&mut RedisObject>
add_reply                      crates/redis-core/src/reply.rs             (&mut Client, RespFrame) -> RedisResult<()>
set_command                    crates/redis-commands/src/string.rs        (&mut CommandContext) -> RedisResult<()>
```

The hook rejects new public functions with registered names outside their owner
and rejects signature drift unless the architect updates the registry.

### Command Registry

`harness/redis/command-registry.json`

Generated from `src/commands/*.json` plus, when the C server is built, a live
snapshot from:

```sh
src/redis-cli --json COMMAND DOCS
src/redis-cli --json COMMAND INFO
```

Use raw JSON files for internal port planning, but use live `COMMAND` output as
the behavioral oracle because upstream explicitly warns that the raw files are
not the public consumer interface.

Enforced fields:

- Command name and subcommand name.
- Arity.
- Group.
- ACL categories.
- Command flags.
- Key specs.
- Function name in C source.
- Reply schema presence.
- Nondeterminism tips.

Generated Rust should include:

- `CommandSpec`.
- A generated command table.
- A generated parser for command argument shapes where useful.
- A generated test that compares Rust command metadata against the C oracle.

### Test Catalog

`harness/redis/test-catalog.json`

Fields:

- Test unit path, for example `unit/protocol`.
- File path.
- Tags.
- External-server compatibility.
- Needs debug command.
- Needs save/BGSAVE.
- Needs replication.
- Needs cluster.
- Needs module build.
- Uses `redis-cli` or `redis-benchmark`.
- Estimated runtime.
- Phase assignment.
- Last C baseline result.
- Last Rust result.

This file lets the orchestrator choose the smallest useful test after each
agent change, instead of reflexively running the full suite.

## Oracles

### Oracle 1: Build Oracle

Build both sides:

```sh
# C reference
make -j4 BUILD_TLS=no DISABLE_WERRORS=yes

# Rust target
cargo check --workspace
cargo test --workspace
cargo build --bin redis-server
```

Redis source builds modules as part of the normal build in this checkout. The
Rust port should delay module compatibility, but the C oracle can still provide
module test binaries.

### Oracle 2: RESP Wire Diff

Launch two servers:

- C Redis on port `p1`.
- Rust Redis on port `p2`.

Feed the same RESP byte scripts to both. Parse replies as RESP frames and
compare after normalizing allowed nondeterminism.

Classes:

- `byte_exact`: PING, ECHO, SET, GET, DEL, EXISTS, INCR, fixed errors.
- `frame_exact`: arrays/maps where byte order is stable enough after RESP parse.
- `normalized`: INFO, COMMAND, MEMORY, LATENCY, TIME, RANDOMKEY, SCAN, CLIENT.
- `state_digest`: compare `DEBUG DIGEST` or equivalent after command sequences.
- `not_yet`: commands with scripts, modules, cluster, replication.

This should be the fastest per-packet oracle. It is much cheaper than Tcl and
better localized than full integration tests.

### Oracle 3: Official Tcl External Mode

Use Redis's existing test harness against an already-running Rust server:

```sh
./runtest --host 127.0.0.1 --port <rust-port> \
  --clients 1 \
  --single unit/protocol \
  --tags -needs:repl \
  --timeout 120
```

Important config for early phases:

- Start Rust with debug command support if the selected tests need it.
- Use `--singledb` until multi-DB is implemented.
- Use `--ignore-encoding` until object encodings are faithful.
- Use `--ignore-digest` until `DEBUG DIGEST` is implemented.
- Deny `needs:repl`, `needs:save`, and module tags until their phases.

This mode is the first official-suite integration layer because it avoids
teaching the Rust server all of Redis's subprocess and temp-dir lifecycle at
the beginning.

### Oracle 4: Binary Shim Mode

Many tests exec fixed paths such as `src/redis-server`, `src/redis-cli`, and
`src/redis-benchmark`. For those, create a test overlay:

```text
harness/redis/test-overlay/
  src/redis-server      -> target/debug/redis-server
  src/redis-cli         -> either C redis-cli initially or Rust later
  src/redis-benchmark   -> C redis-benchmark initially or Rust later
  tests/                -> upstream tests
```

This unlocks tests that spawn local server processes, inspect logs, use unix
sockets, exercise persistence, and run cluster/sentinel topologies.

### Oracle 5: Request/Response Log Oracle

Redis has a hidden `req-res-logfile` configuration and `--log-req-res` test
mode. The harness should use this later as a schema oracle:

1. Run C Redis with request/response logging for selected tests.
2. Run Rust Redis with an equivalent logger.
3. Normalize nondeterministic replies.
4. Compare test-level request/reply traces.

This is useful when Tcl failures are too far away from the command that caused
the divergence.

### Oracle 6: Persistence Oracle

RDB/AOF cannot start as byte-exact. Start with:

- Load official RDB assets and compare resulting command behavior.
- Save in Rust, load in C, compare dataset digest.
- Save in C, load in Rust, compare dataset digest.
- Later, byte-diff RDB/AOF where the format is deterministic.

### Oracle 7: Topology Oracle

For replication, cluster, and sentinel:

- Launch C and Rust topologies with the same scripts.
- Compare role states, replication offsets, failover outcomes, and digest
  convergence.
- Do not expect byte-identical logs or timing.

## Hook Set

Hooks must be scoped to the packet's write set. Global scans are allowed only in
explicit audit mode.

Required hooks:

- `no-local-stubs`: reject `struct Client`, `struct RedisObject`,
  `struct RedisServer`, `enum RespFrame`, etc. outside registered owners.
- `type-vocabulary`: same shape as the Lua hook, but Redis vocabulary starts
  before translation.
- `api-vocabulary`: reject duplicated public functions and signature drift for
  registered APIs.
- `command-registry-sync`: if command JSON changes or command impl changes,
  generated registry tests must still pass.
- `generated-code-lock`: reject manual edits to generated command tables except
  through the generator.
- `dependency-edge-gate`: if a crate needs a registered owner type, the agent
  must add the dependency edge instead of stubbing.
- `unsafe-budget`: track `unsafe` by crate and by subsystem. Redis will need
  some `unsafe` around OS, modules ABI, and fork/process work, but data type
  logic should not casually accumulate it.
- `forbidden-data-string`: reject `String` for Redis keys, values, and protocol
  payloads. Redis strings are byte strings.
- `nondeterminism-tag`: tests/oracles touching nondeterministic commands must
  declare a normalizer.
- `oracle-evidence`: verifier cannot mark a phase green without a fresh artifact.

## Claude Code Execution Model

### Roles

Use explicit roles rather than generic "fix this" agents:

- `profiler`: read-only. Builds inventories, test catalog, command registry,
  phase plan. No writes outside `ANALYSES/` and `harness/redis/`.
- `architect`: owns crate graph, vocabularies, dependency edges, phase gates.
  Only this role may add a dependency between crates or mutate a vocabulary
  owner.
- `shape-translator`: creates Rust module skeletons, type definitions, public
  signatures, and TODO bodies. Does not fill large bodies.
- `body-translator`: fills one syntactic chunk against a frozen shape contract.
- `compiler-fixer`: owns one crate or one file's compile errors.
- `oracle-fixer`: owns one failing wire/Tcl test.
- `normalizer`: updates oracle normalization rules when the C source proves the
  output is intentionally nondeterministic.
- `perf-guard`: read-mostly, compares benchmark traces and flags pathological
  allocations or algorithmic drift.
- `verifier`: no write tools. Runs phase gates and emits evidence summaries.

### Work Packet

Every `claude -p` invocation should receive a packet like:

```json
{
  "packet_id": "redis-t_string-setCommand-001",
  "role": "body-translator",
  "source_commit": "b1a53ea21f6ba49bb98dcfd405ef507ff3187573",
  "source_slices": [
    {"path": "src/t_string.c", "lines": "433-620"},
    {"path": "src/object.h", "lines": "100-180"}
  ],
  "target_files": [
    "crates/redis-commands/src/string.rs"
  ],
  "read_only_context": [
    "PORTING.md",
    "ANALYSES/redis/types.tsv",
    "harness/redis/type-vocabulary.tsv",
    "harness/redis/command-registry.json"
  ],
  "frozen_api": [
    "CommandContext",
    "RedisObject",
    "lookup_key_write",
    "add_reply"
  ],
  "allowed_commands": [
    "cargo check -p redis-commands",
    "harness/redis/oracle/wire-diff --case string-set"
  ],
  "done_when": [
    "cargo check -p redis-commands passes",
    "wire diff case string-set passes",
    "no vocabulary hook failures"
  ]
}
```

The packet is the real unit of work. "Translate `t_string.c`" is too large and
too ambiguous.

### Prompt Cache

Cache as much static context as possible:

- 1 hour cache: `PORTING.md`, Rust workspace conventions, crate graph,
  vocabulary registry, command registry schema.
- 5 minute cache: current source slice, direct headers, test slice, packet.
- Uncached: the concrete instruction and current failure output.

### Model Tiering

Use stronger models for architecture and initial difficult slices. Use cheaper
models for body packets once the shape contract is frozen.

Likely assignment:

- Architect/profiler: strongest available model.
- Shape-translator: strong model.
- Body-translator: cheaper model for small chunks, escalate on repeated failure.
- Compiler-fixer: cheaper model for localized errors, strong model for borrow
  model redesign.
- Verifier: cheapest reliable model, no write tools.

## Contingent Event Graph

```text
packet completed
      |
      v
run scoped hooks
      |
      +-- duplicate type/API --> architect packet
      |
      +-- generated drift ----> generator packet
      |
      +-- unsafe budget ------> owner packet or architect exception
      |
      v
run scoped build
      |
      +-- compile fail -------> compiler-fixer packet
      |
      v
run cheapest relevant oracle
      |
      +-- byte/frame diff ----> oracle-fixer packet
      |
      +-- nondeterminism -----> normalizer packet, then rerun
      |
      +-- missing feature ----> phase planner decides defer vs implement
      |
      v
append evidence ledger
      |
      v
verifier samples/approves phase gate
```

The key rule: an agent that cannot reach a needed type or API is not allowed to
invent it. It either imports the canonical owner or escalates to the architect.

## Phase Plan

### Phase 0: Source, License, and Oracle Freeze

Outputs:

- `harness/redis/source.toml` with repo URL, commit, license note, build
  command, test commands.
- C Redis build artifact.
- C baseline for a small official test subset.
- Decision on source line: current Redis 8+ vs older BSD-3 Redis vs another
  compatible upstream. This is a legal/product decision, not an agent decision.

Gate:

- C source builds.
- `unit/protocol` external mode passes against C.
- License choice documented.

### Phase 1: Ingestion and Registries

Outputs:

- Source inventory.
- Type vocabulary.
- API vocabulary.
- Command registry from JSON plus live `COMMAND` oracle snapshot.
- Test catalog with phase assignments.
- Initial Rust workspace skeleton.

Gate:

- Registry generators are reproducible.
- Hooks can intentionally catch a fake duplicate `Client` or `RespFrame`.
- Test catalog can list, filter, and run one C baseline test.

### Phase 2: RESP and Server Skeleton

Scope:

- RESP2/RESP3 frame model.
- Parser and serializer.
- Minimal TCP server.
- Client state.
- PING, ECHO, HELLO, COMMAND metadata stubs sufficient for protocol tests.

Tests:

- Wire diff for raw protocol cases.
- External `unit/protocol` subset, with unsupported tests explicitly skipped or
  phased.

Gate:

- Rust `redis-server` starts, accepts TCP, replies to basic commands.
- Wire diff passes for basic RESP cases.

### Phase 3: Core Keyspace and Strings

Scope:

- `RedisDb`.
- `RedisObject`.
- Byte-string handling.
- SET, GET, DEL, EXISTS, TYPE, EXPIRE, TTL, INCR/DECR basics.
- Error text compatibility for common failures.

Tests:

- Wire diff command scripts.
- `unit/type/string`.
- `unit/keyspace`.
- selected `unit/expire`.

Gate:

- Basic string/keyspace official tests pass in external mode.
- No local stubs for core types.

### Phase 4: Core Data Structures

Scope:

- Hash, list, set, sorted set.
- Internal encodings only as needed for behavior at first.
- SCAN-family semantics.

Tests:

- `unit/type/hash`
- `unit/type/list`
- `unit/type/set`
- `unit/type/zset`
- `unit/scan`
- generated wire/property tests by command group.

Gate:

- Data type groups pass with `--ignore-encoding` if necessary.
- Later gate removes `--ignore-encoding`.

### Phase 5: Streams, Pub/Sub, Transactions, Blocking

Scope:

- Streams and consumer groups.
- Pub/sub and shard pub/sub.
- MULTI/EXEC/DISCARD/WATCH.
- Blocking list/zset/stream commands and client wakeups.

Tests:

- `unit/type/stream`
- `unit/type/stream-cgroups`
- `unit/pubsub`
- `unit/pubsubshard`
- `unit/multi`
- selected blocking tests.

Gate:

- Event-loop and blocked-client behavior is stable enough for official tests.

### Phase 6: Persistence

Scope:

- RDB load/save.
- AOF rewrite and recovery.
- `rio` equivalents.
- Fork/process model decision.

Tests:

- `integration/rdb`
- `integration/aof`
- `unit/aofrw`
- RDB assets round-trip.

Gate:

- C-to-Rust and Rust-to-C persistence round trips produce equivalent datasets.

### Phase 7: Scripting and Functions

Scope:

- Redis Lua scripting bridge.
- Script cache.
- Functions.
- Determinism rules.

Tests:

- `unit/scripting`
- `unit/functions`
- cluster scripting later.

Gate:

- Existing Lua Rust port decision made: embed our port, bind to C Lua, or use an
  existing Lua crate as a temporary compatibility bridge.

### Phase 8: Replication

Scope:

- SYNC/PSYNC.
- Replica state.
- Backlog.
- Diskless/load behavior.
- WAIT/WAITAOF.

Tests:

- `integration/replication*`
- `integration/psync*`
- `unit/wait`

Gate:

- Master/replica digest convergence across selected scenarios.

### Phase 9: Cluster and Sentinel

Scope:

- Cluster slots, gossip, failover, migration.
- Sentinel monitoring and failover.

Tests:

- `runtest-cluster`
- `runtest-sentinel`
- `unit/cluster/*`

Gate:

- Topology oracle confirms convergence and expected failover state.

### Phase 10: Modules API

Scope:

- `redismodule.h` ABI.
- Module loading.
- Module data types.
- Blocking module clients.
- Module timers/hooks/events.

Tests:

- `runtest-moduleapi`
- `tests/modules/*.c`

Gate:

- Module test shared objects load into Rust Redis through a compatible ABI, or
  the product explicitly scopes modules out.

This is one of the highest-risk phases. Do not let body translators improvise
the module API. It needs an architect-owned ABI contract.

### Phase 11: Performance and Conformance Tightening

Scope:

- `redis-benchmark` comparison.
- Allocation profiles.
- Memory command accuracy.
- Encoding checks.
- TLS/systemd/platform flags if in scope.

Gate:

- Remove `--ignore-encoding` and `--ignore-digest` for core tests.
- Establish acceptable performance envelopes per command group.

## First Pilot

The first Redis pilot should be intentionally narrow:

1. Build C Redis and freeze commit.
2. Generate command registry from `src/commands/*.json`.
3. Scaffold Rust workspace with `redis-types`, `redis-protocol`,
   `redis-core`, `redis-commands`, `redis-server`.
4. Implement RESP parser/serializer and a single-threaded TCP loop.
5. Implement PING, ECHO, HELLO, COMMAND enough for tests.
6. Implement SET/GET/DEL/EXISTS/INCR minimally.
7. Run:

```sh
cargo check --workspace
harness/redis/oracle/wire-diff --suite smoke
./runtest --host 127.0.0.1 --port <rust-port> --single unit/protocol --clients 1 --timeout 120 --tags -needs:repl
```

8. Add `unit/type/string` only after protocol is stable.

Pilot success is not "Redis works." Pilot success is:

- agents can work from packets;
- hooks prevent duplicate cross-cutting types;
- command metadata is generated, not hand-maintained;
- wire diff catches real behavior drift;
- one official test unit passes against Rust;
- the evidence ledger is useful enough to drive the next packet automatically.

## Productized Harness Shape

The reusable system should be organized around a `port.toml`:

```toml
[source]
repo = "https://github.com/redis/redis"
commit = "b1a53ea21f6ba49bb98dcfd405ef507ff3187573"
language = "c"

[target]
language = "rust"
workspace = "redis-rs-port"

[build.reference]
command = "make -j4 BUILD_TLS=no DISABLE_WERRORS=yes"

[build.target]
command = "cargo check --workspace"

[tests.reference]
list = "./runtest --list-tests"
smoke = "./runtest --single unit/protocol --clients 1 --timeout 120"

[oracles]
wire_diff = "harness/redis/oracle/wire-diff"
tcl_external = "harness/redis/oracle/run-tcl-external"
```

From that, the harness generates:

- `ANALYSES/<target>/...`
- `harness/<target>/type-vocabulary.tsv`
- `harness/<target>/api-vocabulary.tsv`
- `harness/<target>/test-catalog.json`
- `harness/<target>/packets/*.json`
- `.claude/skills/<target>-port/SKILL.md`
- `.claude/agents/<target>-*.md`
- `.claude/hooks/<target>-*.sh`

The long-term UX is:

```sh
portkit ingest port.toml
portkit plan --phase pilot
portkit dispatch --workers 16 --engine claude-code
portkit monitor
portkit verify --phase pilot
```

## Skill Packaging

For Claude Code, package Redis-specific behavior as a skill:

```text
.claude/skills/redis-port/SKILL.md
.claude/skills/redis-port/references/resp.md
.claude/skills/redis-port/references/command-registry.md
.claude/skills/redis-port/references/object-model.md
.claude/skills/redis-port/scripts/make_packet.py
.claude/skills/redis-port/scripts/run_wire_diff.py
```

The skill should teach:

- Redis strings are bytes, not UTF-8 text.
- Do not invent `Client`, `RedisObject`, `RedisDb`, `RedisServer`, `RespFrame`,
  or `CommandSpec`.
- Every command implementation takes a `CommandContext`.
- Reply behavior is oracle-driven.
- Command metadata comes from the generator.
- Tests must not be edited to pass.
- If a needed type is unreachable, ask for a dependency edge.

The skill should not contain a giant Redis manual. It should point to registries
and packet-local source slices.

## Main Risks

- Legal/source-line risk. Current Redis 8+ is not BSD-3. Choose the source line
  deliberately before a public port.
- Agent semantic drift. Redis has many tempting names. The vocabulary hooks must
  be in place before translation.
- Bytes vs strings. Any `String` default in Rust code is suspect for keys,
  values, and protocol payloads.
- Generated metadata drift. Command tables must be generated and tested against
  C Redis.
- Nondeterminism. INFO, TIME, SCAN order, RANDOMKEY, MEMORY, latency, CLIENT,
  cluster topology, and replication timing need normalizers.
- Scripting. Redis's embedded Lua is a project inside the project. Treat it as
  a phase boundary.
- Modules ABI. This is probably the hardest compatibility surface after cluster.
- Persistence exactness. Start with behavioral equivalence, then tighten toward
  byte exactness.
- Performance cliffs. A behaviorally-correct map/list implementation can still
  be wildly off Redis performance characteristics.

## Non-Negotiables

- No local stubs for registered cross-cutting types.
- No whole-file translation of giant Redis files.
- No command metadata hand-maintenance.
- No green verifier result without fresh evidence.
- No editing official tests to make Rust pass.
- No "compile-only" success criterion for a phase that claims behavior.
- No hiding unsupported behavior behind generic OK responses.

## Near-Term TODOs

1. Decide source line: Redis current unstable, a Redis release tag, Redis 7.2
   for BSD-3 terms, or another compatible upstream.
2. Add `harness/redis/` skeleton with generators for command registry and test
   catalog.
3. Add type/API vocabulary hooks generalized from Lua.
4. Add a RESP wire-diff oracle.
5. Create the Redis Claude skill and role prompts.
6. Generate the first 20 packets for the pilot.
7. Run a pilot with 4 workers, then scale only after packet metrics are sane.

