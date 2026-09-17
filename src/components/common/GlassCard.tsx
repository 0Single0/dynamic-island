import { forwardRef, type HTMLAttributes } from "react";
import { cn, glassVariants } from "@/lib/utils";

/**
 * 玻璃拟态面板：应用内统一的表面容器。
 * 收起态胶囊使用 .glass-island（更深），展开面板用本组件。
 */
export interface GlassCardProps extends HTMLAttributes<HTMLDivElement> {
  tone?: "dark" | "subtle";
  padding?: "none" | "sm" | "md" | "lg";
}

export const GlassCard = forwardRef<HTMLDivElement, GlassCardProps>(
  ({ className, tone, padding, ...props }, ref) => (
    <div
      ref={ref}
      className={cn(glassVariants({ tone, padding }), className)}
      {...props}
    />
  ),
);
GlassCard.displayName = "GlassCard";
