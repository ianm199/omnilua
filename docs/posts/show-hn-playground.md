<!--
DRAFT — Show HN / r/rust launch blurb. NOT posted anywhere; for the user to edit
and post. No engagement, no "posted to" lines added.

Number sources (in-repo, fact-check before posting):
  ~1.3× geomean of reference C ......... README.md §"Benchmarks" + §"Honesty"
  official suites pass, all 5 versions . README.md §"What's checkable" → Suites
  ~0% safety-tax claim ................. docs/PERFORMANCE_MODEL.md §Safety-tax ablation
  1/3 divergence (5.1-5.4 vs 5.5) ...... crates/lua-rs-runtime/tests/multiversion_oracle.rs L1078 (5.4) / L604 (5.5);
                                         also verified live: /tmp/lua-refs/bin/lua5.{1.5,2.4,3.6,4.7} → 0.33333333333333, lua5.5.0 → 0.33333333333333331
  5 // 2 (syntax err 5.1/5.2; 2 on 5.3+)  verified live vs /tmp/lua-refs/bin/lua5.x (2026-06-13)
  math.type(1) (err <5.3; integer 5.3+)   oracle L1112 + verified live vs /tmp/lua-refs/bin/lua5.x

Link status checked 2026-06-13:
  https://ianm199.github.io/omnilua/ ................. 200 (LIVE)
  https://ianm199.github.io/omnilua/harness/bench/history/ . 200 (LIVE)
  crates.io/crates/omnilua + omnilua-cli ............ 403 (NOT published — crate links go live only after `cargo publish`)
  docs.rs/omnilua ................................... 404 (NOT published — goes live after publish)
  => Lead with the playground (live). Do NOT post crate-install lines until v0.1.0 is published.
-->

**Title:** Show HN: omniLua — one snippet, five Lua versions, live in your browser

---

omniLua is a from-scratch Lua runtime written in pure, safe Rust — no C, no FFI,
no `liblua`. Because it's pure Rust it compiles to `wasm32`, which is the thing
no other Lua-in-Rust can do: `mlua` and `rlua` are bindings to the C library, so
they can't follow your code into the browser or into a wasm game build. omniLua
can.

The launch artifact is a playground that leans on exactly that: paste one Lua
snippet and run it on **5.1, 5.2, 5.3, 5.4 and 5.5 at once**, side by side, with
five real interpreters compiled to wasm. No install, no server round-trip — it's
all running in your tab.

→ **https://ianm199.github.io/omnilua/**

**What to try.** Paste this and hit run-on-all-versions:

```lua
print(1/3)
```

5.1 through 5.4 print `0.33333333333333`; 5.5 prints `0.33333333333333331` —
its default float format changed, same snippet, visible instantly. Then try
`5 // 2` (a syntax error on 5.1/5.2, `2` on 5.3+ where floor division landed) or
`math.type(1)` (errors before 5.3, `integer` from 5.3 on) to watch the
language's history diff in front of you.

**The wedge** is game and wasm scripting — anywhere a binding has to follow your
code to the browser, where a C-backed one stops at the toolchain. The embedding
API is shaped after `mlua`, so porting an embed is mostly mechanical.

**The receipts.** The official PUC-Rio test suites pass on every supported
version (5.4 the full upstream suite; the others their own upstream trees +
reference-binary battery). Performance is ~1.3× the geomean of reference-C wall
time — competitive, not faster. And we measured what the safety costs: the
bounds-checks-and-borrow-guards ablation is ~0% of wall time, so that 1.3× is
representation idiom, not a memory-safety tax.

**Honest limits.** This is not LuaJIT and does not pretend to be — if you need
JIT speed, you want LuaJIT; if you want a decades-mature C binding, you want
`mlua`. omniLua is young: the 5.4 backend is production-ready, the other four
versions are newer surfaces. The pitch is breadth and portability — every Lua,
everywhere, in safe Rust — not raw throughput.

Feedback on the playground and the version-diff UX especially welcome.
