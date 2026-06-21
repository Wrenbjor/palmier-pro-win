// Runner for the Inspector tab-body parity checks (E12-S3..S8).
//   npx tsx src-ui/src/editor/inspector/_run-body-checks.mts
import { runInspectorBodyChecks } from "./body.checks.ts";

const failures = runInspectorBodyChecks();
if (failures.length === 0) {
  console.log("INSPECTOR BODY CHECKS OK: all checks passed");
} else {
  console.error(
    "INSPECTOR BODY CHECK FAILURES:\n" + failures.map((f) => " - " + f).join("\n"),
  );
  process.exit(1);
}
