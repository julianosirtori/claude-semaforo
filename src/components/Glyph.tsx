// The app glyph: dark rounded square with the three traffic-light dots.
// Not the Anthropic mark — an original semáforo.

export function Glyph({ box = 30, dot = 6, gap = 3 }: { box?: number; dot?: number; gap?: number }) {
  return (
    <span className="glyph" style={{ width: box, height: box, gap }}>
      <span className="glyph__d" style={{ width: dot, height: dot, background: "#E5484D" }} />
      <span className="glyph__d" style={{ width: dot, height: dot, background: "#E89B1C", opacity: 0.85 }} />
      <span className="glyph__d" style={{ width: dot, height: dot, background: "#2FA968", opacity: 0.85 }} />
    </span>
  );
}
