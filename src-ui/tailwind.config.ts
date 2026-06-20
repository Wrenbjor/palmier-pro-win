import type { Config } from "tailwindcss";

// Tailwind CSS 4. Most theme config moves to CSS `@theme` (see src/index.css);
// this file mainly declares content sources. Design tokens (FOUNDATION §9) will
// be wired in from src/design/tokens.json via generated CSS variables.
export default {
  content: ["./index.html", "./src/**/*.{ts,tsx}"],
  theme: {
    extend: {},
  },
  plugins: [],
} satisfies Config;
