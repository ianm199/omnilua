/**
 * Per-version divergence smoke for the public JS API.
 *
 * Proves that selecting a Lua version through `loadLuaRsNode({ version })`
 * actually switches the wasm backend — not just the label — by running snippets
 * whose OUTPUT diverges by version and asserting each instance produces the
 * value the official PUC-Rio reference binary produces.
 *
 * Expected values were captured from /tmp/lua-refs/bin/lua5.x (2026-06-12):
 *
 *   print(3/3)        -> "1"   on 5.1/5.2 (float-only model: no ".0")
 *                        "1.0" on 5.3/5.4/5.5 (dual-subtype model)
 *   print(bit32~=nil) -> "true" on 5.2 ONLY (bit32 is a 5.2-only library)
 *   print(_VERSION)   -> "Lua 5.x" matching the selected backend
 *
 * Run: `node packages/omnilua/scripts/version-smoke.mjs`
 * Exit 0 on success, non-zero (with a diff) on any mismatch.
 */
import { loadLuaRsNode } from "../node.mjs";

const EXPECT = {
  "5.1": { div: "1", bit32: "false", version: "Lua 5.1" },
  "5.2": { div: "1", bit32: "true", version: "Lua 5.2" },
  "5.3": { div: "1.0", bit32: "true", version: "Lua 5.3" },
  "5.4": { div: "1.0", bit32: "false", version: "Lua 5.4" },
  "5.5": { div: "1.0", bit32: "false", version: "Lua 5.5" },
};

async function runOn(version, src) {
  let out = "";
  const { lua } = await loadLuaRsNode({ version, onStdout: (chunk) => { out += chunk; } });
  const result = lua.tryExec(src);
  if (!result.ok) {
    throw new Error(`run on ${version} failed: ${result.error}`);
  }
  if (lua.currentVersion() !== version) {
    throw new Error(
      `instance reports version ${lua.currentVersion()} but ${version} was requested`,
    );
  }
  return out.trim();
}

const failures = [];

function check(version, label, actual, expected) {
  const status = actual === expected ? "ok" : "MISMATCH";
  console.log(
    `  ${version} ${label.padEnd(8)} -> ${JSON.stringify(actual)}` +
      (status === "ok" ? "" : `  expected ${JSON.stringify(expected)}  [${status}]`),
  );
  if (status !== "ok") {
    failures.push(`${version} ${label}: got ${JSON.stringify(actual)}, want ${JSON.stringify(expected)}`);
  }
}

console.log("omniLua per-version divergence smoke (public JS API):");
for (const version of Object.keys(EXPECT)) {
  const want = EXPECT[version];
  check(version, "3/3", await runOn(version, "print(3/3)"), want.div);
  check(version, "bit32", await runOn(version, "print(bit32 ~= nil)"), want.bit32);
  check(version, "_VERSION", await runOn(version, "print(_VERSION)"), want.version);
}

const div51 = await runOn("5.1", "print(3/3)");
const div54 = await runOn("5.4", "print(3/3)");
if (div51 === div54) {
  failures.push(
    `5.1 and 5.4 produced identical output ${JSON.stringify(div51)} for print(3/3) — ` +
      `version selection is not switching the backend`,
  );
} else {
  console.log(
    `\n  divergence proven: 5.1 print(3/3) = ${JSON.stringify(div51)} != ` +
      `5.4 print(3/3) = ${JSON.stringify(div54)}`,
  );
}

if (failures.length > 0) {
  console.error("\nFAIL:");
  for (const f of failures) console.error(`  - ${f}`);
  process.exit(1);
}
console.log("\nok: all five backends run live with reference-verified divergent output");
