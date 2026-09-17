import { cn } from "@/lib/utils";

export function IconButton({
  icon: Icon,
  label,
  onClick,
  className,
  active,
}: {
  icon: React.ComponentType<{ className?: string; strokeWidth?: number }>;
  label: string;
  onClick?: (event: React.MouseEvent) => void;
  className?: string;
  active?: boolean;
}) {
  return (
    <button
      type="button"
      title={label}
      aria-label={label}
      onClick={(event) => {
        event.stopPropagation();
        onClick?.(event);
      }}
      className={cn(
        "flex h-8 w-8 items-center justify-center rounded-full text-zinc-400 transition-colors",
        "hover:bg-white/10 hover:text-zinc-100 active:bg-white/15",
        active && "bg-white/10 text-zinc-100",
        className,
      )}
    >
      <Icon className="h-4 w-4" strokeWidth={2} />
    </button>
  );
}
