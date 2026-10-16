import { ToggleGroup as ToggleGroupPrimitive } from "@base-ui/react/toggle-group";
import { cn } from "cn";
import type { Toggle as TogglePrimitive } from "@base-ui/react/toggle";
import type { ReactNode } from "react";

import { Toggle } from "@/components/ui/toggle";

/**
 * A segmented control: exactly one option is always chosen (pressing the
 * chosen one again keeps it). `label` names the group for screen readers.
 */
function ToggleGroup<T extends string>({
  value,
  onValueChange,
  label,
  className,
  children,
}: {
  value: T;
  onValueChange: (value: T) => void;
  label: string;
  className?: string | undefined;
  children: ReactNode;
}) {
  return (
    <ToggleGroupPrimitive
      data-slot="toggle-group"
      aria-label={label}
      value={[value]}
      onValueChange={(next) => {
        // Base UI reports the pressed values; an empty list means the chosen
        // option was pressed again, which keeps it.
        const v = next[0];
        if (v !== undefined) onValueChange(v);
      }}
      className={cn(
        "inline-flex w-fit items-center justify-self-start rounded-lg border border-line bg-background p-0.5",
        className,
      )}
    >
      {children}
    </ToggleGroupPrimitive>
  );
}

function ToggleGroupItem({
  icon = false,
  ...props
}: TogglePrimitive.Props & {
  /** An icon-only option: give it an aria-label (and a tooltip). */
  icon?: boolean;
}) {
  // Every other prop passes through, so a Tooltip can wrap an item.
  return <Toggle data-slot="toggle-group-item" variant={icon ? "icon" : "segment"} {...props} />;
}

export { ToggleGroup, ToggleGroupItem };
