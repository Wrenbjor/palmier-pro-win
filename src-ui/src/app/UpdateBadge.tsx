// Update badge (E1-S10): the frontend equivalent of `App/UpdateBadgeView.swift`.
//
// Subscribes to the `update://status` Tauri event (emitted by
// `crates/palmier-tauri/src/update.rs`). Shows a small "Update available" pill only when
// `available` is true; stays hidden otherwise — including the disabled / dev-build case
// (no signed feed), exactly like the reference badge which only renders when
// `Updater.shared.updateAvailable`.
import { useEffect, useState } from "react";
import { checkForUpdates, onUpdateStatus, type UpdateStatus } from "./api";

export default function UpdateBadge() {
  const [status, setStatus] = useState<UpdateStatus | null>(null);

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    onUpdateStatus(setStatus).then((un) => {
      unlisten = un;
    });
    return () => unlisten?.();
  }, []);

  if (!status?.available) return null;

  return (
    <button
      type="button"
      onClick={() => void checkForUpdates()}
      title="An update is available"
      className="rounded-full bg-[#F29933] px-3 py-1 text-xs font-medium text-black hover:brightness-110"
    >
      Update available{status.version ? ` · ${status.version}` : ""}
    </button>
  );
}
