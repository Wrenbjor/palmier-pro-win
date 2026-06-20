import { runInspectorParityChecks } from "./parity.checks.ts";
const failures = runInspectorParityChecks();
if (failures.length === 0) {
  console.log("INSPECTOR PARITY OK: all checks passed");
} else {
  console.error(
    "INSPECTOR PARITY FAILURES:\n" + failures.map((f) => " - " + f).join("\n"),
  );
  process.exit(1);
}
