# Sandboxing lua-rs: what it would take

Internal design note. Assessment of how feasible it is to make lua-rs safe for
running untrusted scripts, given its current architecture.

## What "sandboxed" means

Running code you don't trust (game mods, user-submitted scripts, plugins,
multi-tenant uploads) while guaranteeing it can't harm or hang the host. Three
independent dimensions:

1. **Capabilities** — what the script can *do*. By default it can only compute
   (math, strings, tables, functions you hand it). It cannot touch the
   filesystem, network, processes, or env unless you explicitly grant it.
2. **Resource limits** — how *much* it can consume. CPU/time (an infinite loop
   must not hang the host) and memory (an allocation bomb must not OOM the host).
3. **Isolation** — it cannot reach into host memory, call host functions it
   wasn't given, or escape via a bug in the VM itself.

Sandboxing only matters for *untrusted* code. Trusted scripts you wrote yourself
don't need it.

## Assessment against lua-rs's architecture

### 1. Capability control — easy, largely already present

The dangerous stdlib already routes through host hooks (`os.execute`, file-open,
popen hooks on the state), and the `wasm32` build already runs with no OS access
at all — it only does what the JS host hands it. So the machinery for "open a
state with a restricted/host-mediated stdlib" mostly exists. Exposing it as a
clean capability profile (omit/limit `os`, `io`, `package`, `load*`) is low
effort. **Effort: low.**

### 2. Memory cap — tractable

The GC already tracks bytes used and thresholds (`heap.bytes_used()`,
`threshold_bytes()`). To cap memory, make allocation refuse past a configured
ceiling and raise a Lua error, like C Lua's allocator returning NULL. lua-rs
propagates errors as `Result`/`LuaError` rather than C's `longjmp`, which helps.
Work: wire a hard limit into the allocation paths and make the OOM error
propagate cleanly without corrupting state. **Effort: medium.**

### 3. CPU bounding — this forks

**Abort a runaway script (kill on budget): doable.** Put a counter in the VM
dispatch loop that ticks per instruction and raises an error when a budget hits
zero. lua-rs already has the debug-hook / `hookmask` / trap machinery (the recent
perf work touched exactly that), so a "stop after N instructions" limit is a
natural extension. Result: an infinite loop in untrusted code errors out instead
of hanging the host. Enough for most "run user scripts" needs. **Effort: medium.**

**Cleanly pause and resume (Piccolo-style fuel + return-to-caller): hard.**
Pausing mid-execution and resuming later requires the VM to *not* hold its state
on the native Rust call stack at the pause point — i.e. it must be stackless.
lua-rs is a faithful port of C Lua's architecture: a register VM with a CallInfo
stack that uses the Rust stack at C-call / `pcall` / coroutine boundaries.
Concrete evidence it isn't stackless: yield-through-C-frames was a known-hard,
partially-incomplete area here (the classic `longjmp`-style problem a stackless
design avoids). Clean arbitrary pause/resume is a major re-architecture, not a
feature add. This is exactly what made Piccolo a multi-year project and why its
`gc-arena` + stackless design is its identity. **Effort: research-grade rewrite.**

### Memory-safety of the implementation — mostly there

A VM bug shouldn't let a script corrupt the host. lua-rs is already mostly safe
Rust (unsafe only in the audited GC and dynamic-loader cores), and "get to full
safety" is on the roadmap. This property is in decent shape.

## Verdict

- **A useful sandbox** — deny OS access + cap memory + abort scripts that exceed
  a CPU budget — is roughly a few weeks of focused work, mostly assembling pieces
  that already exist (hook-based stdlib, GC accounting, the debug-hook/trap
  mechanism, the WASM no-OS precedent). It covers the common case: "run untrusted
  user scripts; kill them if they misbehave; deny them the OS." High value for
  the effort, and a real differentiator (a *complete*, conformant Lua 5.4 that is
  also sandboxable beats Piccolo's intentional incompleteness).

- **A Piccolo-grade sandbox** — cooperative pause/resume, preemptive coroutine
  scheduling, interleaving with async — is a big architectural lift because it
  needs a stackless VM, and lua-rs inherited C Lua's stack-based model. Expensive,
  and it's the lane Piccolo already owns.

Recommendation: scope to the achievable version (capability profiles + memory cap
+ instruction-budget abort). Don't chase clean preemption unless we commit to a
stackless redesign.
