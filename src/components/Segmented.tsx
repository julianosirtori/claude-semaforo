export function Segmented<T extends string>({
  value, options, onChange, variant,
}: {
  value: T;
  options: { value: T; label: string }[];
  onChange: (v: T) => void;
  variant?: "tema";
}) {
  return (
    <div className={`seg${variant === "tema" ? " seg--tema" : ""}`}>
      {options.map((o) => (
        <span
          key={o.value}
          className={`seg__opt${o.value === value ? " seg__opt--on" : ""}`}
          onClick={(e) => { e.stopPropagation(); onChange(o.value); }}
        >
          {o.label}
        </span>
      ))}
    </div>
  );
}
