import type { SVGProps } from 'react';

/**
 * NSP brand mark. Inline so it inherits `currentColor` for stroke /
 * letter and so the gradient picks up the project's primary blue from
 * the Tailwind theme — no PNG/asset hop. Sizing comes from
 * `width`/`height` like a normal SVG (we default to 1em so it tracks
 * surrounding font size, e.g. inline next to the wordmark).
 */
export function Logo({
  className,
  width = 24,
  height = 24,
  ...props
}: SVGProps<SVGSVGElement>) {
  return (
    <svg
      xmlns="http://www.w3.org/2000/svg"
      viewBox="0 0 24 24"
      fill="none"
      width={width}
      height={height}
      className={className}
      role="img"
      aria-label="NSP"
      {...props}
    >
      <defs>
        <linearGradient id="nsp-logo-shield" x1="0" y1="0" x2="24" y2="24" gradientUnits="userSpaceOnUse">
          <stop offset="0" stopColor="hsl(217 91% 60%)" />
          <stop offset="1" stopColor="hsl(217 91% 45%)" />
        </linearGradient>
      </defs>
      <path
        d="M12 2 L20 5 V11.5 C20 16 16.5 20.5 12 22 C7.5 20.5 4 16 4 11.5 V5 Z"
        fill="url(#nsp-logo-shield)"
      />
      <path
        d="M12 2 L20 5 V11.5 C20 16 16.5 20.5 12 22 C7.5 20.5 4 16 4 11.5 V5 Z"
        fill="none"
        stroke="hsl(217 91% 45%)"
        strokeWidth="0.6"
        strokeLinejoin="round"
      />
      <text
        x="12"
        y="15.5"
        textAnchor="middle"
        fontFamily="ui-sans-serif, system-ui, -apple-system, Segoe UI, Roboto, sans-serif"
        fontWeight={700}
        fontSize={9.5}
        letterSpacing={0.2}
        fill="#ffffff"
      >
        N
      </text>
    </svg>
  );
}
