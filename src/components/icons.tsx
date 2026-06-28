// Inline SVG icons matching the design (lucide-style line icons).

import type { ReactNode } from "react";

type P = { size?: number; className?: string };

const svg = (size: number, sw: number, children: ReactNode, className?: string) => (
  <svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor"
    strokeWidth={sw} strokeLinecap="round" strokeLinejoin="round" className={className}>
    {children}
  </svg>
);

export const ChevronLeft = ({ size = 16 }: P) => svg(size, 2.2, <path d="m15 18-6-6 6-6" />);

export const Gear = ({ size = 15 }: P) =>
  svg(size, 2, <><path d="M20 7h-9" /><path d="M14 17H5" /><circle cx="17" cy="7" r="3" /><circle cx="7" cy="17" r="3" /></>);

export const Close = ({ size = 14 }: P) => svg(size, 2.2, <><path d="M18 6 6 18" /><path d="m6 6 12 12" /></>);

export const Copy = ({ size = 14 }: P) =>
  svg(size, 2, <><rect x="9" y="9" width="13" height="13" rx="2" /><path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1" /></>);

export const Refresh = ({ size = 14, className }: P) =>
  svg(size, 2, <>
    <path d="M3 12a9 9 0 0 1 9-9 9.75 9.75 0 0 1 6.74 2.74L21 8" />
    <path d="M21 3v5h-5" />
    <path d="M21 12a9 9 0 0 1-9 9 9.75 9.75 0 0 1-6.74-2.74L3 16" />
    <path d="M3 21v-5h5" />
  </>, className);
