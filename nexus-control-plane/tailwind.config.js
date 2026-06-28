/** @type {import('tailwindcss').Config} */
export default {
  darkMode: 'class',
  content: ["./index.html", "./src/**/*.{ts,tsx}"],
  theme: {
    extend: {
      colors: {
        nx: {
          bg: "rgb(var(--nx-bg) / <alpha-value>)",
          "bg-alt": "rgb(var(--nx-bg-alt) / <alpha-value>)",
          surface: "rgb(var(--nx-surface) / <alpha-value>)",
          "surface-hover": "rgb(var(--nx-surface-hover) / <alpha-value>)",
          "surface-active": "rgb(var(--nx-surface-active) / <alpha-value>)",
          text: "rgb(var(--nx-text) / <alpha-value>)",
          "text-secondary": "rgb(var(--nx-text-secondary) / <alpha-value>)",
          muted: "rgb(var(--nx-muted) / <alpha-value>)",
          dim: "rgb(var(--nx-dim) / <alpha-value>)",
          accent: "rgb(var(--nx-accent) / <alpha-value>)",
          "accent-light": "rgb(var(--nx-accent-light) / <alpha-value>)",
          "accent-hover": "rgb(var(--nx-accent-hover) / <alpha-value>)",
          "accent-soft": "rgb(var(--nx-accent-soft) / <alpha-value>)",
          border: "rgb(var(--nx-border) / <alpha-value>)",
          "border-light": "rgb(var(--nx-border-light) / <alpha-value>)",
          "border-focus": "rgb(var(--nx-border-focus) / <alpha-value>)",
          navy: "rgb(var(--nx-navy) / <alpha-value>)",
          "navy-light": "rgb(var(--nx-navy-light) / <alpha-value>)",
          success: "rgb(var(--nx-success) / <alpha-value>)",
          warning: "rgb(var(--nx-warning) / <alpha-value>)",
          danger: "rgb(var(--nx-danger) / <alpha-value>)",
          info: "rgb(var(--nx-info) / <alpha-value>)",
          "term-bg": "rgb(var(--nx-term-bg) / <alpha-value>)",
          "term-fg": "rgb(var(--nx-term-fg) / <alpha-value>)",
          "term-cursor": "rgb(var(--nx-term-cursor) / <alpha-value>)",
          "term-selection": "rgb(var(--nx-term-selection) / <alpha-value>)",
        },
      },
      fontFamily: {
        heading: ["Montserrat", "sans-serif"],
        body: ["Poppins", "sans-serif"],
        secondary: ["Sora", "sans-serif"],
        mono: ["Menlo", "monospace"],
      },
      animation: {
        'pulse-soft': 'pulse-soft 2s ease-in-out infinite',
      },
      keyframes: {
        'pulse-soft': {
          '0%, 100%': { opacity: '1' },
          '50%': { opacity: '0.4' },
        },
      },
      boxShadow: {
        nx: "0px 2px 8px var(--nx-shadow-color)",
        "nx-md": "0px 4px 12px var(--nx-shadow-color)",
        "nx-lg": "0px 6px 16px var(--nx-shadow-color)",
        "nx-xl": "0px 8px 24px var(--nx-shadow-color)",
      },
    },
  },
  plugins: [require("@tailwindcss/typography")],
};
