import { runEditChecks } from "./edit.checks.ts";
const failures = runEditChecks();
if (failures.length === 0) {
  console.log("EDIT CHECKS OK: all checks passed");
} else {
  console.error("EDIT CHECK FAILURES:\n" + failures.map((f) => " - " + f).join("\n"));
  process.exit(1);
}
