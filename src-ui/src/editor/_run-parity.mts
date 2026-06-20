import { runParityChecks } from "./parity.checks.ts";
const failures = runParityChecks();
if (failures.length === 0) {
  console.log("PARITY OK: all checks passed");
} else {
  console.error("PARITY FAILURES:\n" + failures.map((f) => " - " + f).join("\n"));
  process.exit(1);
}
