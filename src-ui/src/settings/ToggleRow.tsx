// Shared settings toggle row (reference `SettingsToggleRow`).
interface ToggleRowProps {
  label: string;
  description?: string;
  checked: boolean;
  onChange: (next: boolean) => void;
  /** Optional note shown under the row (e.g. "Restart Palmier Pro to apply"). */
  note?: string;
}

export default function ToggleRow({
  label,
  description,
  checked,
  onChange,
  note,
}: ToggleRowProps) {
  return (
    <div className="border-b border-white/10 py-4">
      <div className="flex items-center justify-between">
        <div className="pr-6">
          <div className="text-sm font-medium">{label}</div>
          {description && <div className="text-xs text-white/50">{description}</div>}
        </div>
        <button
          type="button"
          role="switch"
          aria-checked={checked}
          onClick={() => onChange(!checked)}
          className={`relative h-6 w-11 shrink-0 rounded-full transition-colors ${
            checked ? "bg-[#F29933]" : "bg-white/20"
          }`}
        >
          <span
            className={`absolute top-0.5 h-5 w-5 rounded-full bg-white transition-transform ${
              checked ? "translate-x-5" : "translate-x-0.5"
            }`}
          />
        </button>
      </div>
      {note && <div className="mt-2 text-xs text-[#F29933]">{note}</div>}
    </div>
  );
}
