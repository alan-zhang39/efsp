/** Small line icons, 20×20, drawn in currentColor. Decorative: labels carry meaning. */
const base = {
  width: 20,
  height: 20,
  viewBox: "0 0 20 20",
  fill: "none",
  stroke: "currentColor",
  strokeWidth: 1.6,
  strokeLinecap: "round" as const,
  strokeLinejoin: "round" as const,
  "aria-hidden": true,
};

/** Rising bars: the live match card. */
export function LiveIcon() {
  return (
    <svg {...base}>
      <path d="M4 15v-2M8 15v-5M12 15V7M16 15V4" />
    </svg>
  );
}

/** A list with a clock tick: match history. */
export function HistoryIcon() {
  return (
    <svg {...base}>
      <path d="M3 5h9M3 10h6M3 15h9" />
      <circle cx="15" cy="12" r="3.2" />
      <path d="M15 10.6v1.5l1 .8" />
    </svg>
  );
}

/** A globe: server regions. */
export function ServersIcon() {
  return (
    <svg {...base}>
      <circle cx="10" cy="10" r="7" />
      <ellipse cx="10" cy="10" rx="3" ry="7" />
      <path d="M3 10h14" />
    </svg>
  );
}

export function SettingsIcon() {
  return (
    <svg {...base}>
      <circle cx="10" cy="10" r="2.6" />
      <path d="M10 2.5v2.2M10 15.3v2.2M2.5 10h2.2M15.3 10h2.2M4.7 4.7l1.6 1.6M13.7 13.7l1.6 1.6M4.7 15.3l1.6-1.6M13.7 6.3l1.6-1.6" />
    </svg>
  );
}

export function CollapseIcon({ collapsed }: { collapsed: boolean }) {
  return (
    <svg {...base}>
      <rect x="2.5" y="3.5" width="15" height="13" rx="2" />
      <path d="M7.5 3.5v13" />
      <path d={collapsed ? "M11 8.5l1.8 1.5L11 11.5" : "M13 8.5L11.2 10l1.8 1.5"} />
    </svg>
  );
}

/** History as a list of rows. */
export function RowsIcon() {
  return (
    <svg {...base}>
      <path d="M3 5h14M3 10h14M3 15h14" />
    </svg>
  );
}

/** History as a grid of cards. */
export function CardsIcon() {
  return (
    <svg {...base}>
      <rect x="3" y="3" width="6" height="6" rx="1.2" />
      <rect x="11" y="3" width="6" height="6" rx="1.2" />
      <rect x="3" y="11" width="6" height="6" rx="1.2" />
      <rect x="11" y="11" width="6" height="6" rx="1.2" />
    </svg>
  );
}

/** Two arrows chasing round: refresh. */
export function RefreshIcon() {
  return (
    <svg {...base} width={16} height={16}>
      <path d="M15.5 8A6 6 0 0 0 4.6 6M4.5 12a6 6 0 0 0 10.9 2" />
      <path d="M4.5 3v3.2h3.2M15.5 17v-3.2h-3.2" />
    </svg>
  );
}

/** A small down chevron: a dropdown's affordance. */
export function ChevronDownIcon({ className }: { className?: string | undefined }) {
  return (
    <svg {...base} className={className}>
      <path d="M5.5 8l4.5 4.5L14.5 8" />
    </svg>
  );
}

/** A tick: the chosen option in a dropdown. */
export function CheckIcon({ className }: { className?: string | undefined }) {
  return (
    <svg {...base} className={className}>
      <path d="M4.5 10.5l3.5 3.5 7.5-8" />
    </svg>
  );
}
