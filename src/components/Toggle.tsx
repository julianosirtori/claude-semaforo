export function Toggle({ on, onChange, title }: { on: boolean; onChange: (v: boolean) => void; title?: string }) {
  return (
    <button
      type="button"
      title={title}
      aria-pressed={on}
      className={`tg${on ? " tg--on" : ""}`}
      onClick={(e) => { e.stopPropagation(); onChange(!on); }}
    >
      <span className="tg__k" />
    </button>
  );
}
