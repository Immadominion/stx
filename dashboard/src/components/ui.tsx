// Shared primitives: status badge, mono text, external link.

const STATUS: Record<
  string,
  { label: string; dot: string; text: string; bg: string }
> = {
  confirmed: {
    label: "Confirmed",
    dot: "bg-st-confirmed",
    text: "text-st-confirmed-ink",
    bg: "bg-st-confirmed/10",
  },
  finalized: {
    label: "Finalized",
    dot: "bg-st-finalized",
    text: "text-st-finalized-ink",
    bg: "bg-st-finalized/10",
  },
  landed: {
    label: "Landed",
    dot: "bg-st-landed",
    text: "text-st-landed-ink",
    bg: "bg-st-landed/10",
  },
  pending: {
    label: "Pending",
    dot: "bg-st-pending",
    text: "text-st-pending-ink",
    bg: "bg-st-pending/10",
  },
  failed: {
    label: "Failed",
    dot: "bg-st-failed",
    text: "text-st-failed-ink",
    bg: "bg-st-failed/10",
  },
  aborted: {
    label: "Aborted",
    dot: "bg-st-failed",
    text: "text-st-failed-ink",
    bg: "bg-st-failed/10",
  },
  submitted: {
    label: "Submitted",
    dot: "bg-st-submitted",
    text: "text-ink-2",
    bg: "bg-sunk",
  },
};

export function StatusBadge({
  status,
  label,
}: {
  status: string;
  label?: string;
}) {
  const s = STATUS[status] ?? STATUS.submitted;
  return (
    <span
      className={`inline-flex items-center gap-1.5 rounded-full px-2.5 py-1 text-xs font-medium ${s.bg} ${s.text}`}
    >
      <span
        className={`size-1.5 rounded-full ${s.dot} ${status === "pending" ? "animate-pulse" : ""}`}
        aria-hidden
      />
      {label ?? s.label}
    </span>
  );
}

export function Mono({
  children,
  className = "",
}: {
  children: React.ReactNode;
  className?: string;
}) {
  return <span className={`font-mono ${className}`}>{children}</span>;
}

export function ExtLink({
  href,
  children,
  className = "",
}: {
  href: string;
  children: React.ReactNode;
  className?: string;
}) {
  return (
    <a
      href={href}
      target="_blank"
      rel="noopener noreferrer"
      className={`inline-flex items-center gap-1 text-brand-ink underline-offset-2 hover:underline ${className}`}
    >
      {children}
      <svg
        width="11"
        height="11"
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        strokeWidth="2.5"
        strokeLinecap="round"
        strokeLinejoin="round"
        aria-hidden
        className="opacity-60"
      >
        <path d="M7 17 17 7M9 7h8v8" />
      </svg>
    </a>
  );
}
