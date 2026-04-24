/** @type {import('tailwindcss').Config} */
module.exports = {
  content: ["./src/**/*.{html,js}"],
  theme: {
    extend: {
      fontFamily: {
        sans: ['"DM Sans"', 'system-ui', 'sans-serif'],
        mono: ['"JetBrains Mono"', 'Consolas', 'monospace'],
      },
      colors: {
        ox: {
          bg: '#0c0e13',
          surface: '#12151c',
          panel: '#181c25',
          border: '#252a36',
          hover: '#1e2330',
          active: '#262d3d',
          text: '#d4d8e0',
          muted: '#6b7280',
          dim: '#454d5e',
          amber: '#e5a200',
          amberDim: '#a07000',
          amberGlow: 'rgba(229, 162, 0, 0.08)',
          blue: '#4c9aff',
          green: '#3ddc84',
          red: '#ff5c5c',
          v2: '#3ddc84',
          v4: '#4c9aff',
        }
      }
    }
  },
  plugins: [],
}
