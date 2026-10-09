export interface PopupBounds { left: number; top: number; right: number; bottom: number }

export function selectPosition(trigger: PopupBounds, bounds: PopupBounds, desiredHeight: number, desiredWidth: number) {
  const gap = 6;
  const below = Math.max(0, bounds.bottom - trigger.bottom - gap);
  const above = Math.max(0, trigger.top - bounds.top - gap);
  const upwards = below < desiredHeight && above > below;
  const height = Math.min(desiredHeight, upwards ? above : below);
  const width = Math.min(desiredWidth, Math.max(0, bounds.right - bounds.left));
  const left = Math.max(bounds.left, Math.min(trigger.left, bounds.right - width));
  return { left, top: upwards ? trigger.top - gap - height : trigger.bottom + gap, width, maxHeight: height, upwards };
}
