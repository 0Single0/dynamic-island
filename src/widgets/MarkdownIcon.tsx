/** 简洁的 Markdown 标识：在文档页中组合 M 与向下箭头。 */
export function MarkdownIcon({ className = "", strokeWidth = 1.8 }: { className?: string; strokeWidth?: number }) {
  return (
    <svg
      viewBox="0 0 24 24"
      fill="none"
      aria-hidden="true"
      className={className}
      stroke="currentColor"
      strokeWidth={strokeWidth}
      strokeLinecap="round"
      strokeLinejoin="round"
    >
      <path d="M5 3.75h9.3L19 8.4v11.85H5z" fill="currentColor" fillOpacity="0.12" />
      <path d="M14.3 3.75V8.4H19" />
      <path d="M6.9 16V10l2.35 2.55L11.6 10v6" />
      <path d="M15.15 10.1v5.9m0 0 2-2m-2 2-2-2" />
    </svg>
  );
}
