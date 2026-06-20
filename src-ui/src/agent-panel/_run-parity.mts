import { runAgentParityChecks } from "./parity.checks.ts";
const failures = runAgentParityChecks();
if (failures.length === 0) {
  console.log("AGENT PARITY OK: all checks passed");
} else {
  console.error(
    "AGENT PARITY FAILURES:\n" + failures.map((f) => " - " + f).join("\n"),
  );
  process.exit(1);
}
