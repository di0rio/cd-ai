import type { ComponentProps, ReactNode } from "react";

// One stroke family: 16px grid, 1.5 stroke, round caps.
const paths = {
  plus: <path d="M8 3.5v9M3.5 8h9" />,
  folder: (
    <path d="M2.5 4.5A1.5 1.5 0 0 1 4 3h2.2l1.4 1.5H12A1.5 1.5 0 0 1 13.5 6v5.5A1.5 1.5 0 0 1 12 13H4a1.5 1.5 0 0 1-1.5-1.5z" />
  ),
  chevron: <path d="M6.5 4.5 10 8l-3.5 3.5" />,
  chevronDown: <path d="M4.5 6.5 8 10l3.5-3.5" />,
  chevronUp: <path d="M4.5 9.5 8 6l3.5 3.5" />,
  search: (
    <>
      <circle cx="7" cy="7" r="4.25" />
      <path d="m10.25 10.25 3.25 3.25" />
    </>
  ),
  file: (
    <>
      <path d="M4 2.5h5l3 3v8H4z" />
      <path d="M9 2.5v3h3" />
    </>
  ),
  terminal: (
    <>
      <rect x="2" y="3" width="12" height="10" rx="1.75" />
      <path d="m5 6.5 2 1.5-2 1.5M8.5 10H11" />
    </>
  ),
  diff: <path d="M5.5 3v5M3 5.5h5M8 11.5h5" />,
  panelLeft: (
    <>
      <rect x="2" y="2.75" width="12" height="10.5" rx="1.75" />
      <path d="M6.25 2.75v10.5" />
    </>
  ),
  arrowUp: <path d="M8 12.5v-9M4.5 7 8 3.5 11.5 7" />,
  check: <path d="m3.75 8.25 2.75 2.75 5.75-6" />,
  x: <path d="m4.5 4.5 7 7m0-7-7 7" />,
  minus: <path d="M4.5 8h7" />,
  // Solid, or an outlined square at this size reads as an empty checkbox.
  stop: <rect x="4" y="4" width="8" height="8" rx="1.75" fill="currentColor" stroke="none" />,
  alert: <path d="M8 2.75 14 13H2zM8 6.75v2.5M8 11.25v.01" />,
  pencil: <path d="m10.25 3 2.75 2.75L6 12.75H3.25V10z" />,
  lock: (
    <>
      <rect x="3.5" y="7" width="9" height="6.5" rx="1.5" />
      <path d="M5.5 7V5.25a2.5 2.5 0 0 1 5 0V7" />
    </>
  ),
  gitBranch: (
    <>
      <circle cx="5.5" cy="3" r="2" />
      <circle cx="10.5" cy="8" r="2" />
      <circle cx="5.5" cy="13" r="2" />
      <path d="M5.5 5v3M7.5 8H10.5M5.5 11v2M8.5 8H10.5" />
    </>
  ),
} satisfies Record<string, ReactNode>;

export type IconName = keyof typeof paths;

// The rest of the props (ref, data-*) reach the <svg> so the shadcn primitives can use `asChild`
// on an icon without pulling in a second icon family.
export function Icon({
  name,
  className = "size-4",
  ...props
}: { name: IconName } & Omit<ComponentProps<"svg">, "viewBox" | "children">) {
  return (
    <svg
      {...props}
      viewBox="0 0 16 16"
      fill="none"
      stroke="currentColor"
      strokeWidth={1.5}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
      className={`shrink-0 ${className}`}
    >
      {paths[name]}
    </svg>
  );
}
