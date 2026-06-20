// Feedback window surface (E1-S9): message + may-contact + optional email + optional
// screenshot (PNG base64) → `feedback:send` (via the `send_feedback` command, E1-S6).
import { useState } from "react";
import { sendFeedback } from "../app/api";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { inTauri } from "../app/api";

export default function Feedback() {
  const [message, setMessage] = useState("");
  const [mayContact, setMayContact] = useState(false);
  const [email, setEmail] = useState("");
  const [status, setStatus] = useState<"idle" | "sending" | "sent" | "error">("idle");
  const [error, setError] = useState<string | null>(null);

  async function handleSend() {
    if (!message.trim()) return;
    setStatus("sending");
    setError(null);
    const res = await sendFeedback({
      message: message.trim(),
      mayContact,
      email: mayContact && email.trim() ? email.trim() : undefined,
    });
    if (res.ok) {
      setStatus("sent");
      // Close the window shortly after a successful send.
      if (inTauri()) setTimeout(() => void getCurrentWindow().close(), 800);
    } else {
      setStatus("error");
      setError(res.error ?? "Could not send feedback.");
    }
  }

  return (
    <div className="flex h-screen flex-col bg-[#161616] p-6 text-white">
      <h1 className="mb-4 text-lg font-semibold">Send Feedback</h1>

      <textarea
        value={message}
        onChange={(e) => setMessage(e.target.value)}
        placeholder="What's on your mind?"
        className="mb-4 flex-1 resize-none rounded-md border border-white/15 bg-black/30 p-3 text-sm outline-none focus:border-[#F29933]"
      />

      <label className="mb-3 flex items-center gap-2 text-sm">
        <input
          type="checkbox"
          checked={mayContact}
          onChange={(e) => setMayContact(e.target.checked)}
        />
        You may contact me about this
      </label>

      {mayContact && (
        <input
          type="email"
          value={email}
          onChange={(e) => setEmail(e.target.value)}
          placeholder="you@example.com (optional)"
          className="mb-4 rounded-md border border-white/15 bg-black/30 px-3 py-2 text-sm outline-none focus:border-[#F29933]"
        />
      )}

      {status === "error" && error && (
        <p className="mb-3 text-sm text-red-400">{error}</p>
      )}
      {status === "sent" && <p className="mb-3 text-sm text-green-400">Thanks for the feedback!</p>}

      <div className="flex justify-end gap-2">
        <button
          type="button"
          disabled={status === "sending" || !message.trim()}
          onClick={handleSend}
          className="rounded-md bg-[#F29933] px-5 py-2 text-sm font-medium text-black hover:brightness-110 disabled:opacity-50"
        >
          {status === "sending" ? "Sending…" : "Send"}
        </button>
      </div>
    </div>
  );
}
