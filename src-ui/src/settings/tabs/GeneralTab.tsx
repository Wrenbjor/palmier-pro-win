// General tab (E1-S9): Notifications + Privacy panes.
//
// - Notifications toggle ⇒ `io.palmier.pro.notifications.enabled` (absent ⇒ ON).
// - Privacy toggle "Send anonymous crash and error reports" ⇒
//   `io.palmier.pro.telemetry.enabled`; shows "Restart Palmier Pro to apply" when the
//   live value differs from the launch snapshot (telemetry is launch-snapshotted,
//   restart-required — FR-42).
import {
  setNotificationsEnabled,
  setTelemetryEnabled,
  type SettingsSnapshot,
} from "../../app/api";
import ToggleRow from "../ToggleRow";

interface GeneralTabProps {
  settings: SettingsSnapshot;
  onChange: (next: SettingsSnapshot) => void;
}

export default function GeneralTab({ settings, onChange }: GeneralTabProps) {
  const telemetryNeedsRestart =
    settings.telemetryEnabled !== settings.telemetryEnabledForLaunch;

  return (
    <div>
      <h1 className="mb-6 text-lg font-semibold">General</h1>

      <section className="mb-8">
        <h2 className="mb-2 text-xs font-medium uppercase tracking-wide text-white/40">
          Notifications
        </h2>
        <ToggleRow
          label="Show notifications"
          description="Notify when generations and exports finish."
          checked={settings.notificationsEnabled}
          onChange={(next) => {
            onChange({ ...settings, notificationsEnabled: next });
            void setNotificationsEnabled(next);
          }}
        />
      </section>

      <section>
        <h2 className="mb-2 text-xs font-medium uppercase tracking-wide text-white/40">
          Privacy
        </h2>
        <ToggleRow
          label="Send anonymous crash and error reports"
          description="Helps us find and fix problems. No personal data is sent."
          checked={settings.telemetryEnabled}
          onChange={(next) => {
            onChange({ ...settings, telemetryEnabled: next });
            void setTelemetryEnabled(next);
          }}
          note={telemetryNeedsRestart ? "Restart Palmier Pro to apply" : undefined}
        />
      </section>
    </div>
  );
}
