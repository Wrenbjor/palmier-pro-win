// Headless runner for the preview viewport pure-logic checks (E5-S10).
//
//   npx tsx src-ui/src/preview/_run-checks.mts
//
// Mirrors the editor's `_run-edit-checks.mts`: no DOM, no Tauri — just the pure
// geometry / preset / store assertions in `preview.checks.ts`.

import { PREVIEW_CHECK_COUNT, runPreviewChecks } from "./preview.checks.ts";

const failures = runPreviewChecks();
if (failures.length === 0) {
  console.log(`PREVIEW CHECKS OK: all ${PREVIEW_CHECK_COUNT} checks passed`);
} else {
  console.error("PREVIEW CHECK FAILURES:\n" + failures.map((f) => " - " + f).join("\n"));
  process.exit(1);
}
