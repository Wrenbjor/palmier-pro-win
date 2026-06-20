import { runMediaParityChecks } from "./parity.checks.ts";
const failures = runMediaParityChecks();
if (failures.length === 0) {
  console.log("MEDIA PARITY OK: all checks passed");
} else {
  console.error(
    "MEDIA PARITY FAILURES:\n" + failures.map((f) => " - " + f).join("\n"),
  );
  process.exit(1);
}
