import { type VariantProps, cva } from "class-variance-authority";
import { clsx, type ClassValue } from "clsx";
import { twMerge } from "tailwind-merge";

export function cn(...inputs: ClassValue[]) {
  return twMerge(clsx(inputs));
}

export const glassVariants = cva("glass-panel text-zinc-100", {
  variants: {
    tone: {
      dark: "bg-black/60",
      subtle: "bg-white/5",
    },
    padding: {
      none: "",
      sm: "p-3",
      md: "p-4",
      lg: "p-6",
    },
  },
  defaultVariants: {
    tone: "dark",
    padding: "md",
  },
});

export type GlassProps = VariantProps<typeof glassVariants>;
